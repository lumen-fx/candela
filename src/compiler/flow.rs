//! Where control can go in compiled code, and the bookkeeping a pass that
//! changes how many instructions a program has must do to keep every
//! recorded position right.

use crate::compiler::compiler_data::Function;
use crate::compiler::compiler_data::IndirectRegisters;
use crate::compiler::compiler_data::InstrSrc;
use crate::data::Data;
use crate::instr::Instr;
use crate::vm::ObjectPool;
use rustc_hash::FxHashMap;

/// Everything of a compiled program a whole-program pass reads or rewrites.
pub struct Program<'a> {
    pub instructions: &'a mut Vec<Instr>,
    pub registers: &'a mut Vec<Data>,
    pub objs: &'a ObjectPool,
    pub const_registers: &'a mut FxHashMap<Data, u16>,
    pub instr_src: &'a mut Vec<InstrSrc>,
    pub callsite_registers: &'a mut [Vec<u16>],
    pub functions: &'a mut [Function],
    pub indirect: &'a IndirectRegisters,
}

/// Where the branch at `pos` can jump to, for an instruction that branches
/// within the code.
#[must_use]
pub const fn branch_target(pos: usize, instr: Instr) -> Option<usize> {
    match instr {
        Instr::JmpBack(size)
        | Instr::InfIntJmpBack(_, _, size)
        | Instr::StepIntJmpBack(_, _, size) => pos.checked_sub(size as usize),
        _ => match forward_size(instr) {
            Some(size) => Some(pos + size as usize),
            None => None,
        },
    }
}

/// Whether execution can leave the straight line at this instruction: it
/// branches, returns, throws or stops.
#[must_use]
pub const fn ends_straight_run(instr: Instr) -> bool {
    branch_target(0, instr).is_some()
        || matches!(
            instr,
            Instr::JmpBack(_)
                | Instr::InfIntJmpBack(_, _, _)
                | Instr::StepIntJmpBack(_, _, _)
                | Instr::Return(_)
                | Instr::RecursiveReturn(_)
                | Instr::VoidReturn
                | Instr::ThrowError(_)
                | Instr::Halt(_)
                | Instr::StopErrorCatch
        )
}

/// Whether this instruction runs other candela code before the next one.
#[must_use]
pub const fn is_call(instr: Instr) -> bool {
    matches!(
        instr,
        Instr::CallFunc(_, _) | Instr::CallFuncRecursive(_, _) | Instr::CallIndirect(_)
    )
}

/// Where control can go after the instruction at `pos`: the next instruction,
/// a branch target, both, or neither. A call counts as going on to the next
/// instruction, where it returns to; a `try` also goes to its catch.
#[must_use]
pub const fn successors(pos: usize, instr: Instr) -> [Option<usize>; 2] {
    match instr {
        Instr::Jmp(size) => [Some(pos + size as usize), None],
        Instr::JmpBack(size) => [pos.checked_sub(size as usize), None],
        Instr::Return(_)
        | Instr::RecursiveReturn(_)
        | Instr::VoidReturn
        | Instr::Halt(_)
        | Instr::ThrowError(_)
        | Instr::NativeExit => [None, None],
        _ => [Some(pos + 1), branch_target(pos, instr)],
    }
}

/// The instructions a function starting at `entry` runs, in order: every one
/// reachable from it without entering a call.
#[must_use]
pub fn body(instructions: &[Instr], entry: usize) -> Vec<usize> {
    let mut seen = vec![false; instructions.len()];
    let mut stack = vec![entry];
    while let Some(pos) = stack.pop() {
        if pos >= instructions.len() || seen[pos] {
            continue;
        }
        seen[pos] = true;
        for next in successors(pos, instructions[pos]).into_iter().flatten() {
            stack.push(next);
        }
    }
    seen.iter()
        .enumerate()
        .filter_map(|(pos, reached)| reached.then_some(pos))
        .collect()
}

/// For each position, and one past the end, whether a branch can land there.
#[must_use]
pub fn branch_targets(instructions: &[Instr]) -> Vec<bool> {
    let mut lands = vec![false; instructions.len() + 1];
    for (pos, instr) in instructions.iter().enumerate() {
        if let Some(target) = branch_target(pos, *instr)
            && let Some(slot) = lands.get_mut(target)
        {
            *slot = true;
        }
    }
    lands
}

/// The distance of a forward branch, for an instruction that makes one.
pub const fn forward_size(instr: Instr) -> Option<u16> {
    match instr {
        Instr::Jmp(size)
        | Instr::IsFalseJmp(_, size)
        | Instr::IsTrueJmp(_, size)
        | Instr::SupEqFloatJmp(_, _, size)
        | Instr::SupEqIntJmp(_, _, size)
        | Instr::SupFloatJmp(_, _, size)
        | Instr::SupIntJmp(_, _, size)
        | Instr::InfEqFloatJmp(_, _, size)
        | Instr::InfEqIntJmp(_, _, size)
        | Instr::InfFloatJmp(_, _, size)
        | Instr::InfIntJmp(_, _, size)
        | Instr::NotEqJmp(_, _, size)
        | Instr::EqJmp(_, _, size)
        | Instr::ObjNotEqJmp(_, _, size)
        | Instr::ObjEqJmp(_, _, size)
        | Instr::StrNotEqJmp(_, _, size)
        | Instr::StrEqJmp(_, _, size)
        | Instr::StartErrorCatch(size, _) => Some(size),
        _ => None,
    }
}

/// Moves every position the program records to where the rewrite put it:
/// the distance of each branch and each call's return, each call target,
/// where each function's body starts, and each function value's entry.
/// `map[old]` is the new position of the instruction that was at `old`, with
/// one more entry for the end. An instruction `replaced` marks was rewritten
/// into ones that name no position; any other keeps its distances, whatever
/// else the pass changed in it, and gets them moved here.
pub fn relocate(
    program: &mut Program<'_>,
    old: &[Instr],
    replaced: &[bool],
    new: &mut [Instr],
    map: &[usize],
) {
    for pos in 0..old.len() {
        if replaced[pos] {
            continue;
        }
        let at = map[pos];
        let before = new[at];
        let moved = |target: usize| map[target];
        let mut after = before;
        match &mut after {
            Instr::JmpBack(size)
            | Instr::InfIntJmpBack(_, _, size)
            | Instr::StepIntJmpBack(_, _, size) => {
                *size = (at - moved(pos - *size as usize)) as u16;
            }
            Instr::SaveFrame(offset, _, _) => {
                *offset = (moved(pos + *offset as usize) - at) as u16;
            }
            Instr::CallFunc(loc, _) | Instr::CallFuncRecursive(loc, _) => {
                *loc = moved(*loc as usize) as u16;
            }
            instr => {
                if let Some(size) = forward_size(*instr) {
                    let target = moved(pos + size as usize);
                    set_forward_size(instr, (target - at) as u16);
                }
            }
        }
        if after != before {
            new[at] = after;
            if let Some(src) = program.instr_src.iter().find(|src| src.instr == before) {
                let (span, file_id) = (src.span, src.file_id);
                program.instr_src.push(InstrSrc {
                    instr: after,
                    span,
                    file_id,
                });
            }
        }
    }

    for function in program.functions.iter_mut() {
        for fn_impl in &mut function.impls {
            fn_impl.loc = map[fn_impl.loc as usize] as u16;
        }
        if let Some(entry) = function.entry_register {
            let value = program.registers[entry as usize];
            if value.is_int() {
                program.registers[entry as usize] = Data::int(map[value.as_int() as usize] as i64);
            }
        }
    }
}

/// Writes a new distance into a forward branch.
pub const fn set_forward_size(instr: &mut Instr, new_size: u16) {
    match instr {
        Instr::Jmp(size)
        | Instr::IsFalseJmp(_, size)
        | Instr::IsTrueJmp(_, size)
        | Instr::SupEqFloatJmp(_, _, size)
        | Instr::SupEqIntJmp(_, _, size)
        | Instr::SupFloatJmp(_, _, size)
        | Instr::SupIntJmp(_, _, size)
        | Instr::InfEqFloatJmp(_, _, size)
        | Instr::InfEqIntJmp(_, _, size)
        | Instr::InfFloatJmp(_, _, size)
        | Instr::InfIntJmp(_, _, size)
        | Instr::NotEqJmp(_, _, size)
        | Instr::EqJmp(_, _, size)
        | Instr::ObjNotEqJmp(_, _, size)
        | Instr::ObjEqJmp(_, _, size)
        | Instr::StrNotEqJmp(_, _, size)
        | Instr::StrEqJmp(_, _, size)
        | Instr::StartErrorCatch(size, _) => *size = new_size,
        _ => {}
    }
}
