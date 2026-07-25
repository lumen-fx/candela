use crate::data::Data;
use crate::instr::Instr;
use std::hint::unreachable_unchecked;

pub fn move_to_id(x: &mut [Instr], tgt_id: u16) {
    if x.is_empty()
        || matches!(
            x.last().unwrap(),
            Instr::ObjElemMov(_, _, _) | Instr::IncInt(_) | Instr::DecInt(_)
        )
    {
        return;
    }
    let matching_elem_index = x
        .iter()
        .rposition(|w| w.get_tgt_id().is_some())
        .unwrap_or(x.len() - 1);
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
        | Instr::EmptyArray(y)
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
        _ => unsafe { unreachable_unchecked() },
    }
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
