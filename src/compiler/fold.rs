//! Constant folding, a release-profile pass: an operation whose operands are
//! known when the program is compiled is worked out then, and the program
//! loads the result instead.
//!
//! The parser already folds an operator between two literals. What it cannot
//! see is a value that reaches an operation through a register: a variable
//! that holds a literal, a field of a struct kept in registers, or an
//! argument of a body copied into its call site. A register's value is known
//! at an instruction when every way control reaches that instruction left the
//! same constant in it, which a pass over the flow of control works out. An
//! operation on known values becomes a load of its result, and that result is
//! known in turn, so a chain of them folds whole. A branch on a known
//! condition becomes a jump, or goes away when it is never taken. A read of a
//! register whose value is known reads a constant register instead, and a
//! write nothing reads any more, or that the same stretch of code overwrites
//! before reading, is dropped. A result made in one stretch of code is only
//! known in the next once it sits in a constant register, so the pass runs
//! again while it still finds something to fold.
//!
//! Nothing the program can observe changes. An operation that raises an error
//! at run time (an `int` division or remainder by zero, a negative exponent, a
//! shift by a count out of range, a string operation on something that is not
//! a string) stays in the program to raise it where it did, and so does an
//! `int` operation whose result does not fit, which is left to the VM. `^` on
//! floats stays too: its result comes from the math library of the machine
//! that runs the program. Every other float operation is IEEE 754 and gives
//! the same bits here as there. A rewritten instruction that can raise an
//! error keeps its span.

use crate::compiler::flow::Program;
use crate::compiler::flow::branch_target;
use crate::compiler::flow::branch_targets;
use crate::compiler::flow::carry_span;
use crate::compiler::flow::ends_straight_run;
use crate::compiler::flow::forward_size;
use crate::compiler::flow::is_call;
use crate::compiler::flow::keeps_place;
use crate::compiler::flow::relocate;
use crate::compiler::flow::successors;
use crate::data::Data;
use crate::instr::Instr;
use crate::vm::StringPool;
use crate::vm::shift_count_in_range;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// The constant each register is known to hold at one point in the code.
type Known = FxHashMap<u16, Data>;

/// What an instruction amounts to once its operands are known.
enum Folded {
    /// It writes this value into this register.
    Value(u16, Data),
    /// It is a branch, taken (`true`) or not every time it runs.
    Branch(bool),
}

/// How many times the pass runs at most. Each run carries what the one before
/// made constant one stretch of code further, and a program rarely has more
/// than a couple of such steps.
const ROUNDS: usize = 4;

/// Folds every operation the module documentation describes.
pub fn fold_constants(program: &mut Program<'_>) {
    for _ in 0..ROUNDS {
        if !fold_once(program) {
            break;
        }
    }
}

/// One run of the pass, answering whether it changed anything.
fn fold_once(program: &mut Program<'_>) -> bool {
    let old: Vec<Instr> = program.instructions.clone();
    let len = old.len();
    if len == 0 {
        return false;
    }
    let lands = branch_targets(&old);
    let entries = program.entries();
    let outside = program.outside_registers();

    let mut written: FxHashSet<u16> = FxHashSet::default();
    for instr in &old {
        instr.for_each_write_reg(|reg| {
            written.insert(reg);
        });
    }
    // A literal's register nothing writes holds its value everywhere. The
    // null sink is only ever written by an instruction naming it.
    let constants: Known = program
        .const_registers
        .values()
        .filter(|&&reg| !written.contains(&reg) && (reg == 0 || !outside.contains(&reg)))
        .filter_map(|&reg| {
            let value = program.registers[reg as usize];
            tracked(value).then_some((reg, value))
        })
        .collect();

    // The code in straight stretches: each starts where control can arrive
    // other than from the instruction before it, or after one that leaves.
    let mut starts: Vec<usize> = Vec::new();
    let mut block_of: Vec<usize> = Vec::with_capacity(len);
    for pos in 0..len {
        if lands[pos] || entries[pos] || (pos > 0 && ends_straight_run(old[pos - 1])) {
            starts.push(pos);
        }
        block_of.push(starts.len() - 1);
    }
    let end_of = |block: usize| starts.get(block + 1).copied().unwrap_or(len);

    // What is known where each stretch starts: nothing where control enters
    // from outside (a call, a catch, the start of the program), and whatever
    // every stretch that flows into it agrees on elsewhere.
    let mut at_start: Vec<Option<Known>> = vec![None; starts.len()];
    let mut queued = vec![false; starts.len()];
    let mut work: Vec<usize> = Vec::new();
    for (block, &start) in starts.iter().enumerate() {
        if entries[start] {
            at_start[block] = Some(Known::default());
            queued[block] = true;
            work.push(block);
        }
    }
    while let Some(block) = work.pop() {
        queued[block] = false;
        let mut known = at_start[block].clone().unwrap_or_default();
        let end = end_of(block);
        for &instr in &old[starts[block]..end] {
            let folded = fold(instr, &known, &constants, program.strings, false);
            step(&mut known, instr, folded.as_ref(), &constants);
        }
        let last = end - 1;
        for next in successors(last, old[last]).into_iter().flatten() {
            if next >= len || entries[next] {
                continue;
            }
            let next_block = block_of[next];
            let changed = match &mut at_start[next_block] {
                Some(agreed) => {
                    let before = agreed.len();
                    agreed.retain(|reg, value| known.get(reg) == Some(value));
                    agreed.len() != before
                }
                slot @ None => {
                    *slot = Some(known.clone());
                    true
                }
            };
            if changed && !queued[next_block] {
                queued[next_block] = true;
                work.push(next_block);
            }
        }
    }

    let mut new = old.clone();
    let mut dropped = vec![false; len];
    let mut known = Known::default();
    for pos in 0..len {
        let block = block_of[pos];
        if starts[block] == pos {
            known = at_start[block].clone().unwrap_or_default();
        }
        let instr = old[pos];
        let folded = fold(instr, &known, &constants, program.strings, true);
        match folded {
            Some(Folded::Value(dest, value)) if dest != 0 => {
                if let Some(load) = load(program, &written, dest, value) {
                    new[pos] = load;
                }
            }
            Some(Folded::Branch(true)) => {
                new[pos] = match instr {
                    Instr::InfIntJmpBack(_, _, size) => Instr::JmpBack(size),
                    other => Instr::Jmp(forward_size(other).unwrap_or(1)),
                };
            }
            // The test in front of a counted loop runs once per time the loop
            // is reached, and machine code folds it by itself; taking it out
            // only reshapes the code around the loop, which can cost the
            // machine code a register move per turn.
            Some(Folded::Branch(false)) if !guards_loop(&old, pos) => dropped[pos] = true,
            _ => new[pos] = read_constants(program, &written, instr, &known),
        }
        step(&mut known, instr, folded.as_ref(), &constants);
    }

    drop_overwritten(&lands, &new, &mut dropped);
    drop_unread_writes(program, &outside, &new, &mut dropped);
    if new == old && !dropped.contains(&true) {
        return false;
    }

    let mut map: Vec<usize> = Vec::with_capacity(len + 1);
    let mut kept: Vec<Instr> = Vec::with_capacity(len);
    for (pos, &instr) in new.iter().enumerate() {
        map.push(kept.len());
        if !dropped[pos] {
            kept.push(instr);
        }
    }
    map.push(kept.len());
    relocate(program, &old, &dropped, &mut kept, &map);
    *program.instructions = kept;
    true
}

/// Whether the branch at `pos` is the test in front of a loop: it jumps to
/// just past the loop, whose last instruction jumps back to just after it.
fn guards_loop(instructions: &[Instr], pos: usize) -> bool {
    let Some(size) = forward_size(instructions[pos]) else {
        return false;
    };
    let end = pos + size as usize;
    end > pos + 1
        && instructions.get(end - 1).is_some_and(|&last| {
            matches!(
                last,
                Instr::JmpBack(_) | Instr::InfIntJmpBack(..) | Instr::StepIntJmpBack(..)
            ) && branch_target(end - 1, last) == Some(pos + 1)
        })
}

/// Whether the pass keeps track of `value`: a number, a bool, null or a
/// string, the values an operation it folds reads.
const fn tracked(value: Data) -> bool {
    value.is_int() || value.is_float() || value.is_bool() || value.is_null() || value.is_string()
}

/// The instruction that writes `value` into `dest`: one that carries the
/// value where it fits, else a move out of a constant register.
fn load(
    program: &mut Program<'_>,
    written: &FxHashSet<u16>,
    dest: u16,
    value: Data,
) -> Option<Instr> {
    if value.is_int()
        && let Ok(n) = i32::try_from(value.as_int())
    {
        return Some(Instr::SetInt(dest, n));
    }
    if value.is_bool() {
        return Some(Instr::SetBool(value.as_bool(), dest));
    }
    program
        .constant_register(value, written)
        .map(|reg| Instr::Mov(reg, dest))
}

/// `instr` reading a constant register in place of each register whose value
/// is known there, or `instr` itself when it cannot.
///
/// A stored call argument is read when the call runs, not where it is stored,
/// and an operand the instruction also writes is not a plain read, so neither
/// is touched.
fn read_constants(
    program: &mut Program<'_>,
    written: &FxHashSet<u16>,
    instr: Instr,
    known: &Known,
) -> Instr {
    if matches!(instr, Instr::StoreFuncArg(_)) {
        return instr;
    }
    let mut writes: [Option<u16>; 2] = [None; 2];
    let mut count = 0;
    instr.for_each_write_reg(|reg| {
        writes[count] = Some(reg);
        count += 1;
    });
    let mut new = instr;
    new.read_regs_mut(|reg| {
        if writes.contains(&Some(*reg)) {
            return;
        }
        if let Some(&value) = known.get(reg)
            && let Some(constant) = program.constant_register(value, written)
        {
            *reg = constant;
        }
    });
    // Only an instruction that can raise an error is ever looked up in the
    // span table.
    if new == instr || plain(instr) || matches!(instr, Instr::Print(_)) {
        return new;
    }
    if !keeps_place(program.instr_src, instr, new) {
        return instr;
    }
    carry_span(program.instr_src, instr, new);
    new
}

/// Moves what `instr` does to the registers into `known`: a copy of a known
/// value is known, a folded result is known, and anything else written is no
/// longer known. A call can write any register, so after one nothing is.
fn step(known: &mut Known, instr: Instr, folded: Option<&Folded>, constants: &Known) {
    let value = |known: &Known, reg: u16| known.get(&reg).or_else(|| constants.get(&reg)).copied();
    let set = |known: &mut Known, reg: u16, value: Option<Data>| match value {
        Some(value) if tracked(value) && reg != 0 => {
            known.insert(reg, value);
        }
        _ => {
            known.remove(&reg);
        }
    };
    match instr {
        Instr::Mov(from, to) => {
            let moved = value(known, from);
            set(known, to, moved);
        }
        Instr::SetInt(to, n) => set(known, to, Some(Data::int(i64::from(n)))),
        Instr::SetBool(b, to) => set(known, to, Some(Data::bool(b))),
        // `dest = mid`, then `mid = src`.
        Instr::MovChain(dest, mid, src) => {
            let (first, second) = (value(known, mid), value(known, src));
            set(known, dest, first);
            set(known, mid, second);
        }
        other
            if is_call(other)
                || matches!(
                    other,
                    Instr::CallHostFunc(_, _) | Instr::CallDynamicLibFunc(_, _)
                ) =>
        {
            known.clear();
        }
        other => {
            other.for_each_write_reg(|reg| {
                known.remove(&reg);
            });
            if let Some(&Folded::Value(dest, result)) = folded {
                set(known, dest, Some(result));
            }
        }
    }
}

/// What `instr` amounts to when the operands it reads are known and it cannot
/// raise an error. A concatenation makes a new string, so it is only worked
/// out when `intern` says the string may be added to the pool.
fn fold(
    instr: Instr,
    known: &Known,
    constants: &Known,
    strings: &mut StringPool,
    intern: bool,
) -> Option<Folded> {
    let value = |reg: u16| known.get(&reg).or_else(|| constants.get(&reg)).copied();
    let int = |reg: u16| value(reg).filter(|v| v.is_int()).map(Data::as_int);
    let float = |reg: u16| value(reg).filter(|v| v.is_float()).map(Data::as_float);
    let boolean = |reg: u16| value(reg).filter(|v| v.is_bool()).map(Data::as_bool);
    // A value `Eq` compares as its two words: never a string, whose words
    // name a pool slot rather than its text.
    let plain = |reg: u16| value(reg).filter(|v| !v.is_string());
    let text = |reg: u16| value(reg).filter(|v| v.is_string());
    let int_op = |a: u16, b: u16, op: fn(i64, i64) -> Option<i64>| op(int(a)?, int(b)?);
    let float_op = |a: u16, b: u16, op: fn(f64, f64) -> f64| Some(op(float(a)?, float(b)?));
    let int_cmp = |a: u16, b: u16, op: fn(&i64, &i64) -> bool| Some(op(&int(a)?, &int(b)?));
    let float_cmp = |a: u16, b: u16, op: fn(&f64, &f64) -> bool| Some(op(&float(a)?, &float(b)?));
    let str_cmp = |a: u16, b: u16, op: fn(&str, &str) -> bool| {
        let (a, b) = (text(a)?, text(b)?);
        Some(op(a.as_str(strings), b.as_str(strings)))
    };
    let int_value = |dest: u16, n: Option<i64>| n.map(|n| Folded::Value(dest, Data::int(n)));
    let float_value = |dest: u16, x: Option<f64>| x.map(|x| Folded::Value(dest, Data::float(x)));
    let bool_value = |dest: u16, b: Option<bool>| b.map(|b| Folded::Value(dest, Data::bool(b)));
    let branch = |taken: Option<bool>| taken.map(Folded::Branch);
    match instr {
        Instr::AddInt(a, b, d) => int_value(d, int_op(a, b, i64::checked_add)),
        Instr::SubInt(a, b, d) => int_value(d, int_op(a, b, i64::checked_sub)),
        Instr::MulInt(a, b, d) => int_value(d, int_op(a, b, i64::checked_mul)),
        // `None` for a zero divisor, and for the one quotient that overflows.
        Instr::DivInt(a, b, d) => int_value(d, int_op(a, b, i64::checked_div)),
        Instr::ModInt(a, b, d) => int_value(d, int_op(a, b, i64::checked_rem)),
        Instr::PowInt(a, b, d) => int_value(
            d,
            int(a).and_then(|x| x.checked_pow(u32::try_from(int(b)?).ok()?)),
        ),
        Instr::BitAndInt(a, b, d) => int_value(d, int_op(a, b, |x, y| Some(x & y))),
        Instr::BitOrInt(a, b, d) => int_value(d, int_op(a, b, |x, y| Some(x | y))),
        Instr::BitXorInt(a, b, d) => int_value(d, int_op(a, b, |x, y| Some(x ^ y))),
        Instr::BitNotInt(a, d) => int_value(d, int(a).map(|x| !x)),
        Instr::ShlInt(a, b, d) => int_value(
            d,
            int_op(a, b, |x, count| {
                shift_count_in_range(count).then(|| x << count)
            }),
        ),
        Instr::ShrInt(a, b, d) => int_value(
            d,
            int_op(a, b, |x, count| {
                shift_count_in_range(count).then(|| x >> count)
            }),
        ),
        Instr::NegInt(a, d) => int_value(d, int(a).and_then(i64::checked_neg)),
        Instr::AddIntImm(a, imm, d) => int_value(d, int(a).and_then(|x| x.checked_add(imm.into()))),
        // The VM steps an `int` by one with wrapping arithmetic.
        Instr::IncIntTo(a, d) => int_value(d, int(a).map(|x| x.wrapping_add(1))),
        Instr::DecIntTo(a, d) => int_value(d, int(a).map(|x| x.wrapping_sub(1))),
        Instr::IncInt(r) => int_value(r, int(r).map(|x| x.wrapping_add(1))),
        Instr::DecInt(r) => int_value(r, int(r).map(|x| x.wrapping_sub(1))),
        Instr::AddFloat(a, b, d) => float_value(d, float_op(a, b, |x, y| x + y)),
        Instr::SubFloat(a, b, d) => float_value(d, float_op(a, b, |x, y| x - y)),
        Instr::MulFloat(a, b, d) => float_value(d, float_op(a, b, |x, y| x * y)),
        Instr::DivFloat(a, b, d) => float_value(d, float_op(a, b, |x, y| x / y)),
        Instr::ModFloat(a, b, d) => float_value(d, float_op(a, b, |x, y| x % y)),
        Instr::NegFloat(a, d) => float_value(d, float(a).map(|x| -x)),
        Instr::SupInt(a, b, d) => bool_value(d, int_cmp(a, b, i64::gt)),
        Instr::SupEqInt(a, b, d) => bool_value(d, int_cmp(a, b, i64::ge)),
        Instr::InfInt(a, b, d) => bool_value(d, int_cmp(a, b, i64::lt)),
        Instr::InfEqInt(a, b, d) => bool_value(d, int_cmp(a, b, i64::le)),
        Instr::SupFloat(a, b, d) => bool_value(d, float_cmp(a, b, f64::gt)),
        Instr::SupEqFloat(a, b, d) => bool_value(d, float_cmp(a, b, f64::ge)),
        Instr::InfFloat(a, b, d) => bool_value(d, float_cmp(a, b, f64::lt)),
        Instr::InfEqFloat(a, b, d) => bool_value(d, float_cmp(a, b, f64::le)),
        Instr::Eq(a, b, d) => bool_value(d, Some(plain(a)? == plain(b)?)),
        Instr::NotEq(a, b, d) => bool_value(d, Some(plain(a)? != plain(b)?)),
        Instr::StrEq(a, b, d) => bool_value(d, str_cmp(a, b, |x, y| x == y)),
        Instr::StrNotEq(a, b, d) => bool_value(d, str_cmp(a, b, |x, y| x != y)),
        Instr::SupStr(a, b, d) => bool_value(d, str_cmp(a, b, |x, y| x > y)),
        Instr::SupEqStr(a, b, d) => bool_value(d, str_cmp(a, b, |x, y| x >= y)),
        Instr::InfStr(a, b, d) => bool_value(d, str_cmp(a, b, |x, y| x < y)),
        Instr::InfEqStr(a, b, d) => bool_value(d, str_cmp(a, b, |x, y| x <= y)),
        Instr::BoolAnd(a, b, d) => bool_value(d, Some(boolean(a)? && boolean(b)?)),
        Instr::BoolOr(a, b, d) => bool_value(d, Some(boolean(a)? || boolean(b)?)),
        Instr::NegBool(a, d) => bool_value(d, boolean(a).map(|x| !x)),
        Instr::AddStr(a, b, d) if intern => {
            let (a, b) = (text(a)?, text(b)?);
            let joined = format!("{}{}", a.as_str(strings), b.as_str(strings));
            Some(Folded::Value(d, Data::p_str(&joined, strings)))
        }
        Instr::IsFalseJmp(c, _) => branch(boolean(c).map(|x| !x)),
        Instr::IsTrueJmp(c, _) => branch(boolean(c)),
        Instr::SupIntJmp(a, b, _) => branch(int_cmp(a, b, i64::gt)),
        Instr::SupEqIntJmp(a, b, _) => branch(int_cmp(a, b, i64::ge)),
        Instr::InfIntJmp(a, b, _) | Instr::InfIntJmpBack(a, b, _) => branch(int_cmp(a, b, i64::lt)),
        Instr::InfEqIntJmp(a, b, _) => branch(int_cmp(a, b, i64::le)),
        Instr::SupFloatJmp(a, b, _) => branch(float_cmp(a, b, f64::gt)),
        Instr::SupEqFloatJmp(a, b, _) => branch(float_cmp(a, b, f64::ge)),
        Instr::InfFloatJmp(a, b, _) => branch(float_cmp(a, b, f64::lt)),
        Instr::InfEqFloatJmp(a, b, _) => branch(float_cmp(a, b, f64::le)),
        Instr::EqJmp(a, b, _) => branch(Some(plain(a)? == plain(b)?)),
        Instr::NotEqJmp(a, b, _) => branch(Some(plain(a)? != plain(b)?)),
        Instr::StrEqJmp(a, b, _) => branch(str_cmp(a, b, |x, y| x == y)),
        Instr::StrNotEqJmp(a, b, _) => branch(str_cmp(a, b, |x, y| x != y)),
        _ => None,
    }
}

/// Marks as dropped a move or a load whose register the next few
/// instructions write again before anything reads it. Only plain register
/// work may stand between the two: an instruction that can raise an error or
/// call anything could let a catch or a saved frame see the first value.
fn drop_overwritten(lands: &[bool], instructions: &[Instr], dropped: &mut [bool]) {
    for pos in 0..instructions.len() {
        if dropped[pos] {
            continue;
        }
        let (Instr::Mov(_, dest) | Instr::SetInt(dest, _) | Instr::SetBool(_, dest)) =
            instructions[pos]
        else {
            continue;
        };
        if dest == 0 {
            continue;
        }
        for at in pos + 1..instructions.len() {
            if dropped[at] {
                continue;
            }
            let instr = instructions[at];
            let mut reads = false;
            instr.for_each_read_reg(|reg| reads |= reg == dest);
            if lands[at] || reads || !plain(instr) {
                break;
            }
            let mut writes = false;
            instr.for_each_write_reg(|reg| writes |= reg == dest);
            if writes {
                dropped[pos] = true;
                break;
            }
        }
    }
}

/// Whether `instr` only works on registers: it cannot raise an error, call
/// anything, branch, or write a register later than where it stands.
const fn plain(instr: Instr) -> bool {
    matches!(
        instr,
        Instr::Mov(..)
            | Instr::MovChain(..)
            | Instr::SetInt(..)
            | Instr::SetBool(..)
            | Instr::AddInt(..)
            | Instr::SubInt(..)
            | Instr::MulInt(..)
            | Instr::AddIntImm(..)
            | Instr::IncIntTo(..)
            | Instr::DecIntTo(..)
            | Instr::NegInt(..)
            | Instr::BitAndInt(..)
            | Instr::BitOrInt(..)
            | Instr::BitXorInt(..)
            | Instr::BitNotInt(..)
            | Instr::AddFloat(..)
            | Instr::SubFloat(..)
            | Instr::MulFloat(..)
            | Instr::DivFloat(..)
            | Instr::ModFloat(..)
            | Instr::NegFloat(..)
            | Instr::SupInt(..)
            | Instr::SupEqInt(..)
            | Instr::InfInt(..)
            | Instr::InfEqInt(..)
            | Instr::SupFloat(..)
            | Instr::SupEqFloat(..)
            | Instr::InfFloat(..)
            | Instr::InfEqFloat(..)
            | Instr::Eq(..)
            | Instr::NotEq(..)
            | Instr::BoolAnd(..)
            | Instr::BoolOr(..)
            | Instr::NegBool(..)
    )
}

/// Marks a move or a load into a register nothing reads as dropped, and does
/// so again for a move whose own source that left unread, until none is left.
/// A register something outside the instructions reads, one a call saves,
/// and a literal's register are always kept.
fn drop_unread_writes(
    program: &Program<'_>,
    outside: &FxHashSet<u16>,
    instructions: &[Instr],
    dropped: &mut [bool],
) {
    let mut kept: FxHashSet<u16> = outside.clone();
    for list in program.callsite_registers.iter() {
        kept.extend(list.iter().copied());
    }
    kept.extend(program.const_registers.values().copied());
    let mut reads: FxHashMap<u16, usize> = FxHashMap::default();
    for (pos, instr) in instructions.iter().enumerate() {
        if !dropped[pos] {
            instr.for_each_read_reg(|reg| *reads.entry(reg).or_default() += 1);
        }
    }
    loop {
        let mut changed = false;
        for (pos, &instr) in instructions.iter().enumerate() {
            if dropped[pos] {
                continue;
            }
            let (Instr::Mov(_, dest) | Instr::SetInt(dest, _) | Instr::SetBool(_, dest)) = instr
            else {
                continue;
            };
            if kept.contains(&dest) || reads.get(&dest).is_some_and(|&n| n > 0) {
                continue;
            }
            dropped[pos] = true;
            changed = true;
            if let Instr::Mov(from, _) = instr
                && let Some(n) = reads.get_mut(&from)
            {
                *n -= 1;
            }
        }
        if !changed {
            break;
        }
    }
}
