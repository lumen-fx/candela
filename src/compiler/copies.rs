//! Copy forwarding, a release-profile pass: a move into a register that is
//! only read a few instructions later is dropped, and those reads take the
//! moved register instead.
//!
//! The compiler works a value out into one register and moves it into the one
//! that wanted it in many places, and the passes before this one add moves of
//! their own (a field kept in a register is read with a move). A move `t = a`
//! can go when it is the only instruction that ever writes `t`, every read of
//! `t` follows it in the same straight run of instructions, and nothing in
//! that stretch writes `a`: each read then sees exactly what `a` holds.

use crate::compiler::compiler_data::InstrSrc;
use crate::compiler::flow::Program;
use crate::compiler::flow::branch_targets;
use crate::compiler::flow::ends_straight_run;
use crate::compiler::flow::is_call;
use crate::compiler::flow::relocate;
use crate::instr::Instr;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// Drops every move the module documentation describes.
pub fn forward_copies(program: &mut Program<'_>) {
    let mut instructions = program.instructions.clone();
    let lands = branch_targets(&instructions);

    // Registers something besides an instruction here reads or writes: the
    // parameters a call fills, the registers an indirect call passes, function
    // entries, the null sink, and every register a call saves and puts back.
    let mut excluded: FxHashSet<u16> = FxHashSet::default();
    excluded.insert(0);
    excluded.extend(program.indirect.args.iter().copied());
    excluded.extend(program.indirect.env);
    for function in program.functions.iter() {
        excluded.extend(function.entry_register);
        for fn_impl in &function.impls {
            excluded.extend(fn_impl.args_loc.iter().copied());
            excluded.extend(fn_impl.env_loc);
        }
    }
    for list in program.callsite_registers.iter() {
        excluded.extend(list.iter().copied());
    }

    let mut writes: FxHashMap<u16, usize> = FxHashMap::default();
    let mut reads: FxHashMap<u16, Vec<usize>> = FxHashMap::default();
    for (pos, instr) in instructions.iter().enumerate() {
        if let Some(reg) = instr.get_tgt_id() {
            *writes.entry(reg).or_default() += 1;
        }
        instr.for_each_read_reg(|reg| reads.entry(reg).or_default().push(pos));
    }

    let mut dropped = vec![false; instructions.len()];
    for pos in 0..instructions.len() {
        let Instr::Mov(from, to) = instructions[pos] else {
            continue;
        };
        if from == to || excluded.contains(&to) || writes.get(&to) != Some(&1) {
            continue;
        }
        let Some(uses) = reads.get(&to).cloned() else {
            continue;
        };
        // The straight run the move starts: every instruction up to and
        // including the first that can leave it, stopping before any a branch
        // can land on.
        let mut end = pos;
        while end + 1 < instructions.len() && !lands[end + 1] {
            end += 1;
            let instr = instructions[end];
            if ends_straight_run(instr) || is_call(instr) {
                break;
            }
        }
        let last = *uses.iter().max().unwrap_or(&pos);
        // A stored call argument is read when the call runs, not where it is
        // stored, so a register handed to one keeps its move.
        if uses
            .iter()
            .any(|&at| at <= pos || at > end || matches!(instructions[at], Instr::StoreFuncArg(_)))
        {
            continue;
        }
        let from_written = instructions[pos + 1..last]
            .iter()
            .any(|instr| instr.get_tgt_id() == Some(from));
        if from_written {
            continue;
        }
        for &at in &uses {
            let old = instructions[at];
            let mut new = old;
            new.read_regs_mut(|reg| {
                if *reg == to {
                    *reg = from;
                }
            });
            if new != old {
                carry_span(program.instr_src, old, new);
                instructions[at] = new;
            }
        }
        // The reads that moved now read `from`; keep the tallies right for the
        // moves still to be looked at.
        reads.entry(from).or_default().extend(uses.iter().copied());
        reads.remove(&to);
        dropped[pos] = true;
    }

    if !dropped.contains(&true) {
        return;
    }
    let mut map: Vec<usize> = Vec::with_capacity(instructions.len() + 1);
    let mut new: Vec<Instr> = Vec::with_capacity(instructions.len());
    for (pos, &instr) in instructions.iter().enumerate() {
        map.push(new.len());
        if !dropped[pos] {
            new.push(instr);
        }
    }
    map.push(new.len());
    let old = std::mem::take(program.instructions);
    relocate(program, &old, &dropped, &mut new, &map);
    *program.instructions = new;
}

/// Gives `new`, rewritten from `old`, the span `old` has.
fn carry_span(instr_src: &mut Vec<InstrSrc>, old: Instr, new: Instr) {
    if let Some(src) = instr_src.iter().find(|src| src.instr == old) {
        let (span, file_id) = (src.span, src.file_id);
        instr_src.push(InstrSrc {
            instr: new,
            span,
            file_id,
        });
    }
}
