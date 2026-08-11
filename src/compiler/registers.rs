use crate::data::Data;
use crate::instr::Instr;

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
        | Instr::EmptyArray(dest) => Some(dest),
        _ => None,
    };
    if let Some(old) = constructed_dest {
        match x.get_mut(matching_elem_index).unwrap() {
            Instr::CloneStruct(_, y)
            | Instr::CloneEnum(_, y)
            | Instr::CloneMap(_, y)
            | Instr::CloneArray(_, y, _)
            | Instr::EmptyArray(y) => *y = tgt_id,
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
            for i in 1..x.len() - 1 {
                if let Some(Instr::SaveFrame(_, y_frame, _)) = x.get_mut(matching_elem_index - i) {
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

/// Write v, located in the src_id register, into the dest_id register using the cheapest instruction
#[inline(always)]
pub fn move_reg_to_reg(output: &mut Vec<Instr>, src_id: u16, dest_id: u16, v: Data) {
    if v.is_int() {
        output.push(Instr::SetInt(dest_id, v.as_int()));
    } else if v.is_bool() {
        output.push(Instr::SetBool(v.as_bool(), dest_id));
    } else {
        output.push(Instr::Mov(src_id, dest_id));
    }
}
