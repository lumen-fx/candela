use crate::cold_path;
use crate::compiler::compiler_data::InstrSrc;
use crate::compiler::compiler_data::Source;
use crate::compiler::compiler_errors::error_cannot_find_dynlib_symbol;
use crate::compiler::compiler_errors::error_cannot_load_dynlib;
use crate::compiler::compiler_errors::error_cannot_push_type_to_array;
use crate::compiler::compiler_errors::error_cannot_read_file;
use crate::compiler::compiler_errors::error_conditional_expression_without_else;
use crate::compiler::compiler_errors::error_division_by_zero;
use crate::compiler::compiler_errors::error_duplicate_map_key;
use crate::compiler::compiler_errors::error_invalid_index_type;
use crate::compiler::compiler_errors::error_invalid_type;
use crate::compiler::compiler_errors::error_map_diff_types;
use crate::compiler::compiler_errors::error_not_literal_map_key;
use crate::compiler::compiler_errors::error_range_invalid_type;
use crate::compiler::compiler_errors::error_type_arg_count;
use crate::compiler::compiler_errors::error_type_not_indexable;
use crate::compiler::compiler_errors::error_unknown_namespace;
use crate::data::NULL;
use crate::errors::BLUE;
use crate::errors::BOLD;
use crate::errors::RED;
use crate::errors::RESET;
use crate::instr::LibFunc;
use crate::parser;
use crate::rt::TargetOs;
#[cfg(not(target_arch = "wasm32"))]
use crate::rt::dylib_dir;
use crate::rt::resolve_library_filename;
use crate::vm::Pool;
use crate::{data::Data, instr::Instr};
use compiler_data::Ctx;
use compiler_data::DynamicLibFn;
use compiler_data::Dynamiclib;
use compiler_data::EnumType;
use compiler_data::EnumVariant;
use compiler_data::FnGenerics;
use compiler_data::FnSignature;
use compiler_data::Function;
use compiler_data::HostFnSig;
use compiler_data::Pools;
use compiler_data::State;
use compiler_data::Struct;
use compiler_data::Variable;
use expr::Expr;
use expr::METHOD_SEP;
use expr::Span;
use expr::code_modifies_variable;
use functions::handle_functions;
use methods::handle_method_calls;
use registers::move_reg_to_reg;
use registers::move_to_id;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::hint::unreachable_unchecked;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use type_system::DataType;
use type_system::Generics;
use type_system::ReturnAnnotation;
use type_system::TypeCtx;
use type_system::TypeExpr;
use type_system::TypeParams;
use type_system::check_if_returns_void;
use type_system::collect_direct_fn_calls;
use type_system::resolve_generic_variant;
use type_system::struct_field_type_matches;
use type_system::struct_literal_id;

#[cfg(not(target_arch = "wasm32"))]
use libloading::Library;

#[cfg(target_arch = "wasm32")]
use crate::errors::wasm_error;
pub mod compiler_data;
mod compiler_errors;
pub mod type_system;

pub mod expr;

#[path = "functions/functions.rs"]
mod functions;
#[path = "functions/methods.rs"]
mod methods;

mod registers;

pub trait UnwrapId {
    fn unwrap_id(self) -> u16;
}

impl UnwrapId for Option<u16> {
    #[inline(always)]
    fn unwrap_id(self) -> u16 {
        debug_assert!(self.is_some());
        unsafe { self.unwrap_unchecked() }
    }
}

/// Fuses the last comparison instruction into a jump instruction (jumps when condition is false)
fn add_cmp_false(condition_id: u16, len: &mut u16, output: &mut Vec<Instr>, jmp_backwards: bool) {
    if output.is_empty() {
        return output.push(Instr::IsFalseJmp(condition_id, *len));
    }
    *output.last_mut().unwrap() = match *output.last().unwrap() {
        Instr::InfFloat(o1, o2, o3) if o3 == condition_id => Instr::SupEqFloatJmp(o1, o2, *len),
        Instr::InfInt(o1, o2, o3) if o3 == condition_id => Instr::SupEqIntJmp(o1, o2, *len),
        Instr::InfEqFloat(o1, o2, o3) if o3 == condition_id => Instr::SupFloatJmp(o1, o2, *len),
        Instr::InfEqInt(o1, o2, o3) if o3 == condition_id => Instr::SupIntJmp(o1, o2, *len),
        Instr::SupFloat(o1, o2, o3) if o3 == condition_id => Instr::InfEqFloatJmp(o1, o2, *len),
        Instr::SupInt(o1, o2, o3) if o3 == condition_id => Instr::InfEqIntJmp(o1, o2, *len),
        Instr::SupEqFloat(o1, o2, o3) if o3 == condition_id => Instr::InfFloatJmp(o1, o2, *len),
        Instr::SupEqInt(o1, o2, o3) if o3 == condition_id => Instr::InfIntJmp(o1, o2, *len),
        Instr::Eq(o1, o2, o3) if o3 == condition_id => Instr::NotEqJmp(o1, o2, *len),
        Instr::ObjEq(o1, o2, o3) if o3 == condition_id => Instr::ObjNotEqJmp(o1, o2, *len),
        Instr::StrEq(o1, o2, o3) if o3 == condition_id => Instr::StrNotEqJmp(o1, o2, *len),
        Instr::NotEq(o1, o2, o3) if o3 == condition_id => Instr::EqJmp(o1, o2, *len),
        Instr::ObjNotEq(o1, o2, o3) if o3 == condition_id => Instr::ObjEqJmp(o1, o2, *len),
        Instr::StrNotEq(o1, o2, o3) if o3 == condition_id => Instr::StrEqJmp(o1, o2, *len),
        _ => {
            output.push(Instr::IsFalseJmp(condition_id, *len));
            return;
        }
    };
    if jmp_backwards {
        *len -= 1;
    }
}

/// Fuses the last comparison instruction into a jump instruction (jumps when condition is true)
#[inline(always)]
fn add_cmp_true(condition_id: u16, output: &mut Vec<Instr>) {
    if output.is_empty() {
        return output.push(Instr::IsTrueJmp(condition_id, 0));
    }
    let new_instr = match *output.last().unwrap() {
        Instr::InfFloat(o1, o2, o3) if o3 == condition_id => Instr::InfFloatJmp(o1, o2, 0),
        Instr::InfInt(o1, o2, o3) if o3 == condition_id => Instr::InfIntJmp(o1, o2, 0),
        Instr::InfEqFloat(o1, o2, o3) if o3 == condition_id => Instr::InfEqFloatJmp(o1, o2, 0),
        Instr::InfEqInt(o1, o2, o3) if o3 == condition_id => Instr::InfEqIntJmp(o1, o2, 0),
        Instr::SupFloat(o1, o2, o3) if o3 == condition_id => Instr::SupFloatJmp(o1, o2, 0),
        Instr::SupInt(o1, o2, o3) if o3 == condition_id => Instr::SupIntJmp(o1, o2, 0),
        Instr::SupEqFloat(o1, o2, o3) if o3 == condition_id => Instr::SupEqFloatJmp(o1, o2, 0),
        Instr::SupEqInt(o1, o2, o3) if o3 == condition_id => Instr::SupEqIntJmp(o1, o2, 0),
        Instr::Eq(o1, o2, o3) if o3 == condition_id => Instr::EqJmp(o1, o2, 0),
        Instr::ObjEq(o1, o2, o3) if o3 == condition_id => Instr::ObjEqJmp(o1, o2, 0),
        Instr::StrEq(o1, o2, o3) if o3 == condition_id => Instr::StrEqJmp(o1, o2, 0),
        Instr::NotEq(o1, o2, o3) if o3 == condition_id => Instr::NotEqJmp(o1, o2, 0),
        Instr::ObjNotEq(o1, o2, o3) if o3 == condition_id => Instr::ObjNotEqJmp(o1, o2, 0),
        Instr::StrNotEq(o1, o2, o3) if o3 == condition_id => Instr::StrNotEqJmp(o1, o2, 0),
        _ => {
            output.push(Instr::IsTrueJmp(condition_id, 0));
            return;
        }
    };
    *output.last_mut().unwrap() = new_instr;
}

/// Sets the jump size field of a jump instruction
#[inline(always)]
const fn set_jmp_size(instr: &mut Instr, size: u16) {
    match instr {
        Instr::IsFalseJmp(_, jump_size)
        | Instr::IsTrueJmp(_, jump_size)
        | Instr::Jmp(jump_size)
        | Instr::SupEqFloatJmp(_, _, jump_size)
        | Instr::SupEqIntJmp(_, _, jump_size)
        | Instr::SupFloatJmp(_, _, jump_size)
        | Instr::SupIntJmp(_, _, jump_size)
        | Instr::InfEqFloatJmp(_, _, jump_size)
        | Instr::InfEqIntJmp(_, _, jump_size)
        | Instr::InfFloatJmp(_, _, jump_size)
        | Instr::InfIntJmp(_, _, jump_size)
        | Instr::InfIntJmpBack(_, _, jump_size)
        | Instr::NotEqJmp(_, _, jump_size)
        | Instr::EqJmp(_, _, jump_size)
        | Instr::ObjNotEqJmp(_, _, jump_size)
        | Instr::ObjEqJmp(_, _, jump_size)
        | Instr::StrNotEqJmp(_, _, jump_size)
        | Instr::StrEqJmp(_, _, jump_size) => *jump_size = size,
        _ => unsafe { unreachable_unchecked() },
    }
}

/// Compiles short-circuit && and || conditions
/// bool_or_mode true indicates left side of ||, emits true jumps
/// bool_or_mode false emits false jumps
/// Returns (true_jump_idxs, false_jump_idxs)
#[allow(clippy::too_many_arguments)]
fn compile_short_circuit_condition(
    expr: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
    bool_or_mode: bool,
) -> (Vec<usize>, Vec<usize>) {
    match expr {
        Expr::BoolOr(left, right, _, _) => {
            // left side of || always uses true jump mode
            let (mut true_jumps, left_false) =
                compile_short_circuit_condition(left, v, ctx, state, output, true);
            // A false left operand does not settle `||`, so it continues into
            // the right operand rather than out of the whole expression.
            let right_start = output.len();
            for j in left_false {
                set_jmp_size(&mut output[j], (right_start - j) as u16);
            }
            let (right_true, right_false) =
                compile_short_circuit_condition(right, v, ctx, state, output, bool_or_mode);
            true_jumps.extend(right_true);
            (true_jumps, right_false)
        }
        Expr::BoolAnd(left, right, _, _) => {
            if bool_or_mode {
                // `&&` on the left of `||`, where the caller wants jumps taken
                // when this conjunction is true. A false left operand settles
                // the conjunction, so its false jumps skip the right operand
                // and land on whatever is emitted next, which is exactly where
                // the enclosing `||` continues.
                let (_, left_false) =
                    compile_short_circuit_condition(left, v, ctx, state, output, false);
                let (right_true, _) =
                    compile_short_circuit_condition(right, v, ctx, state, output, true);
                let fallthrough = output.len();
                for j in left_false {
                    set_jmp_size(&mut output[j], (fallthrough - j) as u16);
                }
                (right_true, Vec::new())
            } else {
                // normal && -> if either side is false, jump past the body
                let (left_true, mut false_jumps) =
                    compile_short_circuit_condition(left, v, ctx, state, output, false);
                // A true left operand does not settle `&&`, so it continues
                // into the right operand. Only the right operand's true jumps
                // settle the conjunction, and the caller aims those at the body.
                let right_start = output.len();
                for j in left_true {
                    set_jmp_size(&mut output[j], (right_start - j) as u16);
                }
                let (right_true, right_false) =
                    compile_short_circuit_condition(right, v, ctx, state, output, false);
                false_jumps.extend(right_false);
                (right_true, false_jumps)
            }
        }
        expr => {
            let cond_id = expr
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if bool_or_mode {
                add_cmp_true(cond_id, output);
                state.free_reg(cond_id, v);
                (vec![output.len() - 1], Vec::new())
            } else {
                add_cmp_false(cond_id, &mut 0, output, false);
                state.free_reg(cond_id, v);
                (Vec::new(), vec![output.len() - 1])
            }
        }
    }
}

fn parse_loop_flow_control(
    loop_code: &mut [Instr],
    loop_id: u16,
    code_length: u16,
    for_loop: bool,
    indefinite: bool,
) {
    loop_code.iter_mut().enumerate().for_each(|(i, x)| {
        if let Instr::NotEqJmp(break_id, 0, 0) = x
            && *break_id == loop_id
        {
            if for_loop && !indefinite {
                *x = Instr::Jmp(code_length - i as u16 - 1);
            } else {
                *x = Instr::Jmp(code_length - i as u16);
            }
        } else if let Instr::EqJmp(continue_id, 0, 0) = x
            && *continue_id == loop_id
        {
            if for_loop {
                *x = Instr::Jmp(code_length - i as u16 - 3);
            } else {
                // loop blocks and while loops only have 1 trailing instruction
                *x = Instr::Jmp(code_length - i as u16 - 1);
            }
        }
    });
}

#[inline(always)]
fn compile_array_literal(
    array_items: &[Expr],
    spans: &[Span],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    if let Some(first) = array_items.first() {
        let first_type = first.infer_type(v, ctx, state);
        if let Some(failing_elem_idx) = array_items
            .iter()
            .skip(1)
            .position(|x| x.infer_type(v, ctx, state) != first_type)
        {
            let failing_elem_type = array_items[failing_elem_idx + 1].infer_type(v, ctx, state);
            let failing_elem_span = spans[failing_elem_idx + 2];
            compiler_errors::error_array_diff_types(
                ctx.file_idx,
                state.sources,
                spans[1],
                &first_type,
                failing_elem_span,
                &failing_elem_type,
            )
        }
    }
    let array_id = {
        state.pools.objs.push(Vec::with_capacity(array_items.len()));
        state.pools.objs.len() - 1
    };
    if array_items.is_empty() && !ctx.single_run {
        let array_reg = {
            state.registers.push(Data::array(array_id as u32));
            state.registers.len() - 1
        } as u16;
        output.push(Instr::EmptyArray(array_reg));
        return array_reg;
    }
    if ctx.single_run {
        for elem in array_items {
            let id = elem
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if elem.is_constant_literal() {
                state
                    .pools
                    .objs
                    .get_mut(array_id)
                    .push(state.registers[id as usize]);
            } else {
                output.push(Instr::ObjElemMov(
                    id,
                    array_id as u16,
                    state.pools.objs[array_id].len() as u16,
                ));
                state.pools.objs.get_mut(array_id).push(NULL);
            }
        }
        state.registers.push(Data::array(array_id as u32));
        (state.registers.len() - 1) as u16
    } else {
        // Check if all elements are constant (no instructions emitted)
        let mut constant_array = true;
        let mut elem_ids: Vec<u16> = Vec::with_capacity(array_items.len());
        for elem in array_items {
            let id = elem
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if elem.is_constant_literal() {
                state
                    .pools
                    .objs
                    .get_mut(array_id)
                    .push(state.registers[id as usize]);
            } else {
                constant_array = false;
                state.pools.objs.get_mut(array_id).push(NULL);
            }
            elem_ids.push(id);
        }

        if constant_array {
            // The template array is held by a register to prevent it from being freed by the GC
            let template_reg = {
                state.registers.push(Data::array(array_id as u32));
                (state.registers.len() - 1) as u16
            };
            let dest_reg = {
                state.registers.push(Data::array(0)); // 0 is a placeholder that's overwritten by EmptyArray
                (state.registers.len() - 1) as u16
            };
            output.push(Instr::CloneArray(
                template_reg,
                dest_reg,
                state.pools.objs[array_id].len() as u16,
            ));
            dest_reg
        } else {
            let dest_reg = {
                state.registers.push(Data::array(0)); // 0 is a placeholder that's overwritten by EmptyArray
                (state.registers.len() - 1) as u16
            };
            output.push(Instr::EmptyArray(dest_reg));
            for elem_reg in elem_ids {
                output.push(Instr::Push(dest_reg, elem_reg));
            }
            dest_reg
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_struct_literal(
    namespace: &[SmolStr],
    fields: &[(SmolStr, Expr, Span, Span)],
    type_args: &[TypeExpr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let name = &namespace[namespace.len() - 1];
    let expected_struct_idx =
        struct_literal_id(namespace, fields, type_args, span, v, ctx, state) as usize;
    let type_id = state.structs[expected_struct_idx].id;
    let expected_fields_len = state.structs[expected_struct_idx].fields.len();
    if expected_fields_len < fields.len() {
        let unexpected_field = &fields[expected_fields_len];
        compiler_errors::error_struct_no_such_field(
            ctx.file_idx,
            name,
            state.structs[expected_struct_idx].name_span,
            unexpected_field.2,
            &unexpected_field.0,
            state.sources,
        )
    }
    let struct_id = {
        state.pools.objs.push(Vec::with_capacity(fields.len()));
        state.pools.objs.len() - 1
    };
    if ctx.single_run {
        for field_idx in 0..expected_fields_len {
            if let Some((_, field_expr, _, field_value_span)) = fields
                .iter()
                .find(|(f, _, _, _)| f == &state.structs[expected_struct_idx].fields[field_idx].0)
            {
                let field_type = field_expr.infer_type(v, ctx, state);
                let field = &state.structs[expected_struct_idx].fields[field_idx];
                if !struct_field_type_matches(&field.1, &field_type) {
                    compiler_errors::error_struct_field_invalid_type(
                        ctx.file_idx,
                        name,
                        field.2,
                        &field.0,
                        &field.1,
                        *field_value_span,
                        &field_type,
                        state.sources,
                    );
                }
                let id = field_expr
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id();
                if field_expr.is_constant_literal() {
                    state
                        .pools
                        .objs
                        .get_mut(struct_id)
                        .push(state.registers[id as usize]);
                } else {
                    output.push(Instr::ObjElemMov(
                        id,
                        struct_id as u16,
                        state.pools.objs[struct_id].len() as u16,
                    ));
                    state.pools.objs.get_mut(struct_id).push(NULL);
                }
            } else {
                let missing_elems = (0..expected_fields_len)
                    .into_iter()
                    .filter(|i| {
                        !fields.iter().any(|(f, _, _, _)| {
                            f == &state.structs[expected_struct_idx].fields[*i].0
                        })
                    })
                    .map(|i| &state.structs[struct_id].fields[i].0)
                    .collect::<Vec<&SmolStr>>();
                compiler_errors::error_struct_missing_fields(
                    ctx.file_idx,
                    state.structs[expected_struct_idx].name_span,
                    span,
                    state.sources,
                    &missing_elems,
                )
            }
        }

        state
            .registers
            .push(Data::struct_instance(type_id, struct_id as u32));
        (state.registers.len() - 1) as u16
    } else {
        let mut dynamic: Vec<(u16, u16)> = Vec::with_capacity(expected_fields_len);
        for field_idx in 0..expected_fields_len {
            if let Some((_, field_expr, _, field_value_span)) = fields
                .iter()
                .find(|(f, _, _, _)| f == &state.structs[expected_struct_idx].fields[field_idx].0)
            {
                let field_type = field_expr.infer_type(v, ctx, state);
                let field = &state.structs[expected_struct_idx].fields[field_idx];
                if !struct_field_type_matches(&field.1, &field_type) {
                    compiler_errors::error_struct_field_invalid_type(
                        ctx.file_idx,
                        name,
                        field.2,
                        &field.0,
                        &field.1,
                        *field_value_span,
                        &field_type,
                        state.sources,
                    );
                }
                let id = field_expr
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id();
                if field_expr.is_constant_literal() {
                    state
                        .pools
                        .objs
                        .get_mut(struct_id)
                        .push(state.registers[id as usize]);
                } else {
                    state.pools.objs.get_mut(struct_id).push(NULL);
                    dynamic.push((id, field_idx as u16));
                }
            } else {
                let missing_elems = (0..expected_fields_len)
                    .into_iter()
                    .filter(|i| {
                        !fields.iter().any(|(f, _, _, _)| {
                            f == &state.structs[expected_struct_idx].fields[*i].0
                        })
                    })
                    .map(|i| &state.structs[struct_id].fields[i].0)
                    .collect::<Vec<&SmolStr>>();
                compiler_errors::error_struct_missing_fields(
                    ctx.file_idx,
                    state.structs[expected_struct_idx].name_span,
                    span,
                    state.sources,
                    &missing_elems,
                );
            }
        }

        let template_reg = {
            state
                .registers
                .push(Data::struct_instance(type_id, struct_id as u32));
            (state.registers.len() - 1) as u16
        };
        let dest_reg = {
            state.registers.push(Data::struct_instance(type_id, 0));
            (state.registers.len() - 1) as u16
        };
        output.push(Instr::CloneStruct(template_reg, dest_reg));
        for (val_reg, slot) in dynamic {
            output.push(Instr::SetFieldStruct(dest_reg, val_reg, slot));
        }
        dest_reg
    }
}

/// Resolves a call/reference path to an enum variant `(enum_id, variant_idx)`,
/// if it names one. A qualified path (`Color::Red`, `mod::Color::Red`) resolves
/// the enum by its leading segments and the variant by the last segment; a bare
/// name (`Some`, `None`) resolves by searching every registered enum for a
/// variant with that name, first match winning. Never raises a compile error,
/// so callers use it to intercept otherwise-unknown call/reference paths.
pub(crate) fn resolve_enum_variant(path: &[SmolStr], state: &State<'_>) -> Option<(u16, u16)> {
    if path.len() >= 2 {
        let variant = &path[path.len() - 1];
        let enum_name = &path[path.len() - 2];
        let module = &path[..path.len() - 2];
        let eid = state.namespace.find_enum(module, enum_name)?;
        let vidx = state.enums[eid]
            .variants
            .iter()
            .position(|vt| &vt.name == variant)?;
        Some((eid as u16, vidx as u16))
    } else if let Some(name) = path.first() {
        for e in state.enums.iter() {
            if let Some(vidx) = e.variants.iter().position(|vt| &vt.name == name) {
                return Some((e.id, vidx as u16));
            }
        }
        None
    } else {
        None
    }
}

/// Lowers an enum-variant construction (`Color::Red`, `Some(x)`) to a fresh
/// enum value. The object-pool template holds the variant tag at element 0 and
/// the payload at elements `1..`; constant payloads are baked into the template
/// and dynamic ones are written after a `CloneEnum` with `SetFieldStruct`,
/// mirroring how a struct literal is built.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_enum_construction(
    enum_id: u16,
    variant_idx: u16,
    args: &[Expr],
    span: Span,
    args_indexes: &[Span],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let variant = &state.enums[enum_id as usize].variants[variant_idx as usize];
    let variant_name = variant.name.clone();
    let payload_types = variant.payload.clone();
    let arity = payload_types.len();

    compiler_errors::check_args(
        args,
        arity,
        &variant_name,
        span,
        state.sources,
        ctx.file_idx,
    );
    for (i, expected) in payload_types.iter().enumerate() {
        functions::check_arg_type(
            &variant_name,
            v,
            ctx,
            state,
            args,
            args_indexes,
            i,
            std::slice::from_ref(expected),
        );
    }

    let pool_idx = {
        state.pools.objs.push(Vec::with_capacity(arity + 1));
        state.pools.objs.len() - 1
    };
    state
        .pools
        .objs
        .get_mut(pool_idx)
        .push(Data::int(i32::from(variant_idx)));

    let mut dynamic: Vec<(u16, u16)> = Vec::with_capacity(arity);
    for (i, arg) in args.iter().enumerate() {
        let id = arg
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id();
        if arg.is_constant_literal() {
            let d = state.registers[id as usize];
            state.pools.objs.get_mut(pool_idx).push(d);
        } else {
            state.pools.objs.get_mut(pool_idx).push(NULL);
            dynamic.push((id, (i + 1) as u16));
        }
    }

    let template_reg = {
        state
            .registers
            .push(Data::enum_instance(enum_id, pool_idx as u32));
        (state.registers.len() - 1) as u16
    };
    let dest_reg = {
        state.registers.push(Data::enum_instance(enum_id, 0));
        (state.registers.len() - 1) as u16
    };
    output.push(Instr::CloneEnum(template_reg, dest_reg));
    for (val_reg, slot) in dynamic {
        output.push(Instr::SetFieldStruct(dest_reg, val_reg, slot));
    }
    dest_reg
}

/// Registers a nested `enum` declaration (one inside a function body). Top-level
/// enums are pre-registered by `parse_toplevel`; this mirrors
/// `compile_struct_definition` for the nested case.
fn compile_enum_definition(
    name: &SmolStr,
    variants: &[(SmolStr, Box<[TypeExpr]>, Span)],
    span: Span,
    type_params: &TypeParams,
    ctx: Ctx,
    state: &mut State<'_>,
) {
    if !type_params.is_empty() {
        state.generics.add_enum_template(
            name.clone(),
            type_params.clone(),
            ctx.file_idx,
            Box::from(variants),
        );
        return;
    }
    let enum_id = state.enums.len() as u16;
    state.enums.push(EnumType {
        name: name.clone(),
        variants: Box::from([]),
        id: enum_id,
        name_span: span,
    });
    state
        .namespace
        .symbols
        .push((name.clone(), SymbolKind::Enum(enum_id)));
    let resolved = variants
        .iter()
        .map(|(vn, payload, vspan)| EnumVariant {
            name: vn.clone(),
            payload: payload
                .iter()
                .map(|t| t.to_datatype(&mut state.type_ctx(ctx.file_idx)))
                .collect(),
            name_span: *vspan,
        })
        .collect();
    state.enums[enum_id as usize].variants = resolved;
}

/// Extracts a match arm's variant pattern: the variant index within `enum_id`
/// and the payload binder identifiers (`_` ignores a slot). Raises a compile
/// error for an unknown variant, a wrong-arity pattern, or a non-identifier
/// binder.
pub(crate) fn resolve_variant_pattern(
    enum_id: u16,
    pattern: &Expr,
    fallback_span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> (u16, Vec<SmolStr>) {
    let (variant_name, binders, span): (&SmolStr, Vec<SmolStr>, Span) = match pattern {
        Expr::Var(name, span) => (name, Vec::new(), *span),
        Expr::NamespacedRef(path, span, _) => (&path[path.len() - 1], Vec::new(), *span),
        Expr::FunctionCall(args, namespace, span, _, _) => {
            let mut binders = Vec::with_capacity(args.len());
            for arg in args {
                if let Expr::Var(binder, _) = arg {
                    binders.push(binder.clone());
                } else {
                    compiler_errors::error_enum(
                        "Invalid match pattern",
                        "Enum variant patterns may only bind identifiers, e.g. Circle(r)",
                        *span,
                        ctx.file_idx,
                        state.sources,
                    );
                }
            }
            (&namespace[namespace.len() - 1], binders, *span)
        }
        _ => compiler_errors::error_enum(
            "Invalid match pattern",
            "A match on an enum expects variant patterns, e.g. Circle(r) or Unit",
            fallback_span,
            ctx.file_idx,
            state.sources,
        ),
    };
    let e = &state.enums[enum_id as usize];
    let Some(variant_idx) = e.variants.iter().position(|vt| &vt.name == variant_name) else {
        compiler_errors::error_enum(
            "Unknown enum variant",
            &format!("{} is not a variant of enum {}", variant_name, e.name),
            span,
            ctx.file_idx,
            state.sources,
        );
    };
    let expected_arity = e.variants[variant_idx].payload.len();
    if binders.len() != expected_arity {
        compiler_errors::error_enum(
            "Wrong variant payload arity",
            &format!(
                "Variant {} binds {} value(s) but the pattern has {}",
                variant_name,
                expected_arity,
                binders.len()
            ),
            span,
            ctx.file_idx,
            state.sources,
        );
    }
    (variant_idx as u16, binders)
}

/// Lowers a `match` on an enum scrutinee to a variant-tag compare chain with
/// per-arm payload binding, reusing the ordinary conditional-jump machinery.
#[allow(clippy::too_many_arguments)]
fn compile_enum_match(
    enum_id: u16,
    scrutinee: &Expr,
    arms: &[(Expr, Box<[Expr]>)],
    wildcard: Option<&[Expr]>,
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let scrut_reg = scrutinee
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    // Root the scrutinee for the whole match so its object-pool payload is not
    // reclaimed and its register is not reused across arm bodies.
    let v_base = v.len();
    v.push(Variable {
        name: SmolStr::new_static("[MATCH SCRUT]"),
        register_id: scrut_reg,
        var_type: DataType::Enum(enum_id),
    });

    let tag_reg = state.alloc_reg();
    output.push(Instr::GetFieldStruct(scrut_reg, 0, tag_reg));

    let variant_count = state.enums[enum_id as usize].variants.len();
    let mut covered = vec![false; variant_count];
    let mut false_jmps: Vec<usize> = Vec::with_capacity(arms.len());
    let mut arm_starts: Vec<usize> = Vec::with_capacity(arms.len());
    let mut end_jmps: Vec<usize> = Vec::with_capacity(arms.len());

    for (pattern, body) in arms {
        let (variant_idx, binders) = resolve_variant_pattern(enum_id, pattern, span, ctx, state);
        covered[variant_idx as usize] = true;

        arm_starts.push(output.len());
        let idx_reg = state.alloc_reg();
        output.push(Instr::SetInt(idx_reg, i32::from(variant_idx)));
        false_jmps.push(output.len());
        output.push(Instr::NotEqJmp(tag_reg, idx_reg, 0));
        state.free_reg(idx_reg, v);

        // Bind the variant payload into fresh locals for the arm body.
        let v_arm = v.len();
        for (i, binder) in binders.iter().enumerate() {
            if binder.as_str() != "_" {
                let binder_reg = state.alloc_reg();
                output.push(Instr::GetFieldStruct(scrut_reg, (i + 1) as u16, binder_reg));
                let payload_type =
                    state.enums[enum_id as usize].variants[variant_idx as usize].payload[i].clone();
                v.push(Variable {
                    name: binder.clone(),
                    register_id: binder_reg,
                    var_type: payload_type,
                });
            }
        }

        let arm_code = compile_expr(body, v, ctx.advance_offset(output.len() as u16), state);
        output.extend(arm_code);
        v.truncate(v_arm);

        end_jmps.push(output.len());
        output.push(Instr::Jmp(0));
    }

    // Where a non-matching last arm (and the wildcard, if any) begins.
    let after_arms = output.len();
    if let Some(w) = wildcard {
        let wild_code = compile_expr(w, v, ctx.advance_offset(output.len() as u16), state);
        output.extend(wild_code);
    }
    let end = output.len();

    for (k, &j) in false_jmps.iter().enumerate() {
        let target = if k + 1 < arm_starts.len() {
            arm_starts[k + 1]
        } else {
            after_arms
        };
        set_jmp_size(&mut output[j], (target - j) as u16);
    }
    for &j in &end_jmps {
        set_jmp_size(&mut output[j], (end - j) as u16);
    }

    v.truncate(v_base);
    state.free_reg(tag_reg, v);
    state.free_reg(scrut_reg, v);

    if wildcard.is_none() && !covered.iter().all(|&c| c) {
        let missing: Vec<&str> = state.enums[enum_id as usize]
            .variants
            .iter()
            .enumerate()
            .filter(|(i, _)| !covered[*i])
            .map(|(_, vt)| vt.name.as_str())
            .collect();
        compiler_errors::error_enum(
            "Non-exhaustive match",
            &format!(
                "match on enum {} does not cover: {}. Add the missing arm(s) or a `_` wildcard",
                state.enums[enum_id as usize].name,
                missing.join(", ")
            ),
            span,
            ctx.file_idx,
            state.sources,
        );
    }
}

/// Compiles a `match`. An enum scrutinee dispatches to variant-pattern matching
/// with payload binding; any other scrutinee reproduces the equality-chain
/// lowering (`scrutinee == pattern` per arm) that `match` has always had.
fn compile_match(
    scrutinee: &Expr,
    arms: &[(Expr, Box<[Expr]>)],
    wildcard: Option<&[Expr]>,
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    if let DataType::Enum(enum_id) = scrutinee.infer_type(v, ctx, state) {
        compile_enum_match(
            enum_id, scrutinee, arms, wildcard, span, v, ctx, state, output,
        );
    } else {
        let obj_var = SmolStr::new_static("[MATCH TEMP]");
        let (first_pat, first_body) = &arms[0];
        let mut output_code: Vec<Expr> = Vec::with_capacity(arms.len());
        output_code.extend(first_body.iter().cloned());
        for (pat, body) in &arms[1..] {
            output_code.push(Expr::ElseIfBlock(
                Box::new(Expr::Eq(
                    Box::new(Expr::Var(obj_var.clone(), span)),
                    Box::new(pat.clone()),
                )),
                body.clone(),
            ));
        }
        if let Some(w) = wildcard {
            output_code.push(Expr::ElseBlock(Box::from(w)));
        }
        let desugared = Expr::EvalBlock(Box::from([
            Expr::VarDeclare(obj_var.clone(), Box::new(scrutinee.clone())),
            Expr::Condition(
                Box::new(Expr::Eq(
                    Box::new(Expr::Var(obj_var, span)),
                    Box::new(first_pat.clone()),
                )),
                Box::from(output_code),
                span,
            ),
        ]));
        desugared.compile(v, ctx, state, output, None, false, false);
    }
}

fn compile_map_literal(
    kv_pairs: &[(Expr, Span, Expr, Span)],
    map_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let mut global_key_type: DataType = DataType::Unknown;
    let mut global_val_type: DataType = DataType::Unknown;
    let map_id = state.pools.maps.len();
    state.pools.maps.push(HashMap::with_capacity_and_hasher(
        kv_pairs.len(),
        BuildHasherDefault::default(),
    ));
    if ctx.single_run {
        for (i, (key, key_span, val, val_span)) in kv_pairs.iter().enumerate() {
            if let Some((_, repeat_key_span, _, _)) =
                kv_pairs.iter().skip(i + 1).find(|(k, _, _, _)| k == key)
            {
                error_duplicate_map_key(
                    *key_span,
                    *repeat_key_span,
                    map_span,
                    ctx.file_idx,
                    state.sources,
                );
            }
            let key_t = key.infer_type(v, ctx, state);
            let val_t = val.infer_type(v, ctx, state);
            if i == 0 {
                global_key_type = key_t;
                global_val_type = val_t;
            } else {
                if key_t != global_key_type {
                    error_map_diff_types(
                        ctx.file_idx,
                        state.sources,
                        map_span,
                        &global_key_type,
                        *key_span,
                        &key_t,
                    )
                }
                if val_t != global_val_type {
                    error_map_diff_types(
                        ctx.file_idx,
                        state.sources,
                        map_span,
                        &global_val_type,
                        *val_span,
                        &val_t,
                    )
                }
            }
            let output_len = output.len();
            let key_val_id = key
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if !(key.is_constant_literal()
                || matches!(key, Expr::Array(_, _)) && output_len == output.len())
            {
                error_not_literal_map_key(*key_span, map_span, ctx.file_idx, state.sources);
            }
            let key_val = state.registers[key_val_id as usize];
            let id = val
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if val.is_constant_literal() {
                state.pools.maps[map_id].insert(key_val, state.registers[id as usize]);
            } else {
                state.pools.maps[map_id].insert(key_val, NULL);
                output.push(Instr::MapInsert(
                    map_id as u16,
                    state.registers.len() as u16,
                    id,
                ));
                state.registers.push(key_val);
            }
        }
        let dest_id = state.registers.len();
        state.registers.push(Data::map(map_id as u32));
        dest_id as u16
    } else {
        let mut dynamic: Vec<(Data, u16)> = Vec::with_capacity(kv_pairs.len());
        for (i, (key, key_span, val, val_span)) in kv_pairs.iter().enumerate() {
            if let Some((_, repeat_key_span, _, _)) =
                kv_pairs.iter().skip(i + 1).find(|(k, _, _, _)| k == key)
            {
                error_duplicate_map_key(
                    *key_span,
                    *repeat_key_span,
                    map_span,
                    ctx.file_idx,
                    state.sources,
                );
            }
            let key_t = key.infer_type(v, ctx, state);
            let val_t = val.infer_type(v, ctx, state);
            if i == 0 {
                global_key_type = key_t;
                global_val_type = val_t;
            } else {
                if key_t != global_key_type {
                    error_map_diff_types(
                        ctx.file_idx,
                        state.sources,
                        map_span,
                        &global_key_type,
                        *key_span,
                        &key_t,
                    )
                }
                if val_t != global_val_type {
                    error_map_diff_types(
                        ctx.file_idx,
                        state.sources,
                        map_span,
                        &global_val_type,
                        *val_span,
                        &val_t,
                    )
                }
            }
            let output_len = output.len();
            let key_val_id = key
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if !(key.is_constant_literal()
                || matches!(key, Expr::Array(_, _)) && output_len == output.len())
            {
                error_not_literal_map_key(*key_span, map_span, ctx.file_idx, state.sources);
            }
            let key_val = state.registers[key_val_id as usize];
            let val_id = val
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            if val.is_constant_literal() {
                state.pools.maps[map_id].insert(key_val, state.registers[val_id as usize]);
            } else {
                state.pools.maps[map_id].insert(key_val, NULL);
                dynamic.push((key_val, val_id));
            }
        }

        let template_reg = {
            state.registers.push(Data::map(map_id as u32));
            (state.registers.len() - 1) as u16
        };
        let dest_reg = {
            state.registers.push(Data::map(0));
            (state.registers.len() - 1) as u16
        };
        output.push(Instr::CloneMap(template_reg, dest_reg));
        for (key_val, val_id) in dynamic {
            let key_reg = if let Some(&id) = state.const_registers.get(&key_val) {
                id
            } else {
                let id = state.registers.len() as u16;
                state.const_registers.insert(key_val, id);
                state.registers.push(key_val);
                id
            };
            output.push(Instr::MapInsertReg(dest_reg, key_reg, val_id));
        }
        dest_reg
    }
}

fn compile_struct_field_access(
    struct_expr: &Expr,
    field: &SmolStr,
    struct_span: Span,
    field_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let t = struct_expr.infer_type(v, ctx, state);
    if let DataType::Struct(s_id) = t {
        let s = &state.structs[s_id as usize];
        let idx = s
            .fields
            .iter()
            .position(|f| &f.0 == field)
            .unwrap_or_else(|| {
                compiler_errors::error_struct_unknown_field(
                    ctx.file_idx,
                    field_span,
                    field,
                    &s.name,
                    &s.fields,
                    state.sources,
                );
            });
        let id = struct_expr
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id();
        let dest_reg_id = state.alloc_reg();
        output.push(Instr::GetFieldStruct(id, idx as u16, dest_reg_id));
        dest_reg_id
    } else {
        error_invalid_type(
            &DataType::Struct(0),
            &t,
            struct_span,
            None,
            None,
            ctx.file_idx,
            state.sources,
        );
    }
}

fn compile_array_indexing(
    array: &Expr,
    index: &Expr,
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let inferred = array.infer_type(v, ctx, state);
    if !inferred.is_indexable() {
        error_type_not_indexable(&inferred, span, false, ctx.file_idx, state.sources);
    }

    let id = array
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    let index_inferred = index.infer_type(v, ctx, state);
    if index_inferred != DataType::Int {
        error_invalid_index_type(&index_inferred, span, ctx.file_idx, state.sources);
    }
    let index_id = index
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(index_id, v);
    let dest_reg_id = state.alloc_reg();

    let to_push = if inferred == DataType::String {
        Instr::GetIndexString(id, index_id, dest_reg_id)
    } else {
        Instr::GetIndexArray(id, index_id, dest_reg_id)
    };
    output.push(to_push);
    state.add_to_src(ctx, output, span);
    dest_reg_id
}

fn compile_array_slice(
    array: &Expr,
    idx_start: &Expr,
    idx_end: &Expr,
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let inferred = array.infer_type(v, ctx, state);
    if !inferred.is_indexable() {
        error_type_not_indexable(&inferred, span, false, ctx.file_idx, state.sources);
    }
    let id = array
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let idx_start_inferred = idx_start.infer_type(v, ctx, state);
    if idx_start_inferred != DataType::Int {
        error_invalid_index_type(&idx_start_inferred, span, ctx.file_idx, state.sources);
    }
    let idx_start_id = idx_start
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let idx_end_inferred = idx_end.infer_type(v, ctx, state);
    if idx_end_inferred != DataType::Int {
        error_invalid_index_type(&idx_end_inferred, span, ctx.file_idx, state.sources);
    }
    let idx_end_id = idx_end
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    output.push(Instr::StoreFuncArg(idx_end_id));
    state.free_reg(idx_start_id, v);
    state.free_reg(idx_end_id, v);
    let dest_reg_id = state.alloc_reg();
    let to_push = if inferred == DataType::String {
        Instr::GetSliceString(id, idx_start_id, dest_reg_id)
    } else {
        Instr::GetSliceArray(id, idx_start_id, dest_reg_id)
    };
    output.push(to_push);
    state.add_to_src(ctx, output, span);
    dest_reg_id
}

#[inline]
fn uniform_op2(
    instr: fn(u16, u16, u16) -> Instr,
    t_1: &'static DataType,
    instr2: fn(u16, u16, u16) -> Instr,
    t_2: &'static DataType,
    symbol: &'static str,
    l: &Expr,
    r: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let (t_l, t_r) = (l.infer_type(v, ctx, state), r.infer_type(v, ctx, state));
    if !((&t_l == t_1 && &t_r == t_1) || (&t_l == t_2 && &t_r == t_2)) {
        compiler_errors::error_op(
            &t_l,
            &t_r,
            symbol,
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
        );
    }
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let id_r = r
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    state.free_reg(id_r, v);
    let id = state.alloc_reg_tgt(tgt_id);
    output.push(if &t_l == t_1 {
        instr(id_l, id_r, id)
    } else {
        instr2(id_l, id_r, id)
    });
    id
}

fn compile_div_op(
    l: &Expr,
    r: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    // A float left operand next to an `int` zero is a type error, and naming it
    // division by zero would report the wrong mistake. Every other left operand
    // makes this integer division, which does not divide by zero.
    if let Expr::Int(n) = r
        && *n == 0
        && l.infer_type(v, ctx, state) != DataType::Float
    {
        error_division_by_zero(false, span_l.extend(span_r), ctx.file_idx, state.sources);
    }
    let id = uniform_op2(
        Instr::DivFloat,
        &DataType::Float,
        Instr::DivInt,
        &DataType::Int,
        "/",
        l,
        r,
        span_l,
        span_r,
        tgt_id,
        v,
        ctx,
        state,
        output,
    );
    if matches!(output.last(), Some(Instr::DivInt(..))) {
        state.add_to_src(ctx, output, span_l.extend(span_r));
    }
    id
}

fn compile_add_op(
    l: &Expr,
    r: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let t_l = l.infer_type(v, ctx, state);
    let t_r = r.infer_type(v, ctx, state);
    if t_l != t_r
        || !matches!(
            t_l,
            DataType::String | DataType::Array(_) | DataType::Float | DataType::Int
        )
    {
        compiler_errors::error_op(&t_l, &t_r, "+", span_l, span_r, ctx.file_idx, state.sources);
    }
    // var+1 or 1+var use the dedicated IncInt/IncIntTo instructions
    if t_l == DataType::Int
        && let Some(Expr::Var(src_name, _)) = {
            if matches!(r, Expr::Int(1)) {
                Some(l)
            } else if matches!(l, Expr::Int(1)) {
                Some(r)
            } else {
                None
            }
        }
        && let Some(src_var) = v.iter().rfind(|x| x.name == *src_name)
    {
        let src_id = src_var.register_id;
        let id = tgt_id.unwrap_or_else(|| state.alloc_reg());
        output.push(if src_id == id {
            Instr::IncInt(id)
        } else {
            Instr::IncIntTo(src_id, id)
        });
        return id;
    }
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let id_r = r
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    state.free_reg(id_r, v);
    let id = state.alloc_reg_tgt(tgt_id);
    if matches!(t_l, DataType::Array(_)) {
        output.push(Instr::AddArray(id_l, id_r, id));
    } else if t_l == DataType::String {
        output.push(Instr::AddStr(id_l, id_r, id));
    } else if t_l == DataType::Float {
        output.push(Instr::AddFloat(id_l, id_r, id));
    } else {
        output.push(Instr::AddInt(id_l, id_r, id));
    }
    id
}

fn compile_sub_op(
    l: &Expr,
    r: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let t_l = l.infer_type(v, ctx, state);
    let t_r = r.infer_type(v, ctx, state);
    if !((t_l == DataType::Float && t_r == DataType::Float)
        || (t_l == DataType::Int && t_r == DataType::Int))
    {
        compiler_errors::error_op(&t_l, &t_r, "-", span_l, span_r, ctx.file_idx, state.sources);
    }
    // var-1 uses the dedicated DecInt/DecIntTo instructions
    if t_l == DataType::Int
        && matches!(r, Expr::Int(1))
        && let Expr::Var(src_name, _) = l
        && let Some(src_var) = v.iter().rfind(|x| x.name == *src_name)
    {
        let src_id = src_var.register_id;
        let id = tgt_id.unwrap_or_else(|| state.alloc_reg());
        output.push(if src_id == id {
            Instr::DecInt(id)
        } else {
            Instr::DecIntTo(src_id, id)
        });
        return id;
    }
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let id_r = r
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    state.free_reg(id_r, v);
    let id = state.alloc_reg_tgt(tgt_id);
    output.push(if t_l == DataType::Float {
        Instr::SubFloat(id_l, id_r, id)
    } else {
        Instr::SubInt(id_l, id_r, id)
    });
    id
}

fn compile_mod_op(
    l: &Expr,
    r: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    // As in `compile_div_op`: a float left operand makes this a type error
    // rather than a remainder by zero.
    if let Expr::Int(n) = r
        && *n == 0
        && l.infer_type(v, ctx, state) != DataType::Float
    {
        error_division_by_zero(true, span_l.extend(span_r), ctx.file_idx, state.sources);
    }
    let id = uniform_op2(
        Instr::ModFloat,
        &DataType::Float,
        Instr::ModInt,
        &DataType::Int,
        "%",
        l,
        r,
        span_l,
        span_r,
        tgt_id,
        v,
        ctx,
        state,
        output,
    );
    if matches!(output.last(), Some(Instr::ModInt(..))) {
        state.add_to_src(ctx, output, span_l.extend(span_r));
    }
    id
}

/// Compiles `&&` or `||` where a value is wanted rather than a branch.
///
/// The left operand is evaluated into the result register and, when it already
/// settles the answer, the jump skips the right operand entirely, so an
/// expression short-circuits wherever it appears and not only as the condition
/// of an `if` or a `while`.
///
/// The right operand is evaluated into its own register and moved, which keeps
/// the last instruction a `Mov`. An enclosing condition fuses the instruction it
/// finds at the end of a compiled condition into a jump, and fusing the right
/// operand's comparison would strand the short-circuit jump past it.
fn compile_short_circuit_value(
    l: &Expr,
    r: &Expr,
    span_l: Span,
    span_r: Span,
    symbol: &'static str,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let (t_l, t_r) = (l.infer_type(v, ctx, state), r.infer_type(v, ctx, state));
    if t_l != DataType::Bool || t_r != DataType::Bool {
        cold_path();
        compiler_errors::error_op(
            &t_l,
            &t_r,
            symbol,
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
        );
    }

    let id = state.alloc_reg_tgt(tgt_id);
    let left_id = l
        .compile(v, ctx, state, output, Some(id), false, true)
        .unwrap_id();
    if left_id != id {
        output.push(Instr::Mov(left_id, id));
    }

    let skip_idx = output.len();
    // `&&` is settled by a false left operand, `||` by a true one. Either way
    // the left operand's value is already in the result register.
    output.push(if symbol == "&&" {
        Instr::IsFalseJmp(id, 0)
    } else {
        Instr::IsTrueJmp(id, 0)
    });

    let right_id = r
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(right_id, v);
    output.push(Instr::Mov(right_id, id));
    let skip_size = (output.len() - skip_idx) as u16;
    set_jmp_size(&mut output[skip_idx], skip_size);
    id
}

fn compile_eq_op(
    l: &Expr,
    r: &Expr,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let l_type = l.infer_type(v, ctx, state);
    let r_type = r.infer_type(v, ctx, state);
    let is_array = matches!(
        l_type,
        DataType::Array(_) | DataType::Struct(_) | DataType::Enum(_)
    ) && matches!(
        r_type,
        DataType::Array(_) | DataType::Struct(_) | DataType::Enum(_)
    );
    let is_string = l_type == DataType::String || r_type == DataType::String;
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let id_r = r
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    state.free_reg(id_r, v);
    let id = state.alloc_reg_tgt(tgt_id);
    output.push(if is_array {
        Instr::ObjEq(id_l, id_r, id)
    } else if is_string {
        Instr::StrEq(id_l, id_r, id)
    } else {
        Instr::Eq(id_l, id_r, id)
    });
    id
}

fn compile_neq_op(
    l: &Expr,
    r: &Expr,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let l_type = l.infer_type(v, ctx, state);
    let r_type = r.infer_type(v, ctx, state);
    let is_array = matches!(
        l_type,
        DataType::Array(_) | DataType::Struct(_) | DataType::Enum(_)
    ) && matches!(
        r_type,
        DataType::Array(_) | DataType::Struct(_) | DataType::Enum(_)
    );
    let is_string = l_type == DataType::String || r_type == DataType::String;
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let id_r = r
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    state.free_reg(id_r, v);
    let id = state.alloc_reg_tgt(tgt_id);
    if is_array {
        output.push(Instr::ObjNotEq(id_l, id_r, id));
    } else if is_string {
        output.push(Instr::StrNotEq(id_l, id_r, id));
    } else {
        output.push(Instr::NotEq(id_l, id_r, id));
    }
    id
}

fn compile_neg_op(
    l: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let operand_type = l.infer_type(v, ctx, state);
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    let id = state.alloc_reg_tgt(tgt_id);
    if operand_type == DataType::Float {
        output.push(Instr::NegFloat(id_l, id));
    } else if operand_type == DataType::Int {
        output.push(Instr::NegInt(id_l, id));
    } else {
        compiler_errors::error_op(
            &DataType::Null,
            &operand_type,
            "-",
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
        );
    }
    id
}

fn compile_bool_neg_op(
    l: &Expr,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let operand_type = l.infer_type(v, ctx, state);
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    let id = state.alloc_reg_tgt(tgt_id);
    if operand_type != DataType::Bool {
        compiler_errors::error_op(
            &DataType::Null,
            &operand_type,
            "!",
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
        );
    }
    output.push(Instr::NegBool(id_l, id));
    id
}

fn compile_inline_condition_branch(
    branch: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
    tgt_id: u16,
) {
    let regs_before = state.registers.len() as u16;
    let output_len = output.len();
    output.extend(compile_expr(
        &branch[..branch.len() - 1],
        v,
        ctx.advance_offset(output.len() as u16),
        state,
    ));
    let val_id = branch[branch.len() - 1]
        .compile(
            v,
            ctx.advance_offset(output.len() as u16),
            state,
            output,
            Some(tgt_id),
            false,
            true,
        )
        .unwrap_id();
    state.free_scope_registers(regs_before, &output[output_len..], v);
    if val_id != tgt_id {
        output.push(Instr::Mov(val_id, tgt_id));
    }
}

fn compile_inline_condition(
    main_condition: &Expr,
    code: &[Expr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
    tgt_id: Option<u16>,
) -> u16 {
    let return_id = state.alloc_reg_tgt(tgt_id);

    // get first code limit (after which there are only else(if) blocks)
    let main_code_limit = code
        .iter()
        .position(|x| matches!(x, Expr::ElseIfBlock(_, _) | Expr::ElseBlock(_)))
        .unwrap_or(code.len());

    let condition_blocks_count = code.len() - main_code_limit;
    let mut cmp_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);
    let mut jmp_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);
    let mut condition_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);

    // parse the main condition
    let condition_id = main_condition
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    add_cmp_false(condition_id, &mut 0, output, false);
    cmp_markers.push(output.len() - 1);

    compile_inline_condition_branch(&code[..main_code_limit], v, ctx, state, output, return_id);
    if main_code_limit != code.len() {
        output.push(Instr::Jmp(0));
        jmp_markers.push(output.len() - 1);
    }

    let mut else_exists = false;
    for elem in &code[main_code_limit..] {
        if let Expr::ElseIfBlock(condition, code) = elem {
            condition_markers.push(output.len());
            let condition_id = condition
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            add_cmp_false(condition_id, &mut 0, output, false);
            state.free_reg(condition_id, v);
            cmp_markers.push(output.len() - 1);
            compile_inline_condition_branch(code, v, ctx, state, output, return_id);
            output.push(Instr::Jmp(0));
            jmp_markers.push(output.len() - 1);
        } else if let Expr::ElseBlock(code) = elem {
            else_exists = true;
            condition_markers.push(output.len());
            compile_inline_condition_branch(code, v, ctx, state, output, return_id);
        }
    }
    if !else_exists {
        error_conditional_expression_without_else(span, ctx.file_idx, state.sources);
    }

    for y in jmp_markers {
        let diff = output.len() - y;
        output[y] = Instr::Jmp(diff as u16);
    }
    for (i, y) in cmp_markers.iter().enumerate() {
        let diff = if i >= condition_markers.len() {
            output.len() - 1 - y
        } else {
            condition_markers[i] - y
        };
        if let Some(
            Instr::IsFalseJmp(_, jump_size)
            | Instr::SupEqFloatJmp(_, _, jump_size)
            | Instr::SupEqIntJmp(_, _, jump_size)
            | Instr::SupFloatJmp(_, _, jump_size)
            | Instr::SupIntJmp(_, _, jump_size)
            | Instr::InfEqFloatJmp(_, _, jump_size)
            | Instr::InfEqIntJmp(_, _, jump_size)
            | Instr::InfFloatJmp(_, _, jump_size)
            | Instr::InfIntJmp(_, _, jump_size)
            | Instr::NotEqJmp(_, _, jump_size)
            | Instr::ObjNotEqJmp(_, _, jump_size)
            | Instr::EqJmp(_, _, jump_size)
            | Instr::ObjEqJmp(_, _, jump_size),
        ) = output.get_mut(*y)
        {
            *jump_size = diff as u16;
        }
    }
    state.free_reg(condition_id, v);
    return_id
}

fn compile_array_index_assignment(
    array: &Expr,
    index: &Expr,
    value: &Expr,
    index_span: Span,
    elem_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let array_type = array.infer_type(v, ctx, state);
    if !array_type.is_indexable() {
        error_type_not_indexable(&array_type, index_span, false, ctx.file_idx, state.sources);
    }
    // Get the id of the source array/string (may be a nested GetIndex)
    let id = array
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    let final_id = index
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    let elem_type = value.infer_type(v, ctx, state);
    let elem_id = value
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(elem_id, v);
    if {
        if let DataType::Array(Some(array_type)) = &array_type
            && array_type.as_ref() != &elem_type
        {
            true
        } else {
            false
        }
    } || (array_type == DataType::String && elem_type != DataType::String)
    {
        error_cannot_push_type_to_array(
            &array_type,
            &elem_type,
            index_span,
            elem_span,
            ctx.file_idx,
            state.sources,
        );
    }

    let to_push = if array_type == DataType::String {
        Instr::SetElementString(id, elem_id, final_id)
    } else {
        Instr::SetElementObj(id, elem_id, final_id)
    };
    output.push(to_push);
    state.add_to_src(ctx, output, index_span);
    state.free_reg(id, v);
}

fn compile_struct_field_assignment(
    struct_expr: &Expr,
    field: &SmolStr,
    new_val: &Expr,
    struct_span: Span,
    field_span: Span,
    value_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let t = struct_expr.infer_type(v, ctx, state);
    let new_val_type = new_val.infer_type(v, ctx, state);
    let DataType::Struct(struct_id) = t else {
        error_invalid_type(
            &DataType::Struct(0),
            &t,
            struct_span,
            None,
            None,
            ctx.file_idx,
            state.sources,
        );
    };
    let mut field_index: Option<u16> = None;
    let field_struct = &state.structs[struct_id as usize];
    let struct_name = &field_struct.name;
    for (i, (expected_field_name, expected_field_type, expected_field_span)) in
        field_struct.fields.iter().enumerate()
    {
        if expected_field_name == field {
            if !struct_field_type_matches(expected_field_type, &new_val_type) {
                compiler_errors::error_struct_field_invalid_type(
                    ctx.file_idx,
                    struct_name,
                    *expected_field_span,
                    expected_field_name,
                    expected_field_type,
                    value_span,
                    &new_val_type,
                    state.sources,
                );
            }
            field_index = Some(i as u16);
            break;
        }
    }
    let Some(field_index) = field_index else {
        compiler_errors::error_struct_unknown_field(
            ctx.file_idx,
            field_span,
            field,
            struct_name,
            &field_struct.fields,
            state.sources,
        );
    };
    let id = struct_expr
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let new_elem_reg_id = new_val
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    output.push(Instr::SetFieldStruct(id, new_elem_reg_id, field_index));
}

fn compile_condition(
    main_condition: &Expr,
    code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    // get first code limit (after which there are only else(if) blocks)
    let main_code_limit = code
        .iter()
        .position(|x| matches!(x, Expr::ElseIfBlock(_, _) | Expr::ElseBlock(_)))
        .unwrap_or(code.len());

    let condition_blocks_count = code.len() - main_code_limit;
    // Each entry is the list of false-jump instruction indices for one condition block.
    let mut conditional_false_jmp_idxs: Vec<Vec<usize>> =
        Vec::with_capacity(condition_blocks_count + 1);
    let mut jmp_instr_idx: Vec<usize> = Vec::with_capacity(condition_blocks_count);
    let mut condition_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);

    // Compile the main condition
    let (true_jump_idxs, false_jump_idxs) =
        compile_short_circuit_condition(main_condition, v, ctx, state, output, false);
    conditional_false_jmp_idxs.push(false_jump_idxs);

    // Modify true jump instructions to point to body_start
    let body_start = output.len();
    for j in true_jump_idxs {
        set_jmp_size(&mut output[j], (body_start - j) as u16);
    }

    // parse the main code block
    let cond_code = compile_expr(
        &code[0..main_code_limit],
        v,
        ctx.advance_offset(output.len() as u16),
        state,
    );
    output.extend(cond_code);
    if main_code_limit != code.len() {
        output.push(Instr::Jmp(0));
        jmp_instr_idx.push(output.len() - 1);
    }

    for elem in &code[main_code_limit..] {
        if let Expr::ElseIfBlock(condition, code) = elem {
            condition_markers.push(output.len());
            let condition_id = condition
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(condition_id, v);
            add_cmp_false(condition_id, &mut 0, output, false);
            conditional_false_jmp_idxs.push(vec![output.len() - 1]);
            let cond_code = compile_expr(code, v, ctx.advance_offset(output.len() as u16), state);
            output.extend(cond_code);
            output.push(Instr::Jmp(0));
            jmp_instr_idx.push(output.len() - 1);
        } else if let Expr::ElseBlock(code) = elem {
            condition_markers.push(output.len());
            let cond_code = compile_expr(code, v, ctx.advance_offset(output.len() as u16), state);
            output.extend(cond_code);
        }
    }

    for y in jmp_instr_idx {
        let diff = output.len() - y;
        output[y] = Instr::Jmp(diff as u16);
    }
    // Fix all false-jump instructions for each condition block
    for (cm_idx, false_idxs) in conditional_false_jmp_idxs.iter().enumerate() {
        let target = if cm_idx < condition_markers.len() {
            condition_markers[cm_idx]
        } else {
            output.len()
        };
        for &y in false_idxs {
            set_jmp_size(&mut output[y], (target - y) as u16);
        }
    }
}

fn compile_while_loop(
    condition: &Expr,
    code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let output_len_before = output.len();

    let (true_jump_idxs, false_jump_idxs) =
        compile_short_circuit_condition(condition, v, ctx, state, output, false);

    let body_start = output.len();
    for j in true_jump_idxs {
        set_jmp_size(&mut output[j], (body_start - j) as u16);
    }

    // parse the code block, clone the vars to avoid overriding anything
    let loop_id = ctx.block_id + 1;

    let mut cond_code = compile_expr(
        code,
        v,
        ctx.no_single_run().advance_offset(output.len() as u16),
        state,
    );

    let exit = output.len() + cond_code.len() + 1;
    for j in false_jump_idxs {
        set_jmp_size(&mut output[j], (exit - j) as u16);
    }

    let cond_len = (output.len() - output_len_before) as u16;
    let body_len = cond_code.len() as u16;
    let len = cond_len + body_len; // full span used by JmpBack
    // Break/Continue offsets are relative to cond_code, so pass body_len+1 (body remaining + JmpBack)
    parse_loop_flow_control(&mut cond_code, loop_id, body_len + 1, false, false);
    output.extend(cond_code);
    output.push(Instr::JmpBack(len));
}

fn compile_for_loop(
    var_name: &SmolStr,
    array: &Expr,
    code: &[Expr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let real_var = var_name.as_str() != "_";

    // parse the array, get its id (the target array is the first Expr in array_code)
    let array_type = array.infer_type(v, ctx, state);
    let mut array = array
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    // Iterating a map walks its keys: materialize the key array and iterate that
    // with the ordinary array machinery. The loop variable binds each key.
    if matches!(array_type, DataType::Map(_)) {
        let keys_reg = state.alloc_reg();
        output.push(Instr::CallLibFunc(LibFunc::Keys, array, keys_reg));
        array = keys_reg;
    }

    let array_len_id = state.alloc_reg();

    output.push(Instr::CallLibFunc(LibFunc::Len, array, array_len_id));

    // set up the id of the index variable (0..len)
    let index_id = if ctx.single_run {
        state.registers.push(0.into());
        (state.registers.len() - 1) as u16
    } else {
        let id = state.alloc_reg();
        output.push(Instr::SetInt(id, 0));
        id
    };

    // do the 'i < len' condition, set up the condition's id (true/false)
    let condition_id = state.alloc_reg();

    output.push(Instr::InfInt(index_id, array_len_id, condition_id));

    // set up the variable for the current element (for current_element_id in ... {}) => current_element_id = array[index]
    let current_element_id = if real_var { state.alloc_reg() } else { 0 };

    let v_len = v.len();

    let is_str = array_type == DataType::String;

    if real_var {
        v.push(Variable {
            name: var_name.clone(),
            register_id: current_element_id,
            var_type: match array_type {
                DataType::String => DataType::String,
                DataType::Array(a_type) => a_type.map_or(DataType::Null, |t| *t),
                // A map iterates its keys; the loop variable is a key.
                DataType::Map(m) => m.0.unwrap_or(DataType::Unknown),
                t => {
                    error_type_not_indexable(&t, span, true, ctx.file_idx, state.sources);
                }
            },
        });
    }
    let loop_id = ctx.block_id + 1;

    // accounts for the GetIndexArray/GetIndexString instruction
    let pending = real_var as u16;

    let regs_before = state.registers.len() as u16;
    let mut cond_code = compile_expr(
        code,
        v,
        ctx.no_single_run()
            .advance_offset(output.len() as u16 + pending),
        state,
    );
    // Clean up variables
    v.truncate(v_len);
    state.free_loop_scope_registers(regs_before, &cond_code, v);

    // add the condition ('i < len') jumping logic
    let mut len = (cond_code.len() + 3) as u16 + pending;
    add_cmp_false(condition_id, &mut len, output, true);

    // load the element's value into the current_element_id register
    if real_var {
        if is_str {
            output.push(Instr::GetIndexString(array, index_id, current_element_id));
        } else {
            output.push(Instr::GetIndexArray(array, index_id, current_element_id));
        }
    }
    parse_loop_flow_control(&mut cond_code, loop_id, len, true, false);
    // then add the condition code
    output.extend(cond_code);
    // add 1 to the index (i+=1) so that the next loop iteration will have the next element in the array
    output.push(Instr::IncInt(index_id));

    // jump back to the loop if still inside of it
    output.push(Instr::JmpBack(len));

    if ctx.single_run {
        state.free_reg(array_len_id, v);
        state.free_reg(index_id, v);
        state.free_reg(condition_id, v);
        if real_var {
            state.free_reg(current_element_id, v);
        }
    }
}

fn compile_int_for_loop(
    var_name: &SmolStr,
    start_elem: &Expr,
    end_elem: &Expr,
    code: &[Expr],
    span1: Span,
    span2: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    // IntForLoop is compiled to:
    // ----
    // (1) if i >= end_elem jump out
    // (2) loop_body
    // (3) i += 1
    // (4) if i < end_elem jump back to body
    // ----
    //
    //
    // Check start and elem type
    let t1 = start_elem.infer_type(v, ctx, state);
    let t2 = end_elem.infer_type(v, ctx, state);
    if t1 != DataType::Int {
        error_range_invalid_type(span1, &t1, ctx.file_idx, state.sources);
    }
    if t2 != DataType::Int {
        error_range_invalid_type(span2, &t2, ctx.file_idx, state.sources);
    }
    let elem_id = if ctx.single_run {
        start_elem
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id()
    } else {
        let start_elem_id = start_elem
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id();
        let start_val = state.registers[start_elem_id as usize];
        let elem_id = state.alloc_reg();
        if state.const_registers.values().any(|&v| v == start_elem_id) && start_val.is_int() {
            output.push(Instr::SetInt(elem_id, start_val.as_int()));
        } else {
            output.push(Instr::Mov(start_elem_id, elem_id));
        }
        elem_id
    };
    let end_elem_id = end_elem
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    // elem_id is a fresh mutable register -> remove from const_registers just in case
    state.const_registers.retain(|_, &mut v| v != elem_id);

    let v_len = v.len();
    v.push(Variable {
        name: var_name.clone(),
        register_id: elem_id,
        var_type: DataType::Int,
    });
    let loop_id = ctx.block_id + 1;

    // (1) if i >= end_elem jump out -> push placeholder first so that compile_expr sees the correct offset
    let jmp_idx = output.len();
    output.push(Instr::SupEqIntJmp(elem_id, end_elem_id, 0));

    let regs_before = state.registers.len() as u16;
    let compiled_loop_code = compile_expr(
        code,
        v,
        ctx.no_single_run().advance_offset(output.len() as u16),
        state,
    );
    state.free_loop_scope_registers(regs_before, &compiled_loop_code, v);
    let compiled_loop_code_len = compiled_loop_code.len() as u16;

    // (2) loop_body
    output.extend(compiled_loop_code);

    // (3) i+= 1
    output.push(Instr::IncInt(elem_id));

    // (4) if i < end_elem jump back to body
    output.push(Instr::InfIntJmpBack(
        elem_id,
        end_elem_id,
        compiled_loop_code_len + 1,
    ));

    let exit_size = (output.len() - jmp_idx) as u16;
    output[jmp_idx] = Instr::SupEqIntJmp(elem_id, end_elem_id, exit_size);

    parse_loop_flow_control(&mut output[jmp_idx + 1..], loop_id, exit_size, true, false);
    v.truncate(v_len);

    if ctx.single_run {
        state.free_reg(end_elem_id, v);
        state.free_reg(elem_id, v);
    }
}

fn compile_loop_block(
    code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let loop_id = ctx.block_id + 1;
    let regs_before = state.registers.len() as u16;
    let mut compiled = compile_expr(
        code,
        v,
        ctx.no_single_run().advance_offset(output.len() as u16),
        state,
    );
    state.free_loop_scope_registers(regs_before, &compiled, v);
    let code_length = compiled.len() as u16;
    parse_loop_flow_control(&mut compiled, loop_id, code_length + 1, false, true);
    output.extend(compiled);
    output.push(Instr::JmpBack(code_length));
}

fn compile_try_catch_block(
    e: &[Expr],
    err_var: &SmolStr,
    catch_code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    output.push(Instr::StartErrorCatch(0, 0)); // patched later on
    let err_catch_instr = output.len() - 1;
    // A function body is compiled inline at its first call site and records its
    // absolute entry address as `ctx.offset + output.len()`, so both blocks are
    // compiled at the offset they occupy. Compiling them at the
    // enclosing offset makes every call inside a `try` jump short by the length
    // of the code already emitted before it.
    let main_code = compile_expr(e, v, ctx.advance_offset(output.len() as u16), state);
    output.extend(main_code);
    output.push(Instr::StopErrorCatch);
    output.push(Instr::Jmp(0)); // jumps over the catch handler if no error arises
    let jmp_catch_instr = output.len() - 1;

    let v_len = v.len();
    let err_reg_id = state.alloc_reg();
    v.push(Variable {
        name: err_var.clone(),
        register_id: err_reg_id,
        var_type: DataType::String,
    });
    output[err_catch_instr] =
        Instr::StartErrorCatch((output.len() - err_catch_instr) as u16, err_reg_id);
    let catch_code = compile_expr(
        catch_code,
        v,
        ctx.advance_offset(output.len() as u16),
        state,
    );
    v.truncate(v_len);
    output.extend(catch_code);
    output[jmp_catch_instr] = Instr::Jmp((output.len() - jmp_catch_instr) as u16);
    state.free_reg(err_reg_id, v);
}

fn compile_var_declaration(
    name: &SmolStr,
    value: &Expr,
    remaining_code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let var_type = value.infer_type(v, ctx, state);

    let var_id = if ctx.single_run {
        value
            .compile(v, ctx, state, output, None, true, true)
            .unwrap_id()
    } else {
        let src_id = value
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id();
        if code_modifies_variable(name, remaining_code) {
            let mutable_id = state.alloc_reg();
            move_reg_to_reg(output, src_id, mutable_id, state.registers[src_id as usize]);
            mutable_id
        } else {
            src_id
        }
    };

    if let DataType::Fn(fn_id) = &var_type {
        state
            .namespace
            .symbols
            .push((name.clone(), SymbolKind::Fn(*fn_id)));
    }
    v.push(Variable {
        name: name.clone(),
        register_id: var_id,
        var_type,
    });
}

fn compile_var_assignment(
    name: &SmolStr,
    value: &Expr,
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let var_type = value.infer_type(v, ctx, state);
    let var_pos = v.iter().rposition(|x| x.name == *name).unwrap_or_else(|| {
        compiler_errors::error_unknown_variable(name, span, v, ctx.file_idx, state.sources);
    });
    let id = v[var_pos].register_id;

    if var_type == DataType::Int {
        // (is_inc, src_var_name)
        let inc_dec: Option<(bool, &str)> = match value {
            // var+1/1+var use the dedicated IncInt/IncIntTo instructions
            Expr::Add(l, r, _, _) => {
                let src = if matches!(r.as_ref(), Expr::Int(1)) {
                    Some(l.as_ref())
                } else if matches!(l.as_ref(), Expr::Int(1)) {
                    Some(r.as_ref())
                } else {
                    None
                };
                src.and_then(|e| {
                    if let Expr::Var(src_name, _) = e {
                        v.iter()
                            .rfind(|x| x.name == *src_name)
                            .filter(|x| x.var_type == DataType::Int)
                            .map(|_| (true, src_name.as_str()))
                    } else {
                        None
                    }
                })
            }
            // var-1 uses the dedicated DecInt/DecIntTo instructions
            Expr::Sub(l, r, _, _) => {
                if matches!(r.as_ref(), Expr::Int(1)) {
                    if let Expr::Var(src_name, _) = l.as_ref() {
                        v.iter()
                            .rfind(|x| x.name == *src_name)
                            .filter(|x| x.var_type == DataType::Int)
                            .map(|_| (false, src_name.as_str()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((is_inc, src_name)) = inc_dec {
            let src_id = v.iter().rfind(|x| x.name == src_name).unwrap().register_id;
            output.push(if src_id == id {
                if is_inc {
                    Instr::IncInt(id)
                } else {
                    Instr::DecInt(id)
                }
            } else {
                if is_inc {
                    Instr::IncIntTo(src_id, id)
                } else {
                    Instr::DecIntTo(src_id, id)
                }
            });
            return;
        }
    }

    let output_len = output.len();
    let obj_id = value
        .compile(v, ctx, state, output, Some(id), false, true)
        .unwrap_id();
    if output.len() != output_len {
        if !move_to_id(output, id) {
            output.push(Instr::Mov(obj_id, id));
        }
    } else if state.const_registers.values().any(|&v| v == obj_id) {
        move_reg_to_reg(output, obj_id, id, state.registers[obj_id as usize]);
    } else {
        output.push(Instr::Mov(obj_id, id));
    }
    if !v
        .iter()
        .any(|var| &var.name != name && var.register_id == obj_id)
    {
        state.free_reg(obj_id, v);
    }
    v[var_pos].var_type = var_type;
}

fn compile_struct_definition(
    name: &SmolStr,
    fields: &[(SmolStr, TypeExpr, Span)],
    span: Span,
    type_params: &TypeParams,
    ctx: Ctx,
    state: &mut State<'_>,
    _output: &mut Vec<Instr>,
) {
    if !type_params.is_empty() {
        state.generics.add_struct_template(
            name.clone(),
            type_params.clone(),
            ctx.file_idx,
            Box::from(fields),
        );
        return;
    }
    let struct_id = state.structs.len() as u16;
    state.structs.push(Struct {
        // pushing it first allows structs to be recursive
        name: name.clone(),
        fields: Box::from([]),
        id: struct_id,
        name_span: span,
    });
    state.namespace.symbols.push((
        name.clone(),
        SymbolKind::Struct((state.structs.len() - 1) as u16),
    ));
    let parsed_fields = fields
        .iter()
        .map(|(f, f_t, f_span)| {
            (
                f.clone(),
                f_t.to_datatype(&mut state.type_ctx(ctx.file_idx)),
                *f_span,
            )
        })
        .collect();
    state.structs[struct_id as usize].fields = parsed_fields;
}

fn compile_function_definition(
    fn_name: &SmolStr,
    fn_args: &[(SmolStr, Option<TypeExpr>)],
    fn_code: &Rc<[Expr]>,
    span: Span,
    declared_return_type: Option<&(TypeExpr, Span)>,
    type_params: &TypeParams,
    _v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    _output: &mut Vec<Instr>,
) {
    if let Some(func) = state.fns.iter().find(|func| &func.name == fn_name) {
        compiler_errors::error_function_already_defined(func, span, ctx.file_idx, state.sources);
    }
    let mut callees = Vec::new();
    collect_direct_fn_calls(fn_code, &mut callees);
    state
        .namespace
        .symbols
        .push((fn_name.clone(), SymbolKind::Fn(state.fns.len() as u16)));
    // An annotation naming one of this function's own type parameters resolves
    // per call site, so it is left un-pinned here.
    let args: Box<[(SmolStr, Option<DataType>)]> = fn_args
        .iter()
        .map(|(a, t)| {
            (
                a.clone(),
                t.clone()
                    .filter(|t_e| !t_e.mentions_any(type_params))
                    .map(|t_e| t_e.to_datatype(&mut state.type_ctx(ctx.file_idx))),
            )
        })
        .collect();
    let return_type = declared_return_type
        .filter(|(t_e, _)| !t_e.mentions_any(type_params))
        .map(|(t_e, t_span)| (t_e.to_datatype(&mut state.type_ctx(ctx.file_idx)), *t_span));
    let generics = (!type_params.is_empty()).then(|| {
        Box::new(FnGenerics {
            params: type_params.clone(),
            arg_types: fn_args.iter().map(|(_, t)| t.clone()).collect(),
            return_type: declared_return_type.map(|t| Box::new(t.clone())),
            bindings: Box::from([]),
            file_idx: ctx.file_idx,
        })
    });
    state.fns.push(Function {
        name: fn_name.clone(),
        args,
        code: fn_code.clone(),
        impls: Vec::new(),
        is_recursive: None,
        returns_null: check_if_returns_void(fn_code),
        src_file: ctx.file_idx,
        return_type_cache: Vec::new(),
        direct_calls: callees.into_boxed_slice(),
        name_span: span,
        return_type,
        generics,
    });
    state.fn_registers.push(Vec::new());
}

fn compile_return(
    return_value: Option<&Expr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    // `main` is compiled inline at the program's top level, so a `return` there
    // has no call frame to pop. It ends the program, and its value, like the
    // value of `main` itself, has nowhere to go.
    if !ctx.in_function {
        if let Some(x) = return_value {
            let id = x
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
        }
        output.push(Instr::Halt(0));
        return;
    }
    let Some(x) = return_value else {
        // `return;` leaves the function with no value. A body always ends in a
        // `VoidReturn`, but an early return needs one of its own, or control
        // falls through into the statements that follow it.
        output.push(Instr::VoidReturn);
        return;
    };
    let id = x
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    if ctx.is_compiling_recursive {
        output.push(Instr::RecursiveReturn(id));
    } else {
        output.push(Instr::Return(id));
    }
}

#[inline]
fn compile_loop_break(ctx: Ctx, output: &mut Vec<Instr>) {
    output.push(Instr::NotEqJmp(ctx.block_id + 1, 0, 0));
}

#[inline]
fn compile_loop_continue(ctx: Ctx, output: &mut Vec<Instr>) {
    output.push(Instr::EqJmp(ctx.block_id + 1, 0, 0));
}

#[inline]
fn compile_eval_block(
    code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    output.extend(compile_expr(
        code,
        v,
        ctx.set_offset(output.len() as u16),
        state,
    ));
}

pub fn compile_expr(
    input: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<Instr> {
    let v_len = v.len();
    let fn_len = state.fns.len();
    let symbols_len = state.namespace.symbols.len();
    let mut output: Vec<Instr> = Vec::with_capacity(input.len());
    for (idx, x) in input.iter().enumerate() {
        if let Some(id) = x.compile_with_code_context(
            v,
            ctx,
            state,
            &mut output,
            None,
            false,
            &input[idx + 1..],
            false,
        ) {
            state.free_reg(id, v);
        }
    }
    v.truncate(v_len);
    state.fns.truncate(fn_len);
    state.namespace.symbols.truncate(symbols_len);
    output
}

impl Expr {
    #[must_use]
    pub const fn is_constant_literal(&self) -> bool {
        matches!(
            self,
            Self::Int(_) | Self::Float(_) | Self::String(_) | Self::Bool(_) | Self::Null
        )
    }
    #[inline(always)]
    pub fn compile(
        &self,
        v: &mut Vec<Variable>,
        ctx: Ctx,
        state: &mut State<'_>,
        output: &mut Vec<Instr>,
        tgt_id: Option<u16>,
        var_assignment: bool,
        uses_id: bool,
    ) -> Option<u16> {
        self.compile_with_code_context(v, ctx, state, output, tgt_id, var_assignment, &[], uses_id)
    }
    pub fn compile_with_code_context(
        &self,
        v: &mut Vec<Variable>,
        ctx: Ctx,
        state: &mut State<'_>,
        output: &mut Vec<Instr>,
        tgt_id: Option<u16>,
        var_assignment: bool,
        remaining_code: &[Self],
        uses_id: bool,
    ) -> Option<u16> {
        match self {
            Self::Int(num) => {
                debug_assert!(uses_id);
                if var_assignment {
                    state.registers.push((*num).into());
                    return Some((state.registers.len() - 1) as u16);
                }
                let data = (*num).into();
                if let Some(&id) = state.const_registers.get(&data) {
                    Some(id)
                } else {
                    let id = state.registers.len() as u16;
                    state.const_registers.insert(data, id);
                    state.registers.push(data);
                    Some(id)
                }
            }
            Self::Float(num) => {
                debug_assert!(uses_id);
                if var_assignment {
                    state.registers.push((*num).into());
                    return Some((state.registers.len() - 1) as u16);
                }
                let data = (*num).into();
                if let Some(&id) = state.const_registers.get(&data) {
                    Some(id)
                } else {
                    state.registers.push(data);
                    let id = (state.registers.len() - 1) as u16;
                    state.const_registers.insert(data, id);
                    Some(id)
                }
            }
            Self::String(str) => {
                debug_assert!(uses_id);
                if var_assignment {
                    state
                        .registers
                        .push(Data::p_str(str, &mut state.pools.strings));
                    return Some((state.registers.len() - 1) as u16);
                }
                let data = Data::p_str(str, &mut state.pools.strings);
                if let Some(&id) = state.const_registers.get(&data) {
                    Some(id)
                } else {
                    let id = state.registers.len() as u16;
                    state.const_registers.insert(data, id);
                    state.registers.push(data);
                    Some(id)
                }
            }
            Self::Null => {
                debug_assert!(uses_id);
                if var_assignment {
                    state.registers.push(NULL);
                    return Some((state.registers.len() - 1) as u16);
                }
                if let Some(&id) = state.const_registers.get(&NULL) {
                    Some(id)
                } else {
                    let id = state.registers.len() as u16;
                    state.const_registers.insert(NULL, id);
                    state.registers.push(NULL);
                    Some(id)
                }
            }
            Self::Bool(bool) => {
                debug_assert!(uses_id);
                if var_assignment {
                    state.registers.push((*bool).into());
                    return Some((state.registers.len() - 1) as u16);
                }
                let data: Data = (*bool).into();
                if let Some(&id) = state.const_registers.get(&data) {
                    Some(id)
                } else {
                    let id = state.registers.len() as u16;
                    state.const_registers.insert(data, id);
                    state.registers.push(data);
                    Some(id)
                }
            }
            Self::Var(name, span) => {
                debug_assert!(uses_id);
                if let Some(Variable {
                    name: _,
                    register_id,
                    var_type: _,
                }) = v.iter().rfind(|v_temp| *name == v_temp.name)
                {
                    Some(*register_id)
                } else if let Some((enum_id, variant_idx)) =
                    resolve_enum_variant(std::slice::from_ref(name), state)
                {
                    Some(compile_enum_construction(
                        enum_id,
                        variant_idx,
                        &[],
                        *span,
                        &[],
                        v,
                        ctx,
                        state,
                        output,
                    ))
                } else {
                    compiler_errors::error_unknown_variable(
                        name,
                        *span,
                        v,
                        ctx.file_idx,
                        state.sources,
                    );
                }
            }
            Self::Array(array_items, spans) => {
                debug_assert!(uses_id);
                Some(compile_array_literal(
                    array_items,
                    spans,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::Struct(namespace, fields, span, type_args) => {
                debug_assert!(uses_id);
                Some(compile_struct_literal(
                    namespace, fields, type_args, *span, v, ctx, state, output,
                ))
            }
            Self::Map(kv_pairs, span) => {
                debug_assert!(uses_id);
                Some(compile_map_literal(kv_pairs, *span, v, ctx, state, output))
            }
            Self::GetStructField(struct_expr, field, struct_span, field_span) => {
                debug_assert!(uses_id);
                Some(compile_struct_field_access(
                    struct_expr,
                    field,
                    *struct_span,
                    *field_span,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            // array[index]
            Self::ArrayGetIndex(array, index, span) => {
                debug_assert!(uses_id);
                Some(compile_array_indexing(
                    array, index, *span, v, ctx, state, output,
                ))
            }
            // array[start..end]
            Self::ArrayGetSlice(array, idx_start, idx_end, span) => {
                debug_assert!(uses_id);
                Some(compile_array_slice(
                    array, idx_start, idx_end, *span, v, ctx, state, output,
                ))
            }
            Self::Mul(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(uniform_op2(
                    Instr::MulFloat,
                    &DataType::Float,
                    Instr::MulInt,
                    &DataType::Int,
                    "*",
                    l,
                    r,
                    *span1,
                    *span2,
                    tgt_id,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::Div(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_div_op(
                    l, r, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::Add(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_add_op(
                    l, r, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::Sub(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_sub_op(
                    l, r, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::Mod(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_mod_op(
                    l, r, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::Pow(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(uniform_op2(
                    Instr::PowFloat,
                    &DataType::Float,
                    Instr::PowInt,
                    &DataType::Int,
                    "^",
                    l,
                    r,
                    *span1,
                    *span2,
                    tgt_id,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::Eq(l, r) => {
                debug_assert!(uses_id);
                Some(compile_eq_op(l, r, tgt_id, v, ctx, state, output))
            }
            Self::NotEq(l, r) => {
                debug_assert!(uses_id);
                Some(compile_neq_op(l, r, tgt_id, v, ctx, state, output))
            }
            Self::Sup(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(uniform_op2(
                    Instr::SupFloat,
                    &DataType::Float,
                    Instr::SupInt,
                    &DataType::Int,
                    ">",
                    l,
                    r,
                    *span1,
                    *span2,
                    tgt_id,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::SupEq(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(uniform_op2(
                    Instr::SupEqFloat,
                    &DataType::Float,
                    Instr::SupEqInt,
                    &DataType::Int,
                    ">=",
                    l,
                    r,
                    *span1,
                    *span2,
                    tgt_id,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::Inf(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(uniform_op2(
                    Instr::InfFloat,
                    &DataType::Float,
                    Instr::InfInt,
                    &DataType::Int,
                    "<",
                    l,
                    r,
                    *span1,
                    *span2,
                    tgt_id,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::InfEq(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(uniform_op2(
                    Instr::InfEqFloat,
                    &DataType::Float,
                    Instr::InfEqInt,
                    &DataType::Int,
                    "<=",
                    l,
                    r,
                    *span1,
                    *span2,
                    tgt_id,
                    v,
                    ctx,
                    state,
                    output,
                ))
            }
            Self::BoolAnd(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_short_circuit_value(
                    l, r, *span1, *span2, "&&", tgt_id, v, ctx, state, output,
                ))
            }
            Self::BoolOr(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_short_circuit_value(
                    l, r, *span1, *span2, "||", tgt_id, v, ctx, state, output,
                ))
            }
            Self::Neg(l, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_neg_op(
                    l, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::BoolNeg(l, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_bool_neg_op(
                    l, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::InlineCondition(main_condition, code, span) => {
                debug_assert!(uses_id);
                Some(compile_inline_condition(
                    main_condition,
                    code,
                    *span,
                    v,
                    ctx,
                    state,
                    output,
                    tgt_id,
                ))
            }
            Self::FunctionCall(args, namespace, markers, args_indexes, type_args) if uses_id => {
                Some(
                    handle_functions(
                        output,
                        v,
                        ctx,
                        state,
                        tgt_id,
                        args,
                        namespace,
                        *markers,
                        args_indexes,
                        type_args,
                    )
                    .unwrap_or_else(|| {
                        if let Some(&id) = state.const_registers.get(&NULL) {
                            id
                        } else {
                            let id = state.registers.len() as u16;
                            state.const_registers.insert(NULL, id);
                            state.registers.push(NULL);
                            id
                        }
                    }),
                )
            }
            Self::AnonymousFunction(_, _, _) => {
                debug_assert!(uses_id);
                if let Some(&id) = state.const_registers.get(&NULL) {
                    Some(id)
                } else {
                    let id = state.registers.len() as u16;
                    state.const_registers.insert(NULL, id);
                    state.registers.push(NULL);
                    Some(id)
                }
            }

            // ------------------
            // --- STATEMENTS ---
            // ------------------

            // x[y] = z;
            Self::ArrayModify(array, index, value, index_markers, elem_markers) => {
                debug_assert!(!uses_id);
                compile_array_index_assignment(
                    array,
                    index,
                    value,
                    *index_markers,
                    *elem_markers,
                    v,
                    ctx,
                    state,
                    output,
                );
                None
            }
            Self::SetStructField(
                struct_expr,
                field,
                new_val,
                struct_span,
                field_span,
                value_span,
            ) => {
                debug_assert!(!uses_id);
                compile_struct_field_assignment(
                    struct_expr,
                    field,
                    new_val,
                    *struct_span,
                    *field_span,
                    *value_span,
                    v,
                    ctx,
                    state,
                    output,
                );
                None
            }
            Self::Condition(main_condition, code, _) => {
                debug_assert!(!uses_id);
                compile_condition(main_condition, code, v, ctx, state, output);
                None
            }
            Self::WhileBlock(condition, code) => {
                debug_assert!(!uses_id);
                compile_while_loop(condition, code, v, ctx, state, output);
                None
            }
            Self::ForLoop(var_name, array, code, span) => {
                debug_assert!(!uses_id);
                compile_for_loop(var_name, array, code, *span, v, ctx, state, output);
                None
            }
            Self::IntForLoop(var_name, start_elem, end_elem, code, span1, span2) => {
                debug_assert!(!uses_id);
                compile_int_for_loop(
                    var_name, start_elem, end_elem, code, *span1, *span2, v, ctx, state, output,
                );
                None
            }
            Self::LoopBlock(code) => {
                debug_assert!(!uses_id);
                compile_loop_block(code, v, ctx, state, output);
                None
            }
            Self::TryCatchBlock(e, err_var, catch_code) => {
                debug_assert!(!uses_id);
                compile_try_catch_block(e, err_var, catch_code, v, ctx, state, output);
                None
            }
            Self::VarDeclare(name, value) => {
                debug_assert!(!uses_id);
                compile_var_declaration(name, value, remaining_code, v, ctx, state, output);
                None
            }
            Self::VarAssign(name, value, span) => {
                debug_assert!(!uses_id);
                compile_var_assignment(name, value, *span, v, ctx, state, output);
                None
            }
            Self::StructDeclare(name, fields, span, type_params) => {
                debug_assert!(!uses_id);
                compile_struct_definition(name, fields, *span, type_params, ctx, state, output);
                None
            }
            Self::EnumDeclare(name, variants, span, type_params) => {
                debug_assert!(!uses_id);
                compile_enum_definition(name, variants, *span, type_params, ctx, state);
                None
            }
            Self::Match(scrutinee, arms, wildcard, span) => {
                debug_assert!(!uses_id);
                compile_match(
                    scrutinee,
                    arms,
                    wildcard.as_deref(),
                    *span,
                    v,
                    ctx,
                    state,
                    output,
                );
                None
            }
            Self::NamespacedRef(path, span, type_args) => {
                debug_assert!(uses_id);
                if !type_args.is_empty() {
                    let (enum_id, variant_idx) =
                        resolve_generic_variant(path, type_args, *span, ctx, state);
                    return Some(compile_enum_construction(
                        enum_id,
                        variant_idx,
                        &[],
                        *span,
                        &[],
                        v,
                        ctx,
                        state,
                        output,
                    ));
                }
                if let Some((enum_id, variant_idx)) = resolve_enum_variant(path, state) {
                    Some(compile_enum_construction(
                        enum_id,
                        variant_idx,
                        &[],
                        *span,
                        &[],
                        v,
                        ctx,
                        state,
                        output,
                    ))
                } else {
                    compiler_errors::error_enum(
                        "Unknown enum variant",
                        &format!("{} does not name an enum variant", path.join("::")),
                        *span,
                        ctx.file_idx,
                        state.sources,
                    );
                }
            }
            Self::FunctionCall(args, namespace, markers, args_indexes, type_args) if !uses_id => {
                let output_id = handle_functions(
                    output,
                    v,
                    ctx,
                    state,
                    tgt_id,
                    args,
                    namespace,
                    *markers,
                    args_indexes,
                    type_args,
                );
                if let Some(id) = output_id {
                    state.free_reg(id, v);
                }
                None
            }
            Self::ObjFunctionCall(
                obj,
                args,
                namespace,
                obj_span,
                fn_span,
                args_indexes,
                type_args,
            ) if !uses_id => {
                let output_id = handle_method_calls(
                    output,
                    v,
                    ctx,
                    state,
                    tgt_id,
                    obj,
                    args,
                    namespace,
                    *obj_span,
                    *fn_span,
                    args_indexes,
                    type_args,
                );
                if let Some(id) = output_id {
                    state.free_reg(id, v);
                }
                None
            }
            Self::ObjFunctionCall(
                obj,
                args,
                namespace,
                obj_span,
                fn_span,
                args_indexes,
                type_args,
            ) if uses_id => Some(
                handle_method_calls(
                    output,
                    v,
                    ctx,
                    state,
                    tgt_id,
                    obj,
                    args,
                    namespace,
                    *obj_span,
                    *fn_span,
                    args_indexes,
                    type_args,
                )
                .unwrap_or_else(|| {
                    if let Some(&id) = state.const_registers.get(&NULL) {
                        id
                    } else {
                        let id = state.registers.len() as u16;
                        state.const_registers.insert(NULL, id);
                        state.registers.push(NULL);
                        id
                    }
                }),
            ),
            Self::FunctionDecl(fn_name, fn_args, fn_code, span, return_type, type_params) => {
                debug_assert!(!uses_id);
                compile_function_definition(
                    fn_name,
                    fn_args,
                    fn_code,
                    *span,
                    return_type.as_deref(),
                    type_params,
                    v,
                    ctx,
                    state,
                    output,
                );
                None
            }
            Self::ReturnVal(return_value) => {
                debug_assert!(!uses_id);
                compile_return(return_value.as_ref().as_ref(), v, ctx, state, output);
                None
            }
            Self::Break => {
                debug_assert!(!uses_id);
                compile_loop_break(ctx, output);
                None
            }
            Self::Continue => {
                debug_assert!(!uses_id);
                compile_loop_continue(ctx, output);
                None
            }
            Self::EvalBlock(code) => {
                debug_assert!(!uses_id);
                compile_eval_block(code, v, ctx, state, output);
                None
            }
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

#[cfg(target_arch = "aarch64")]
const ARCH_SUFFIX: &str = "-aarch64";
#[cfg(target_arch = "x86_64")]
const ARCH_SUFFIX: &str = "-x86_64";
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
const ARCH_SUFFIX: &str = "";

#[derive(Debug, Copy, Clone)]
pub enum SymbolKind {
    Fn(u16),
    Struct(u16),
    Enum(u16),
}

/// Whether two symbols are the same underlying definition (same kind, same
/// id in the global fn/struct/enum tables).
const fn symbol_ids_equal(a: SymbolKind, b: SymbolKind) -> bool {
    match (a, b) {
        (SymbolKind::Fn(x), SymbolKind::Fn(y))
        | (SymbolKind::Struct(x), SymbolKind::Struct(y))
        | (SymbolKind::Enum(x), SymbolKind::Enum(y)) => x == y,
        _ => false,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Namespace {
    pub symbols: Vec<(SmolStr, SymbolKind)>,
    pub children: Vec<(SmolStr, Self)>,
}

impl Namespace {
    pub fn fns(&self) -> impl Iterator<Item = &(SmolStr, SymbolKind)> {
        self.symbols
            .iter()
            .filter(|(_, kind)| matches!(kind, SymbolKind::Fn(_)))
    }
    pub fn structs(&self) -> impl Iterator<Item = &(SmolStr, SymbolKind)> {
        self.symbols
            .iter()
            .filter(|(_, kind)| matches!(kind, SymbolKind::Struct(_)))
    }
    #[must_use]
    pub fn find_function(
        &self,
        path: &[SmolStr],
        function_name: &str,
        span: Span,
        file_idx: u16,
        sources: &[Source],
    ) -> Option<usize> {
        self.walk_to_namespace(path, span, file_idx, sources)
            .symbols
            .iter()
            .find_map(|(name, kind)| {
                if name.as_str() == function_name
                    && let SymbolKind::Fn(fn_id) = kind
                {
                    Some(*fn_id as usize)
                } else {
                    None
                }
            })
    }
    #[must_use]
    pub fn find_struct(
        &self,
        path: &[SmolStr],
        struct_name: &str,
        span: Span,
        file_idx: u16,
        sources: &[Source],
    ) -> Option<usize> {
        self.walk_to_namespace(path, span, file_idx, sources)
            .symbols
            .iter()
            .find_map(|(name, kind)| {
                if name.as_str() == struct_name
                    && let SymbolKind::Struct(struct_id) = kind
                {
                    Some(*struct_id as usize)
                } else {
                    None
                }
            })
    }
    /// Resolves an enum type id by name (with an optional module path). Returns
    /// `None` when no enum by that name exists in the resolved namespace; never
    /// raises a compile error itself so it can be used for speculative
    /// enum-variant resolution against otherwise-unknown call/reference paths.
    #[must_use]
    pub fn find_enum(&self, path: &[SmolStr], enum_name: &str) -> Option<usize> {
        let mut current = self;
        for sub in path {
            current = &current.children.iter().find(|(name, _)| name == sub)?.1;
        }
        current.symbols.iter().find_map(|(name, kind)| {
            if name.as_str() == enum_name
                && let SymbolKind::Enum(enum_id) = kind
            {
                Some(*enum_id as usize)
            } else {
                None
            }
        })
    }
    #[must_use]
    pub fn walk_to_namespace(
        &self,
        path: &[SmolStr],
        span: Span,
        file_idx: u16,
        sources: &[Source],
    ) -> &Self {
        let mut current = self;
        for sub in path {
            current = if let Some((_, child_namespace)) =
                current.children.iter().find(|(name, _)| name == sub)
            {
                child_namespace
            } else {
                error_unknown_namespace(path, span, file_idx, sources);
            };
        }
        current
    }
}

/// Loads the `std/list` module implicitly so its `impl list` methods
/// (`arr.map(f)`, `arr.sum()`, ...) work with no explicit import. Resolution
/// mirrors the library-import path (`CANDELA_LIB_PATH` or `libs/` beside the
/// executable); a missing library directory is not an error, the prelude is
/// absent.
#[cfg(not(target_arch = "wasm32"))]
fn load_auto_prelude(
    fns: &mut Vec<Function>,
    structs: &mut Vec<Struct>,
    enums: &mut Vec<EnumType>,
    fn_registers: &mut Vec<Vec<u16>>,
    dynamic_libs: &mut Vec<Dynamiclib>,
    sources: &mut Vec<Source>,
    namespace: &mut Namespace,
    files: &mut FxHashMap<PathBuf, Namespace>,
    file_namespaces: &mut FxHashMap<u16, Namespace>,
    pending_structs: &mut Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)>,
    pending_enums: &mut PendingEnums,
    pending_fns: &mut PendingFns,
    pending_dylibs: &mut Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        Rc<Library>,
        SmolStr,
        Span,
    )>,
    pending_host: &mut Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )>,
    generics: &mut Generics,
) {
    const PRELUDE_REL: &str = "std/list.cdl";
    const PRELUDE_CHILD: &str = "list";

    if namespace
        .children
        .iter()
        .any(|(name, _)| name.as_str() == PRELUDE_CHILD)
    {
        return;
    }

    let path = if let Some(base) = std::env::var_os("CANDELA_LIB_PATH") {
        PathBuf::from(base).join(PRELUDE_REL)
    } else if let Ok(exe) = std::env::current_exe() {
        exe.canonicalize()
            .unwrap_or(exe)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("libs")
            .join(PRELUDE_REL)
    } else {
        return;
    };

    if let Some(cached) = files.get(&path) {
        namespace
            .children
            .push((PRELUDE_CHILD.into(), cached.clone()));
        return;
    }

    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };

    let child_src_idx = sources.len() as u16;
    let file_name: SmolStr = path.to_str().unwrap_or(PRELUDE_REL).into();
    sources.push(Source {
        filename: file_name,
        contents,
    });
    let parsed = parser::parse(&sources.last().unwrap().contents, sources.last().unwrap());
    generics.add_impls(parsed.impls, child_src_idx);

    let mut child_namespace = Namespace {
        symbols: Vec::new(),
        children: Vec::new(),
    };
    parse_toplevel(
        parsed.code,
        &path,
        child_src_idx,
        fns,
        structs,
        enums,
        fn_registers,
        dynamic_libs,
        sources,
        &mut child_namespace,
        files,
        file_namespaces,
        pending_structs,
        pending_enums,
        pending_fns,
        pending_dylibs,
        pending_host,
        generics,
    );
    files.insert(path, child_namespace.clone());
    namespace
        .children
        .push((PRELUDE_CHILD.into(), child_namespace));
}

/// Opens the library a bare logical import (`dylib "z"`) names. `filename` is
/// that name in this platform's convention (`resolve_library_filename`'s
/// output), and `dirs` are the directories it is looked for in, in order.
///
/// A bare filename handed to the OS loader (`dlopen` on Linux/macOS) is
/// resolved only through the system search path (the run-path,
/// `LD_LIBRARY_PATH`, `ld.so.cache`, `/lib`, `/usr/lib`), never the current
/// directory or the importing file's directory. Windows' `LoadLibraryA`
/// differs: it also searches the application directory and the current
/// directory by default. A library built to sit next to the `.cdl` file
/// (rather than installed as a system library) therefore loads on Windows but
/// silently fails on Linux/macOS.
///
/// To match Windows' default search order, the name is tried, in order: under
/// each directory in `dirs`, then in the current directory, and only then
/// handed to the OS loader bare, so genuine system libraries (`z`, `m`,
/// `sqlite3`) still resolve exactly as before.
#[cfg(not(target_arch = "wasm32"))]
fn open_logical_dylib(dirs: &[&Path], filename: &str) -> Option<Library> {
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(dirs.len() + 1);
    for dir in dirs {
        let candidate = dir.join(filename);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    let cwd_candidate = Path::new(".").join(filename);
    if !candidates.contains(&cwd_candidate) {
        candidates.push(cwd_candidate);
    }
    for candidate in &candidates {
        if let Ok(lib) = unsafe { Library::new(candidate) } {
            return Some(lib);
        }
    }
    unsafe { Library::new(filename) }.ok()
}

/// Opens the library a path import (`dylib "../native/mylib"`) names, and
/// returns the namespace name its functions live under.
///
/// A relative path is tried under each directory in `dirs`, in order; an
/// absolute one goes straight to the loader. The namespace comes from the last
/// component of the path either way, so where the file was found does not
/// change how the program calls into it.
#[cfg(not(target_arch = "wasm32"))]
fn open_path_dylib(dirs: &[&Path], spec: &str) -> (Option<Library>, SmolStr) {
    let name = Path::new(spec)
        .file_prefix()
        .and_then(|s| s.to_str())
        .unwrap_or(spec)
        .to_smolstr();

    if Path::new(spec).is_absolute() {
        return (open_library_path(spec), name);
    }
    for dir in dirs {
        if let Some(lib) = open_library_path(dir.join(spec).to_string_lossy().as_ref()) {
            return (Some(lib), name);
        }
    }
    (None, name)
}

/// Opens the library file `base` stands for.
///
/// A path that already carries an extension is used as written. One without
/// picks up an architecture-specific build when it sits beside it
/// (`mylib-x86_64.so`), so a single directory can hold builds for several
/// architectures, and otherwise gets this platform's extension.
#[cfg(not(target_arch = "wasm32"))]
fn open_library_path(base: &str) -> Option<Library> {
    let resolved = if Path::new(base).extension().is_none() {
        let arch_path = format!(
            "{base}{ARCH_SUFFIX}.{}",
            TargetOs::CURRENT.dynamic_lib_extension()
        );
        if Path::new(&arch_path).exists() {
            arch_path
        } else {
            resolve_library_filename(base, TargetOs::CURRENT)
        }
    } else {
        base.to_owned()
    };
    unsafe { Library::new(&resolved) }.ok()
}

/// Recursively collects functions, dyn libs, and imported files
/// Deferred enum-payload resolution: `(enum_id, src_file_idx, variants)`, filled
/// in `resolve_types` after every type name is registered, so an enum payload
/// may reference a type declared later.
type PendingEnums = Vec<(u16, u16, Box<[(SmolStr, Box<[TypeExpr]>, Span)]>)>;
/// Deferred function-signature resolution: `(fn_id, src_file_idx, args,
/// return_type, type_params)`, resolved in `resolve_types` once every type name
/// is registered.
type PendingFns = Vec<(
    u16,
    u16,
    Box<[(SmolStr, Option<TypeExpr>)]>,
    ReturnAnnotation,
    TypeParams,
)>;

fn parse_toplevel(
    code: Vec<Expr>,
    file_path: &Path,
    src_file_idx: u16,
    fns: &mut Vec<Function>,
    structs: &mut Vec<Struct>,
    enums: &mut Vec<EnumType>,
    fn_registers: &mut Vec<Vec<u16>>,
    dynamic_libs: &mut Vec<Dynamiclib>,
    sources: &mut Vec<Source>,
    namespace: &mut Namespace,
    files: &mut FxHashMap<PathBuf, Namespace>,
    file_namespaces: &mut FxHashMap<u16, Namespace>,
    pending_structs: &mut Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)>,
    pending_enums: &mut PendingEnums,
    pending_fns: &mut PendingFns,
    #[cfg(not(target_arch = "wasm32"))] pending_dylibs: &mut Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        Rc<Library>,
        // Library spec exactly as written in the source (logical name or path),
        // carried to the artifact recipe so a `.cdlb` re-resolves it by name.
        SmolStr,
        Span,
    )>,
    pending_host: &mut Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )>,
    generics: &mut Generics,
) {
    let mut imports = Vec::new();
    for expr in code {
        match expr {
            Expr::FunctionDecl(fn_name, fn_args, fn_code, span, fn_return_type, type_params) => {
                if let Some((_, SymbolKind::Fn(func_id))) =
                    namespace.symbols.iter().rfind(|(f, _)| f == &fn_name)
                {
                    let func = &fns[*func_id as usize];
                    compiler_errors::error_function_already_defined(
                        func,
                        span,
                        src_file_idx,
                        sources,
                    );
                }
                fn_registers.push(Vec::new());
                let returns_void = check_if_returns_void(&fn_code);
                let mut callees = Vec::new();
                collect_direct_fn_calls(&fn_code, &mut callees);

                let fn_id = fns.len() as u16;
                fns.push(Function {
                    name: fn_name.clone(),
                    args: Box::new([]),
                    code: fn_code,
                    impls: Vec::new(),
                    is_recursive: None,
                    returns_null: returns_void,
                    src_file: src_file_idx,
                    return_type_cache: Vec::new(),
                    direct_calls: callees.into_boxed_slice(),
                    name_span: span,
                    // Resolved with the argument types once every file's
                    // namespace is known; see the `pending_fns` drain.
                    return_type: None,
                    generics: (!type_params.is_empty()).then(|| {
                        Box::new(FnGenerics {
                            params: type_params.clone(),
                            arg_types: fn_args.iter().map(|(_, t)| t.clone()).collect(),
                            return_type: fn_return_type.clone(),
                            bindings: Box::from([]),
                            file_idx: src_file_idx,
                        })
                    }),
                });
                pending_fns.push((fn_id, src_file_idx, fn_args, fn_return_type, type_params));
                namespace.symbols.push((fn_name, SymbolKind::Fn(fn_id)));
            }
            Expr::StructDeclare(name, fields, span, type_params) => {
                // A generic declaration registers no type of its own: each
                // instantiation of it becomes an ordinary struct.
                if !type_params.is_empty() {
                    generics.add_struct_template(name, type_params, src_file_idx, fields);
                    continue;
                }
                let struct_id = structs.len() as u16;
                structs.push(Struct {
                    name: name.clone(),
                    fields: Box::from([]),
                    id: struct_id,
                    name_span: span,
                });
                namespace
                    .symbols
                    .push((name, SymbolKind::Struct(struct_id)));
                pending_structs.push((struct_id, src_file_idx, fields));
            }
            Expr::EnumDeclare(name, variants, span, type_params) => {
                if !type_params.is_empty() {
                    generics.add_enum_template(name, type_params, src_file_idx, variants);
                    continue;
                }
                let enum_id = enums.len() as u16;
                enums.push(EnumType {
                    name: name.clone(),
                    variants: Box::from([]),
                    id: enum_id,
                    name_span: span,
                });
                namespace.symbols.push((name, SymbolKind::Enum(enum_id)));
                pending_enums.push((enum_id, src_file_idx, variants));
            }
            #[cfg(target_arch = "wasm32")]
            Expr::ImportDylib(..) => wasm_error("WASM does not support loading dynamic libraries"),
            #[cfg(target_arch = "wasm32")]
            Expr::ImportFile(..) => wasm_error("WASM does not support importing files"),
            import @ (Expr::ImportFile(..) | Expr::ImportDylib(..) | Expr::HostBlock(..)) => {
                imports.push(import);
            }
            _ => {}
        }
    }

    files.insert(file_path.to_path_buf(), namespace.clone());

    // Auto-prelude: make the std::list array methods (map/filter/reduce and
    // friends) callable as methods on arrays without an explicit import. This is
    // best-effort: if the shipped library directory is not present (for
    // example an embedding host with no `libs/` tree), the prelude is skipped and
    // array methods resolve as they did before.
    #[cfg(not(target_arch = "wasm32"))]
    if src_file_idx == 0 {
        load_auto_prelude(
            fns,
            structs,
            enums,
            fn_registers,
            dynamic_libs,
            sources,
            namespace,
            files,
            file_namespaces,
            pending_structs,
            pending_enums,
            pending_fns,
            pending_dylibs,
            pending_host,
            generics,
        );
    }

    // Names merged into this file's scope by bare imports, with the module
    // each came from; consulted to report both sources on a collision.
    let mut merged_symbol_origins: Vec<(SmolStr, SmolStr)> = Vec::new();

    for import in imports {
        match import {
            #[cfg(not(target_arch = "wasm32"))]
            Expr::ImportDylib(path, fn_signatures, span) => {
                // The spec exactly as written; recorded in the artifact recipe so
                // a `.cdlb` re-resolves the library by name (per-OS) at load.
                let spec = path.clone();
                // A bare logical name (no path separator, not absolute, no
                // extension, e.g. `z`, `sqlite3`) names a system library the
                // OS loader searches for. Anything with a separator or extension
                // is an explicit path.
                let is_logical = !spec.contains('/')
                    && !spec.contains('\\')
                    && !Path::new(spec.as_str()).is_absolute()
                    && Path::new(spec.as_str()).extension().is_none();

                // Where a relative library reference is looked for, in order:
                // the directory the embedding host named, then the importing
                // file's own. A host whose libraries sit apart from its sources
                // (`lib/` beside `src/`) names that directory and both forms
                // find it.
                let host_dir = dylib_dir();
                let file_dir = file_path.parent().unwrap_or_else(|| Path::new("."));
                let mut dirs: Vec<&Path> = Vec::with_capacity(2);
                if let Some(dir) = host_dir.as_deref() {
                    dirs.push(dir);
                }
                dirs.push(file_dir);

                let (lib, dylib_name) = if is_logical {
                    // e.g. `z` -> `libz.so` / `libz.dylib` / `z.dll`.
                    let filename = resolve_library_filename(spec.as_str(), TargetOs::CURRENT);
                    (open_logical_dylib(&dirs, &filename), spec.clone())
                } else {
                    open_path_dylib(&dirs, spec.as_str())
                };

                let lib = Rc::new(
                    lib.unwrap_or_else(|| error_cannot_load_dynlib(span, src_file_idx, sources)),
                );
                pending_dylibs.push((
                    src_file_idx,
                    dynamic_libs.len() as u16,
                    fn_signatures,
                    lib,
                    spec,
                    span,
                ));
                dynamic_libs.push(Dynamiclib {
                    name: dylib_name,
                    fns: Box::new([]),
                    is_host: false,
                });
            }
            Expr::HostBlock(host_namespace, fn_signatures, span) => {
                pending_host.push((
                    src_file_idx,
                    dynamic_libs.len() as u16,
                    fn_signatures,
                    host_namespace.clone(),
                    span,
                ));
                dynamic_libs.push(Dynamiclib {
                    name: host_namespace,
                    fns: Box::new([]),
                    is_host: true,
                });
            }
            Expr::ImportFile(path, alias, is_logical, span) => {
                // The shipped library directory: `CANDELA_LIB_PATH` overrides its
                // location (it names the `libs/` dir that holds `std/` and, for the
                // C-backed modules, `std_src/`); otherwise it is `libs/` beside the
                // running executable, which is where the toolchain installs it. This
                // is the single source of truth for the default std location.
                let shipped_lib = |path: &SmolStr| -> Option<PathBuf> {
                    if let Some(base) = std::env::var_os("CANDELA_LIB_PATH") {
                        return Some(PathBuf::from(base).join(path.as_str()));
                    }
                    std::env::current_exe().ok().map(|p| {
                        p.canonicalize()
                            .unwrap_or(p)
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .join("libs")
                            .join(path.as_str())
                    })
                };

                let file_path = if is_logical {
                    // A library import (`import "std/string";`, extensionless)
                    // resolves against the shipped library directory only, never
                    // source-relative, so it works from any working directory
                    // with nothing set.
                    shipped_lib(&path).unwrap_or_else(|| {
                        error_cannot_read_file(span, src_file_idx, sources);
                    })
                } else {
                    // A `.cdl` file import resolves next to the importing file first,
                    // then falls back to the shipped library directory.
                    file_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(path.as_str())
                        .canonicalize()
                        .unwrap_or_else(|_| {
                            shipped_lib(&path).unwrap_or_else(|| {
                                error_cannot_read_file(span, src_file_idx, sources);
                            })
                        })
                };

                let child_namespace = if let Some(cached) = files.get(&file_path) {
                    cached.clone()
                } else {
                    let file_contents = std::fs::read_to_string(&file_path).unwrap_or_else(|_| {
                        error_cannot_read_file(span, src_file_idx, sources);
                    });
                    let file_name: SmolStr = file_path.to_str().unwrap_or(path.as_str()).into();

                    let child_src_idx = sources.len() as u16;

                    sources.push(Source {
                        filename: file_name.clone(),
                        contents: file_contents,
                    });

                    // Parse the imported file's contents
                    let parsed =
                        parser::parse(&sources.last().unwrap().contents, sources.last().unwrap());
                    generics.add_impls(parsed.impls, child_src_idx);

                    let mut child_namespace = Namespace {
                        symbols: Vec::new(),
                        children: Vec::new(),
                    };

                    parse_toplevel(
                        parsed.code,
                        &file_path,
                        child_src_idx,
                        fns,
                        structs,
                        enums,
                        fn_registers,
                        dynamic_libs,
                        sources,
                        &mut child_namespace,
                        files,
                        file_namespaces,
                        pending_structs,
                        pending_enums,
                        pending_fns,
                        #[cfg(not(target_arch = "wasm32"))]
                        pending_dylibs,
                        pending_host,
                        generics,
                    );
                    files.insert(file_path.clone(), child_namespace.clone());
                    child_namespace
                };

                if let Some(alias) = alias {
                    // `import "..." as name;` binds the module under a
                    // namespace: its symbols are reachable as `name::symbol`.
                    namespace.children.push((alias, child_namespace));
                } else {
                    // A bare import merges the module's symbols into this
                    // file's own scope. The module path as written, used to
                    // name the source in a collision error.
                    let module_display: SmolStr = if is_logical {
                        path.strip_suffix(".cdl").unwrap_or(path.as_str()).into()
                    } else {
                        path.clone()
                    };
                    for (name, kind) in child_namespace.symbols {
                        if let Some((_, existing)) =
                            namespace.symbols.iter().find(|(n, _)| n == &name)
                        {
                            // The same underlying symbol arriving through two
                            // routes (for example two modules that both import
                            // a third) is not a conflict.
                            if symbol_ids_equal(*existing, kind) {
                                continue;
                            }
                            let existing_origin = merged_symbol_origins
                                .iter()
                                .find(|(n, _)| n == &name)
                                .map_or_else(
                                    || String::from("defined in this file"),
                                    |(_, module)| format!("imported from \"{module}\""),
                                );
                            compiler_errors::error_import_symbol_collision(
                                &name,
                                &existing_origin,
                                &module_display,
                                span,
                                src_file_idx,
                                sources,
                            );
                        }
                        merged_symbol_origins.push((name.clone(), module_display.clone()));
                        namespace.symbols.push((name, kind));
                    }
                }
            }
            _ => unsafe { unreachable_unchecked() },
        }
    }
    file_namespaces.insert(src_file_idx, namespace.clone());
}

fn resolve_types(
    structs: &mut Vec<Struct>,
    enums: &mut Vec<EnumType>,
    fns: &mut Vec<Function>,
    fn_registers: &mut Vec<Vec<u16>>,
    generics: &mut Generics,
    pending_structs: Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)>,
    pending_enums: PendingEnums,
    pending_fns: PendingFns,
    #[cfg(not(target_arch = "wasm32"))] pending_dylibs: Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        Rc<Library>,
        // Library spec exactly as written in the source (logical name or path),
        // carried to the artifact recipe so a `.cdlb` re-resolves it by name.
        SmolStr,
        Span,
    )>,
    pending_host: Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )>,
    file_namespaces: &FxHashMap<u16, Namespace>,
    dynamic_libs_fns: &mut Vec<DynamicLibFn>,
    host_fns: &mut Vec<HostFnSig>,
    dynamic_libs: &mut [Dynamiclib],
    sources: &[Source],
) {
    // An `impl` on a generic type has to name the type arguments it is written
    // against: `impl Cell` has no instantiation to attach its methods to.
    for func in fns.iter() {
        let Some((type_name, _)) = func.name.split_once(METHOD_SEP) else {
            continue;
        };
        if let Some(params) = generics.params_of(type_name) {
            error_type_arg_count(
                func.name_span,
                func.src_file,
                type_name,
                params.len(),
                0,
                sources,
            );
        }
    }
    for (struct_id, src_file_idx, fields) in pending_structs {
        let namespace = file_namespaces[&src_file_idx].clone();
        let resolved_fields = fields
            .iter()
            .map(|(field_name, field_type, field_span)| {
                (
                    field_name.clone(),
                    field_type.to_datatype(&mut TypeCtx {
                        file_idx: src_file_idx,
                        namespace: &namespace,
                        sources,
                        structs,
                        enums,
                        fns,
                        fn_registers,
                        generics,
                    }),
                    *field_span,
                )
            })
            .collect();
        structs[struct_id as usize].fields = resolved_fields;
    }
    for (enum_id, src_file_idx, variants) in pending_enums {
        let namespace = file_namespaces[&src_file_idx].clone();
        let resolved_variants = variants
            .iter()
            .map(|(variant_name, payload, name_span)| EnumVariant {
                name: variant_name.clone(),
                payload: payload
                    .iter()
                    .map(|t| {
                        t.to_datatype(&mut TypeCtx {
                            file_idx: src_file_idx,
                            namespace: &namespace,
                            sources,
                            structs,
                            enums,
                            fns,
                            fn_registers,
                            generics,
                        })
                    })
                    .collect(),
                name_span: *name_span,
            })
            .collect();
        enums[enum_id as usize].variants = resolved_variants;
    }
    for (fn_id, src_file_idx, args, return_type, type_params) in pending_fns {
        let namespace = file_namespaces[&src_file_idx].clone();
        // An annotation naming one of the function's own type parameters is
        // left un-pinned: it resolves per call site, once the call names its
        // type arguments.
        let resolved_args = args
            .iter()
            .map(|(arg_name, arg_type)| {
                (
                    arg_name.clone(),
                    arg_type
                        .clone()
                        .filter(|t_e| !t_e.mentions_any(&type_params))
                        .map(|t_e| {
                            t_e.to_datatype(&mut TypeCtx {
                                file_idx: src_file_idx,
                                namespace: &namespace,
                                sources,
                                structs,
                                enums,
                                fns,
                                fn_registers,
                                generics,
                            })
                        }),
                )
            })
            .collect();
        fns[fn_id as usize].args = resolved_args;
        fns[fn_id as usize].return_type = return_type
            .map(|annotation| *annotation)
            .filter(|(t_e, _)| !t_e.mentions_any(&type_params))
            .map(|(t_e, t_span)| {
                (
                    t_e.to_datatype(&mut TypeCtx {
                        file_idx: src_file_idx,
                        namespace: &namespace,
                        sources,
                        structs,
                        enums,
                        fns,
                        fn_registers,
                        generics,
                    }),
                    t_span,
                )
            });
    }
    #[cfg(not(target_arch = "wasm32"))]
    for (src_file_idx, dynlib_id, fn_signatures, lib, library_spec, span) in pending_dylibs {
        let namespace = &file_namespaces[&src_file_idx];
        let resolved: Vec<FnSignature> = fn_signatures
            .iter()
            .map(|(fn_name, fn_args, fn_return_type, fn_name_span)| {
                let fn_args = fn_args
                    .iter()
                    .map(|t| {
                        t.to_datatype(&mut TypeCtx {
                            file_idx: src_file_idx,
                            namespace,
                            sources,
                            structs,
                            enums,
                            fns,
                            fn_registers,
                            generics,
                        })
                    })
                    .collect::<Vec<DataType>>()
                    .into_boxed_slice();
                let fn_return_type = fn_return_type.to_datatype(&mut TypeCtx {
                    file_idx: src_file_idx,
                    namespace,
                    sources,
                    structs,
                    enums,
                    fns,
                    fn_registers,
                    generics,
                });
                let return_val = FnSignature {
                    name: fn_name.clone(),
                    args: fn_args.clone(),
                    return_type: fn_return_type.clone(),
                    id: dynamic_libs_fns.len() as u16,
                    variadic: false,
                };
                let arg_types: Vec<_> = fn_args.iter().map(|t| t.to_c_type(structs)).collect();
                let return_type = fn_return_type.to_c_type(structs);
                let cif = libffi::middle::Cif::new(arg_types, return_type);
                let ptr = unsafe {
                    libffi::middle::CodePtr(
                        lib.get::<*const ()>(fn_name.as_bytes())
                            .unwrap_or_else(|_| {
                                error_cannot_find_dynlib_symbol(
                                    fn_name,
                                    *fn_name_span,
                                    span,
                                    src_file_idx,
                                    sources,
                                );
                            })
                            .try_as_raw_ptr()
                            .unwrap_unchecked(),
                    )
                };

                let mut types = vec![fn_return_type];
                types.extend(fn_args);

                dynamic_libs_fns.push(DynamicLibFn {
                    types: Box::from(types),
                    library: library_spec.clone(),
                    symbol: fn_name.clone(),
                    _lib: Rc::clone(&lib),
                    ptr,
                    cif,
                });
                return_val
            })
            .collect();
        dynamic_libs[dynlib_id as usize].fns = resolved.into_boxed_slice();
    }

    // Resolve `host "..." { ... }` blocks. Unlike dylibs there is no shared
    // object to load or FFI CIF to build: each signature is type-checked and
    // recorded as a `HostFnSig` whose `id` the VM later uses to dispatch to the
    // Rust closure the embedding `Engine` bound to `(namespace, name)`.
    for (src_file_idx, dynlib_id, fn_signatures, host_namespace, _span) in pending_host {
        let namespace = &file_namespaces[&src_file_idx];
        let resolved: Vec<FnSignature> = fn_signatures
            .iter()
            .map(|(fn_name, fn_args, fn_return_type, _fn_name_span)| {
                // A lone `...` sentinel argument marks a variadic host fn: it
                // takes no fixed argument types, and the call site forwards
                // every supplied argument to the registered closure.
                let variadic = fn_args.len() == 1
                    && matches!(&fn_args[0], TypeExpr::Identifier(s, _) if s.as_str() == "...");
                let fn_args = if variadic {
                    Box::from([])
                } else {
                    fn_args
                        .iter()
                        .map(|t| {
                            t.to_datatype(&mut TypeCtx {
                                file_idx: src_file_idx,
                                namespace,
                                sources,
                                structs,
                                enums,
                                fns,
                                fn_registers,
                                generics,
                            })
                        })
                        .collect::<Vec<DataType>>()
                        .into_boxed_slice()
                };
                let fn_return_type = fn_return_type.to_datatype(&mut TypeCtx {
                    file_idx: src_file_idx,
                    namespace,
                    sources,
                    structs,
                    enums,
                    fns,
                    fn_registers,
                    generics,
                });
                let return_val = FnSignature {
                    name: fn_name.clone(),
                    args: fn_args.clone(),
                    return_type: fn_return_type.clone(),
                    id: host_fns.len() as u16,
                    variadic,
                };

                let mut types = vec![fn_return_type];
                types.extend(fn_args);
                host_fns.push(HostFnSig {
                    types: types.into_boxed_slice(),
                    namespace: host_namespace.clone(),
                    name: fn_name.clone(),
                    variadic,
                });
                return_val
            })
            .collect();
        dynamic_libs[dynlib_id as usize].fns = resolved.into_boxed_slice();
    }
}

/// The complete result of compiling a candela program.
///
/// In addition to the fields the CLI/VM needs to run `main`, this carries the
/// compiler-side tables (`functions`, `dyn_libs`, `namespace`, register
/// bookkeeping) that the embedding `Program` keeps alive so it can compile
/// additional function specializations on demand for `Program::call`, plus the
/// resolved `host_fns` signature table used to dispatch `host` calls.
pub struct CompileOutput {
    pub instructions: Vec<Instr>,
    pub registers: Vec<Data>,
    pub pools: Pools,
    pub instr_src: Vec<InstrSrc>,
    pub fn_registers: Vec<Vec<u16>>,
    pub dyn_lib_fns: Vec<DynamicLibFn>,
    pub host_fns: Vec<HostFnSig>,
    pub allocated_arg_count: usize,
    pub allocated_call_depth: usize,
    pub sources: Vec<Source>,
    pub structs: Vec<Struct>,
    pub enums: Vec<EnumType>,
    pub functions: Vec<Function>,
    pub dyn_libs: Vec<Dynamiclib>,
    pub namespace: Namespace,
    pub const_registers: FxHashMap<Data, u16>,
    pub free_registers: Vec<u16>,
    /// The generic declarations and the instantiations made from them, kept so
    /// a later compile against this program (an embedding host calling in, a
    /// REPL line) resolves them the same way.
    pub generics: Generics,
}

#[must_use]
pub fn compile(contents: String, filename: &str, debug: bool) -> CompileOutput {
    #[cfg(not(target_arch = "wasm32"))]
    let now = std::time::Instant::now();

    // A previous compilation on this thread may have been aborted mid-inference
    // by an error unwind; make sure its bookkeeping doesn't leak into this one.
    type_system::reset_inference_state();

    let main_src = Source {
        filename: SmolStr::from(filename),
        contents,
    };

    let parsed = parser::parse(&main_src.contents, &main_src);

    #[cfg(not(target_arch = "wasm32"))]
    if debug {
        println!("PARSING TIME: {:.2?}", now.elapsed());
    }

    let mut variables: Vec<Variable> = Vec::new();
    let mut registers: Vec<Data> = Vec::new();
    let mut pools: Pools = Pools {
        objs: Pool::with_capacity(10),
        maps: Pool::with_capacity(2),
        strings: Pool::with_capacity(10),
    };
    let mut instr_src: Vec<InstrSrc> = Vec::new();
    let mut fn_registers: Vec<Vec<u16>> = Vec::new();
    let mut functions: Vec<Function> = Vec::new();
    let mut structs: Vec<Struct> = Vec::new();
    let mut enums: Vec<EnumType> = Vec::new();
    let mut dyn_libs: Vec<Dynamiclib> = Vec::new();
    let mut dyn_lib_fns: Vec<DynamicLibFn> = Vec::new();
    let mut host_fns: Vec<HostFnSig> = Vec::new();
    let mut allocated_arg_count = 0;
    let mut allocated_call_depth = 0;
    let mut const_registers: FxHashMap<Data, u16> = FxHashMap::default();
    let mut free_registers = Vec::new();

    let mut sources: Vec<Source> = vec![main_src];
    let main_path = PathBuf::from(filename)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(filename));
    let mut namespace = Namespace::default();

    let mut files: FxHashMap<PathBuf, Namespace> = FxHashMap::default();
    let mut file_namespaces: FxHashMap<u16, Namespace> = FxHashMap::default();
    let mut pending_structs: Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)> = Vec::new();
    let mut pending_enums: PendingEnums = Vec::new();
    let mut pending_fns: PendingFns = Vec::with_capacity(2);
    let mut generics = Generics::default();
    #[cfg(not(target_arch = "wasm32"))]
    let mut pending_dylibs: Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        Rc<Library>,
        SmolStr,
        Span,
    )> = Vec::new();
    let mut pending_host: Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )> = Vec::new();

    generics.add_impls(parsed.impls, 0);
    parse_toplevel(
        parsed.code,
        &main_path,
        0,
        &mut functions,
        &mut structs,
        &mut enums,
        &mut fn_registers,
        &mut dyn_libs,
        &mut sources,
        &mut namespace,
        &mut files,
        &mut file_namespaces,
        &mut pending_structs,
        &mut pending_enums,
        &mut pending_fns,
        #[cfg(not(target_arch = "wasm32"))]
        &mut pending_dylibs,
        &mut pending_host,
        &mut generics,
    );
    // Every file's scope is known once the whole import tree is parsed; an
    // instantiation resolves a template against the scope it was declared in.
    generics.set_file_namespaces(&file_namespaces);
    resolve_types(
        &mut structs,
        &mut enums,
        &mut functions,
        &mut fn_registers,
        &mut generics,
        pending_structs,
        pending_enums,
        pending_fns,
        #[cfg(not(target_arch = "wasm32"))]
        pending_dylibs,
        pending_host,
        &file_namespaces,
        &mut dyn_lib_fns,
        &mut host_fns,
        &mut dyn_libs,
        &sources,
    );

    let ctx = Ctx {
        block_id: 0,
        is_compiling_recursive: false,
        file_idx: 0,
        single_run: true,
        in_function: false,
        offset: 0,
    };
    let mut state = State {
        registers: &mut registers,
        fns: &mut functions,
        structs: &mut structs,
        enums: &mut enums,
        pools: &mut pools,
        instr_src: &mut instr_src,
        fn_registers: &mut fn_registers,
        dyn_libs: &mut dyn_libs,
        allocated_arg_count: &mut allocated_arg_count,
        allocated_call_depth: &mut allocated_call_depth,
        const_registers: &mut const_registers,
        free_registers: &mut free_registers,
        sources: &mut sources,
        reserved_registers: FxHashSet::default(),
        namespace: &mut namespace,
        generics: &mut generics,
    };
    let mut instructions = compile_expr(
        &state.fns
            .iter()
            .find(|func| func.name == "main" && func.src_file == 0)
            .unwrap_or_else(|| {
                #[cfg(target_arch = "wasm32")]
                wasm_error("Cannot find main function");

                if crate::errors::diagnostics_enabled() {
                    crate::errors::emit_diagnostic(
                        state.sources[0].filename.as_str(),
                        0..0,
                        String::from("Cannot find main function"),
                        "no_main_function",
                    );
                }
                eprintln!(
                    "--------------\n{RED}CANDELA RUNTIME ERROR:{RESET}\nCannot find {BLUE}{BOLD}main{RESET} function\n--------------",
                );
                std::process::exit(1);
            })
            .code
            .clone(),
        &mut variables,
        ctx,
        &mut state,
    );
    instructions.push(Instr::Halt(0));

    #[cfg(debug_assertions)]
    if debug {
        println!("---- DEBUG ----");
        if !pools.objs.is_empty() {
            println!("---  ARRAYS  ---");
            for (i, data) in pools.objs.iter().enumerate() {
                println!(" {i} {data:?}");
            }
        }
        println!("-- REGISTERS --");
        for (i, data) in registers.iter().enumerate() {
            println!(
                " [{i}] {}",
                data.format(
                    &pools.objs,
                    &pools.strings,
                    &pools.maps,
                    &structs,
                    &enums,
                    true
                )
            );
        }
        if !instructions.is_empty() {
            println!("-- INSTRUCTIONS --");
            for (i, instr) in instructions.iter().enumerate() {
                println!(" {i}: {instr:?}");
            }
        }
        println!("------------------");
    }

    CompileOutput {
        instructions,
        registers,
        pools,
        instr_src,
        fn_registers,
        dyn_lib_fns,
        host_fns,
        allocated_arg_count,
        allocated_call_depth,
        sources,
        structs,
        enums,
        functions,
        dyn_libs,
        namespace,
        const_registers,
        free_registers,
        generics,
    }
}
