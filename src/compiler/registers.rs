// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::data::Data;
use crate::instr::Instr;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// Redirects the value produced by the trailing instructions of `x` into
/// `tgt_id`.
///
/// Returns whether the value now lands in `tgt_id`. `false` means the emitted
/// instructions do not write the value into any register the rewrite can
/// retarget, which is what happens when a literal is built directly into the
/// constant pool: the value sits in a register nothing wrote, and the caller
/// moves it across itself.
pub fn move_to_id(x: &mut [Instr], tgt_id: u16) -> bool {
    if x.is_empty()
        || matches!(
            x.last().unwrap(),
            // A pooled array fill, and the in-place integer steps, leave the
            // value where it already is rather than writing it somewhere new.
            Instr::ObjElemMov(_, _, _)
                | Instr::MapInsert(_, _, _)
                | Instr::IncInt(_)
                | Instr::DecInt(_)
        )
    {
        return false;
    }
    let matching_elem_index = x
        .iter()
        .rposition(|w| w.get_tgt_id().is_some())
        .unwrap_or(x.len() - 1);
    // A struct, enum, map or array construction is a group: one instruction
    // that allocates the value into a destination register, then writes that
    // fill it in and name that same register. Rewriting the allocation alone
    // would leave those writes pointing at the old register, so the whole group
    // moves to `tgt_id` together.
    let constructed_dest = match x[matching_elem_index] {
        Instr::CloneStruct(_, dest)
        | Instr::CloneEnum(_, dest)
        | Instr::CloneMap(_, dest)
        | Instr::CloneArray(_, dest, _)
        | Instr::EmptyArray(dest)
        | Instr::EmptyFnValue(dest) => Some(dest),
        _ => None,
    };
    if let Some(old) = constructed_dest {
        match x.get_mut(matching_elem_index).unwrap() {
            Instr::CloneStruct(_, y)
            | Instr::CloneEnum(_, y)
            | Instr::CloneMap(_, y)
            | Instr::CloneArray(_, y, _)
            | Instr::EmptyArray(y)
            | Instr::EmptyFnValue(y) => *y = tgt_id,
            _ => {}
        }
        for instr in &mut x[matching_elem_index + 1..] {
            // `MapInsert` and `ObjElemMov` address the constant pool rather
            // than a register, so they are left alone.
            match instr {
                Instr::SetFieldStruct(reg, _, _)
                | Instr::MapInsertReg(reg, _, _)
                | Instr::Push(reg, _)
                | Instr::SetElementObj(reg, _, _)
                    if *reg == old =>
                {
                    *reg = tgt_id;
                }
                _ => {}
            }
        }
        return true;
    }
    let matching_elem = x.get_mut(matching_elem_index).unwrap();
    match matching_elem {
        Instr::Mov(_, y)
        | Instr::SetInt(y, _)
        | Instr::SetBool(_, y)
        | Instr::CallFunc(_, y)
        | Instr::AddFloat(_, _, y)
        | Instr::AddInt(_, _, y)
        | Instr::AddIntImm(_, _, y)
        | Instr::AddArray(_, _, y)
        | Instr::AddStr(_, _, y)
        | Instr::MulFloat(_, _, y)
        | Instr::MulInt(_, _, y)
        | Instr::SubFloat(_, _, y)
        | Instr::SubInt(_, _, y)
        | Instr::DivFloat(_, _, y)
        | Instr::DivInt(_, _, y)
        | Instr::ModFloat(_, _, y)
        | Instr::ModInt(_, _, y)
        | Instr::PowFloat(_, _, y)
        | Instr::PowInt(_, _, y)
        | Instr::BitAndInt(_, _, y)
        | Instr::BitOrInt(_, _, y)
        | Instr::BitXorInt(_, _, y)
        | Instr::BitNotInt(_, y)
        | Instr::ShlInt(_, _, y)
        | Instr::ShrInt(_, _, y)
        | Instr::Eq(_, _, y)
        | Instr::ObjEq(_, _, y)
        | Instr::StrEq(_, _, y)
        | Instr::NotEq(_, _, y)
        | Instr::ObjNotEq(_, _, y)
        | Instr::StrNotEq(_, _, y)
        | Instr::SupFloat(_, _, y)
        | Instr::SupInt(_, _, y)
        | Instr::SupEqFloat(_, _, y)
        | Instr::SupEqInt(_, _, y)
        | Instr::InfFloat(_, _, y)
        | Instr::InfInt(_, _, y)
        | Instr::InfEqFloat(_, _, y)
        | Instr::InfEqInt(_, _, y)
        | Instr::BoolAnd(_, _, y)
        | Instr::BoolOr(_, _, y)
        | Instr::NegBool(_, y)
        | Instr::NegFloat(_, y)
        | Instr::NegInt(_, y)
        | Instr::CallLibFunc(_, _, y)
        | Instr::GetIndexArray(_, _, y)
        | Instr::GetFieldStruct(_, _, y)
        | Instr::GetSliceArray(_, _, y)
        | Instr::GetIndexString(_, _, y)
        | Instr::GetSliceString(_, _, y)
        | Instr::SaveFrame(_, y, _)
        | Instr::CallDynamicLibFunc(_, y)
        | Instr::CallHostFunc(_, y)
        | Instr::MapGet(_, _, y)
        | Instr::IncIntTo(_, y)
        | Instr::DecIntTo(_, y) => *y = tgt_id,
        Instr::CallFuncRecursive(_, y_func) => {
            *y_func = tgt_id;
            // The frame the call returns through is the `SaveFrame` whose
            // offset lands on this call. A call made for one of its arguments
            // saves a frame of its own in between, so the nearest one is not
            // necessarily it.
            for (pos, instr) in x[..matching_elem_index].iter_mut().enumerate().rev() {
                if let Instr::SaveFrame(offset, y_frame, _) = instr
                    && pos + *offset as usize == matching_elem_index
                {
                    *y_frame = tgt_id;
                    break;
                }
            }
        }
        // Any other instruction writes somewhere this rewrite cannot follow.
        // The caller moves the value across instead.
        _ => return false,
    }
    true
}

/// Returns the IDs of all the registers which are modified by the given instructions
#[must_use]
pub fn get_tgt_ids(x: &[Instr]) -> Vec<u16> {
    let mut ids: Vec<u16> = x.iter().filter_map(|i| i.get_tgt_id()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The `SetInt` operand for `v`, or `None` when `v` is no `int` or is one too
/// wide for that operand to carry.
#[must_use]
pub fn int_immediate(v: Data) -> Option<i32> {
    if v.is_int() {
        i32::try_from(v.as_int()).ok()
    } else {
        None
    }
}

/// Write v, located in the src_id register, into the dest_id register using the cheapest instruction
#[inline(always)]
pub fn move_reg_to_reg(output: &mut Vec<Instr>, src_id: u16, dest_id: u16, v: Data) {
    // An `int` the immediate operand cannot hold is moved out of its register
    // like any other value.
    if let Some(n) = int_immediate(v) {
        output.push(Instr::SetInt(dest_id, n));
    } else if v.is_bool() {
        output.push(Instr::SetBool(v.as_bool(), dest_id));
    } else {
        output.push(Instr::Mov(src_id, dest_id));
    }
}

/// Rewrites additions of a constant `int` into the form that carries the
/// constant in the instruction, so the VM reads one register rather than two.
///
/// A constant register is one the compiler filled before the run that no
/// instruction writes: an entry of `const_registers` that neither `rest`, the
/// program the instructions join, nor the instructions themselves name as a
/// target. A variable can be given the register of the literal it starts
/// from, and a register some instruction writes holds whatever was last
/// written there, so only one nothing writes is carried as a constant. Only a
/// constant that fits the instruction's 16-bit operand moves into it. Each
/// instruction is replaced one for one, so no jump distance changes.
pub fn use_immediates(
    instructions: &mut [Instr],
    rest: &[Instr],
    registers: &[Data],
    const_registers: &FxHashMap<Data, u16>,
) {
    let written: FxHashSet<u16> = rest
        .iter()
        .chain(instructions.iter())
        .filter_map(|instr| instr.get_tgt_id())
        .collect();
    let constants: FxHashSet<u16> = const_registers
        .values()
        .copied()
        .filter(|reg| !written.contains(reg))
        .collect();
    let constant = |reg: u16| -> Option<i16> {
        if !constants.contains(&reg) {
            return None;
        }
        let value = *registers.get(reg as usize)?;
        if value.is_int() {
            i16::try_from(value.as_int()).ok()
        } else {
            None
        }
    };
    for instr in instructions.iter_mut() {
        let replacement = match *instr {
            Instr::AddInt(a, b, dest) => match (constant(a), constant(b)) {
                (None, Some(imm)) => Some(Instr::AddIntImm(a, imm, dest)),
                (Some(imm), None) => Some(Instr::AddIntImm(b, imm, dest)),
                _ => None,
            },
            // `x - k` is `x + -k`, for a `k` whose negation still fits.
            Instr::SubInt(a, b, dest) => match (constant(a), constant(b)) {
                (None, Some(imm)) => imm.checked_neg().map(|neg| Instr::AddIntImm(a, neg, dest)),
                _ => None,
            },
            _ => None,
        };
        if let Some(replacement) = replacement {
            *instr = replacement;
        }
    }
}
