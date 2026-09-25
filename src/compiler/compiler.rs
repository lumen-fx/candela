// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::cold_path;
use crate::compiler::compiler_data::InstrSrc;
use crate::compiler::compiler_data::Source;
use crate::compiler::compiler_data::TypeNames;
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
use crate::compiler::compiler_errors::error_non_bool_condition;
use crate::compiler::compiler_errors::error_not_literal_map_key;
use crate::compiler::compiler_errors::error_range_invalid_type;
use crate::compiler::compiler_errors::error_shift_count_out_of_range;
use crate::compiler::compiler_errors::error_type_arg_count;
use crate::compiler::compiler_errors::error_type_has_no_c_representation;
use crate::compiler::compiler_errors::error_type_not_indexable;
use crate::compiler::compiler_errors::error_unknown_namespace;
use crate::compiler::imports::ImportResolver;
use crate::data::NULL;
use crate::instr::LibFunc;
use crate::parser;
use crate::rt::FnValue;
use crate::rt::LibraryOrigin;
use crate::rt::TargetOs;
use crate::rt::resolve_library_filename;
use crate::vm::CandelaMap;
use crate::vm::Pool;
use crate::vm::StringPool;
use crate::vm::map_insert;
use crate::vm::shift_count_in_range;
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
use compiler_data::IndirectRegisters;
use compiler_data::Pools;
use compiler_data::ReturnCheck;
use compiler_data::State;
use compiler_data::Struct;
use compiler_data::Variable;
use expr::Expr;
use expr::METHOD_SEP;
use expr::Span;
use expr::UNARY_MINUS_METHOD;
use expr::code_captures_variable;
use expr::code_modifies_variable;
use functions::handle_functions;
use functions::handle_value_call;
use functions::user_functions::check_declared_fn;
use functions::user_functions::declared_holds_fn_signature;
use functions::user_functions::ensure_indirect_impl;
use methods::compile_operator_method;
use methods::handle_method_calls;
use registers::int_immediate;
use registers::move_reg_to_reg;
use registers::move_to_id;
pub(crate) use registers::use_immediates;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
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
use type_system::bare_fn_value_type;
use type_system::check_if_returns_void;
use type_system::check_operator_method;
use type_system::collect_direct_fn_calls;
use type_system::param_type_matches;
use type_system::qualify_duplicate_type_names;
use type_system::resolve_generic_variant;
use type_system::struct_field_type_matches;
use type_system::struct_literal_id;
use type_system::variant_constructor_at;

#[cfg(not(target_arch = "wasm32"))]
use libloading::Library;

#[cfg(target_arch = "wasm32")]
use crate::errors::wasm_error;

/// What keeps a bound library open. Where no library loads, a binding is one
/// of the runtime's intrinsics and there is nothing to hold. A check-only
/// compile opens no library, so it holds `None`.
#[cfg(not(target_arch = "wasm32"))]
type LibHandle = Option<Rc<Library>>;
#[cfg(target_arch = "wasm32")]
type LibHandle = ();

thread_local! {
    /// Set while a check-only compile runs; see [`check_only_scope`].
    static CHECK_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Puts the previous check-only setting back when dropped, so a compile that
/// unwinds out of an error leaves it as it found it.
struct CheckOnlyGuard(bool);

impl Drop for CheckOnlyGuard {
    fn drop(&mut self) {
        CHECK_ONLY.set(self.0);
    }
}

/// Runs `f` with every compile inside it checking only.
///
/// A `dylib` block binds the signatures it declares and opens no library,
/// looks up no symbol and builds no calling interface. The program that comes
/// out type-checks the same as a full compile but cannot call into a library,
/// so it is for checking, never for running.
pub fn check_only_scope<R>(f: impl FnOnce() -> R) -> R {
    let _guard = CheckOnlyGuard(CHECK_ONLY.replace(true));
    f()
}

/// Whether the compile running now is a check-only one.
#[cfg(not(target_arch = "wasm32"))]
fn is_check_only() -> bool {
    CHECK_ONLY.get()
}

/// A `dylib` block waiting for the types it names to resolve: the file it is
/// in, its slot in the library table, its signatures, the open library, the
/// spec an artifact records for it with the search that resolves that spec,
/// and where it was written.
type PendingDylib = (
    u16,
    u16,
    Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
    LibHandle,
    (SmolStr, LibraryOrigin),
    Span,
);
pub mod compiler_data;
mod compiler_errors;
pub mod imports;
pub mod type_system;

pub mod expr;

#[path = "functions/functions.rs"]
mod functions;
#[path = "functions/methods.rs"]
mod methods;

mod copies;
mod flow;
mod registers;
mod scalar;

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
        | Instr::StepIntJmpBack(_, _, jump_size)
        | Instr::NotEqJmp(_, _, jump_size)
        | Instr::EqJmp(_, _, jump_size)
        | Instr::ObjNotEqJmp(_, _, jump_size)
        | Instr::ObjEqJmp(_, _, jump_size)
        | Instr::StrNotEqJmp(_, _, jump_size)
        | Instr::StrEqJmp(_, _, jump_size) => *jump_size = size,
        _ => unsafe { unreachable_unchecked() },
    }
}

/// Compiles one condition operand and checks that it is a `bool`.
///
/// candela has no truthiness, so a condition whose type the compiler knows and
/// which is not `bool` is reported where it is written. A condition typed `any`
/// carries no type to check against, so it goes through `AsBoolVal`, which
/// hands a bool straight back and raises `bad_downcast` on anything else; only
/// that path pays for the check.
fn compile_condition_operand(
    expr: &Expr,
    condition_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let condition_type = expr.infer_type(v, ctx, state);
    match condition_type {
        DataType::Bool => expr
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id(),
        DataType::Unknown => {
            let cond_id = expr
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(cond_id, v);
            let checked_id = state.alloc_reg();
            output.push(Instr::CallLibFunc(LibFunc::AsBoolVal, cond_id, checked_id));
            checked_id
        }
        other => error_non_bool_condition(
            condition_span,
            &other,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        ),
    }
}

/// Compiles short-circuit && and || conditions
/// bool_or_mode true indicates left side of ||, emits true jumps
/// bool_or_mode false emits false jumps
/// Returns (true_jump_idxs, false_jump_idxs)
#[allow(clippy::too_many_arguments)]
fn compile_short_circuit_condition(
    expr: &Expr,
    condition_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
    bool_or_mode: bool,
) -> (Vec<usize>, Vec<usize>) {
    match expr {
        Expr::BoolOr(left, right, left_span, right_span) => {
            // left side of || always uses true jump mode
            let (mut true_jumps, left_false) =
                compile_short_circuit_condition(left, *left_span, v, ctx, state, output, true);
            // A false left operand does not settle `||`, so it continues into
            // the right operand rather than out of the whole expression.
            let right_start = output.len();
            for j in left_false {
                set_jmp_size(&mut output[j], (right_start - j) as u16);
            }
            let (right_true, right_false) = compile_short_circuit_condition(
                right,
                *right_span,
                v,
                ctx,
                state,
                output,
                bool_or_mode,
            );
            true_jumps.extend(right_true);
            (true_jumps, right_false)
        }
        Expr::BoolAnd(left, right, left_span, right_span) => {
            if bool_or_mode {
                // `&&` on the left of `||`, where the caller wants jumps taken
                // when this conjunction is true. A false left operand settles
                // the conjunction, so its false jumps skip the right operand
                // and land on whatever is emitted next, which is exactly where
                // the enclosing `||` continues.
                let (_, left_false) =
                    compile_short_circuit_condition(left, *left_span, v, ctx, state, output, false);
                let (right_true, _) = compile_short_circuit_condition(
                    right,
                    *right_span,
                    v,
                    ctx,
                    state,
                    output,
                    true,
                );
                let fallthrough = output.len();
                for j in left_false {
                    set_jmp_size(&mut output[j], (fallthrough - j) as u16);
                }
                (right_true, Vec::new())
            } else {
                // normal && -> if either side is false, jump past the body
                let (left_true, mut false_jumps) =
                    compile_short_circuit_condition(left, *left_span, v, ctx, state, output, false);
                // A true left operand does not settle `&&`, so it continues
                // into the right operand. Only the right operand's true jumps
                // settle the conjunction, and the caller aims those at the body.
                let right_start = output.len();
                for j in left_true {
                    set_jmp_size(&mut output[j], (right_start - j) as u16);
                }
                let (right_true, right_false) = compile_short_circuit_condition(
                    right,
                    *right_span,
                    v,
                    ctx,
                    state,
                    output,
                    false,
                );
                false_jumps.extend(right_false);
                (right_true, false_jumps)
            }
        }
        expr => {
            let cond_id = compile_condition_operand(expr, condition_span, v, ctx, state, output);
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
            // A `continue` lands on the instruction that ends the turn: a
            // `for` loop's step, or a `loop` block's or `while` loop's jump
            // back, each the last instruction of its loop.
            if for_loop {
                *x = Instr::Jmp(code_length - i as u16 - 2);
            } else {
                *x = Instr::Jmp(code_length - i as u16 - 1);
            }
        }
    });
}

/// Compiles `expr` as a function value: a list whose first slot says where the
/// function's body starts, followed by the cells it captured.
///
/// A closure that captures already compiles to exactly that, and so does a
/// value read out of a position that holds one, so a value is built here only
/// for a function that captures nothing.
pub(crate) fn compile_fn_value(
    expr: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
    tgt_id: Option<u16>,
) -> u16 {
    if let DataType::Fn(fn_id) = expr.infer_type(v, ctx, state)
        && state.fns[fn_id as usize].captures.is_empty()
    {
        return compile_fn_entry_value(fn_id as usize, v, ctx, state, output, tgt_id);
    }
    expr.compile(v, ctx, state, output, tgt_id, false, true)
        .unwrap_id()
}

/// Builds the value of a function that captures nothing: one slot saying where
/// the body starts, which is what a call through the value jumps to.
///
/// A function that annotates every parameter is compiled at those types here,
/// unless a call through a value already gave it a body. The value can go where
/// the compiler stops following it, into an `any`, and come back out as a
/// `fn(...)` type, so no call site would compile it; the annotations say what
/// such a call passes.
pub(crate) fn compile_fn_entry_value(
    fn_id: usize,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
    tgt_id: Option<u16>,
) -> u16 {
    if let Some(params) = state.fns[fn_id].declared_params()
        && !state.fns[fn_id]
            .impls
            .iter()
            .any(|fn_impl| fn_impl.indirect)
    {
        let span = state.fns[fn_id].name_span;
        ensure_indirect_impl(fn_id, &params, span, output, v, ctx, state);
    }
    let entry_id = state.fn_entry_register(fn_id);
    let value_id = state.alloc_reg_tgt(tgt_id);
    output.push(Instr::EmptyFnValue(value_id));
    output.push(Instr::Push(value_id, entry_id));
    value_id
}

/// Whether a collection literal builds its elements as function values: it
/// holds a function at all. A single one is enough, since a function only ever
/// called by its name leaves nothing in a register for the collection to carry,
/// and what the collection holds is what a program reads back out of it.
fn holds_functions(types: &[DataType]) -> bool {
    types
        .iter()
        .any(|t| matches!(t, DataType::Fn(_) | DataType::FnValue(_)))
}

/// Compiles one element of a collection literal, as a function value where the
/// collection holds functions and as itself everywhere else.
fn compile_element(
    elem: &Expr,
    as_fn_value: bool,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    if as_fn_value {
        compile_fn_value(elem, v, ctx, state, output, None)
    } else {
        elem.compile(v, ctx, state, output, None, false, true)
            .unwrap_id()
    }
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
                state.type_names(),
            )
        }
    }
    // A list that holds functions holds function values: no element is the
    // function the list holds, so each one carries its own identity for a call
    // through the list to dispatch on and for a print to read it by.
    let elem_types = array_items
        .iter()
        .map(|elem| elem.infer_type(v, ctx, state))
        .collect::<Vec<DataType>>();
    let as_fn_values = holds_functions(&elem_types);
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
            let id = compile_element(elem, as_fn_values, v, ctx, state, output);
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
            let id = compile_element(elem, as_fn_values, v, ctx, state, output);
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
            let template_reg = state.template_register(Data::array(array_id as u32));
            // `CloneArray` gives the destination its own pool entry and
            // overwrites the register, so what sits there until then is only a
            // placeholder. It names this literal's own template entry rather
            // than pool slot 0, which belongs to whatever object the program
            // built first: a placeholder aimed at slot 0 makes anything that
            // reads the register file before execution, the debug register
            // dump included, show that object's contents in a register about
            // to hold this array, and keeps slot 0 reachable from the register
            // file for as long as the register is unwritten.
            let dest_reg = {
                state.registers.push(Data::array(array_id as u32));
                (state.registers.len() - 1) as u16
            };
            output.push(Instr::CloneArray(
                template_reg,
                dest_reg,
                state.pools.objs[array_id].len() as u16,
            ));
            dest_reg
        } else {
            // `EmptyArray` allocates the destination's entry, so the same
            // placeholder rule applies. This branch emits no template
            // instruction, but the literal still owns the pool entry reserved
            // for it above, which holds its constant elements and a null per
            // dynamic one: the closest value of the right shape it has, and no
            // root the literal was not already holding.
            let dest_reg = {
                state.registers.push(Data::array(array_id as u32));
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

/// Checks a struct field's value against the type the field declares, and
/// answers whether the value is built as a function value. A field declared
/// `fn(...)` holds a function value, so the function written into it is
/// compiled at the types the field declares, and the value carries which
/// function it is.
#[allow(clippy::too_many_arguments)]
fn compile_struct_field_type(
    name: &SmolStr,
    struct_idx: usize,
    field_idx: usize,
    field_expr: &Expr,
    field_value_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> bool {
    let field_type = field_expr.infer_type(v, ctx, state);
    let declared = state.structs[struct_idx].fields[field_idx].1.clone();
    let as_fn_value = declared_holds_fn_signature(&declared);
    if as_fn_value {
        check_declared_fn(
            field_expr,
            &declared,
            field_value_span,
            output,
            v,
            ctx,
            state,
        );
    }
    let as_fn_value = as_fn_value && matches!(declared, DataType::FnValue(_));
    // A function was just checked against the declared signature, so it
    // stands for that; anything else is checked as the type it is.
    let field_type = if as_fn_value && matches!(field_type, DataType::Fn(_) | DataType::FnValue(_))
    {
        declared
    } else {
        field_type
    };
    let field = &state.structs[struct_idx].fields[field_idx];
    if !struct_field_type_matches(&field.1, &field_type, state.generics) {
        compiler_errors::error_struct_field_invalid_type(
            ctx.file_idx,
            name,
            field.2,
            &field.0,
            &field.1,
            field_value_span,
            &field_type,
            state.sources,
            state.type_names(),
        );
    }
    as_fn_value
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
                let as_fn_value = compile_struct_field_type(
                    name,
                    expected_struct_idx,
                    field_idx,
                    field_expr,
                    *field_value_span,
                    v,
                    ctx,
                    state,
                    output,
                );
                let id = compile_element(field_expr, as_fn_value, v, ctx, state, output);
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
                let as_fn_value = compile_struct_field_type(
                    name,
                    expected_struct_idx,
                    field_idx,
                    field_expr,
                    *field_value_span,
                    v,
                    ctx,
                    state,
                    output,
                );
                let id = compile_element(field_expr, as_fn_value, v, ctx, state, output);
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

        let template_reg =
            state.template_register(Data::struct_instance(type_id, struct_id as u32));
        // `CloneStruct` writes the destination register, so until it runs the
        // register holds a placeholder. It names this literal's own template
        // entry, not pool slot 0, so a read of the register file before
        // execution sees this struct's fields instead of the first object the
        // program built dressed up as one.
        let dest_reg = {
            state
                .registers
                .push(Data::struct_instance(type_id, struct_id as u32));
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
pub(crate) fn resolve_enum_variant(
    path: &[SmolStr],
    file_idx: u16,
    state: &State<'_>,
) -> Option<(u16, u16)> {
    if path.len() >= 2 {
        let variant = &path[path.len() - 1];
        let enum_name = &path[path.len() - 2];
        let module = &path[..path.len() - 2];
        let eid = state.scope(file_idx).find_enum(module, enum_name)?;
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

/// The enum and variant a constructor names, for the lowering.
///
/// A generic enum has no registered type until something names one, so the
/// arguments are typed first and the enum is instantiated at what its payload
/// binds; see [`variant_constructor_at`]. A plain enum resolves by name.
pub(crate) fn variant_constructor(
    path: &[SmolStr],
    args: &[Expr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<(u16, u16)> {
    let arg_types: Vec<DataType> = args.iter().map(|a| a.infer_type(v, ctx, state)).collect();
    variant_constructor_at(path, &arg_types, span, ctx, state)
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
    // A payload is checked the way a function parameter is: an instantiation
    // whose type arguments are `any` lines up with a named one, so a bare
    // payload-less variant (`Leaf`, which pins nothing and is a `Tree<any>`)
    // reaches a slot declared `Tree<int>`.
    for (i, expected) in payload_types.iter().enumerate() {
        let received = args[i].infer_type(v, ctx, state);
        if !param_type_matches(expected, &received, state.generics) {
            compiler_errors::error_function_arg_invalid_type(
                &received,
                expected,
                args_indexes[i],
                &variant_name,
                None,
                ctx.file_idx,
                state.sources,
                state.type_names(),
            );
        }
    }

    let pool_idx = {
        state.pools.objs.push(Vec::with_capacity(arity + 1));
        state.pools.objs.len() - 1
    };
    state
        .pools
        .objs
        .get_mut(pool_idx)
        .push(Data::int(i64::from(variant_idx)));

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

    let template_reg = state.template_register(Data::enum_instance(enum_id, pool_idx as u32));
    // `CloneEnum` allocates the destination's own pool entry and overwrites the
    // register, so what sits there until then is only a placeholder. It aims at
    // the template's entry rather than at pool slot 0, which belongs to
    // whatever object the program built first: a placeholder aimed at slot 0
    // reads that object's first word as this enum's variant tag, which is how
    // the debug register dump came to format an array element as a tag. Aiming
    // at the template makes the placeholder a valid value of the right enum, so
    // anything that reads the register file before execution sees the variant
    // the register is about to hold.
    let dest_reg = {
        state
            .registers
            .push(Data::enum_instance(enum_id, pool_idx as u32));
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
        let template = state.generics.add_enum_template(
            name.clone(),
            type_params.clone(),
            ctx.file_idx,
            Box::from(variants),
        );
        state
            .scope_mut(ctx.file_idx)
            .symbols
            .push((name.clone(), SymbolKind::Template(template)));
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
        .scope_mut(ctx.file_idx)
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
///
/// A qualified pattern resolves through its qualifier, the way a qualified
/// construction does: `Shape::Circle(r)` finds `Shape` in the arm's scope,
/// `sh::Shape::Circle(r)` walks the alias, and `Slot<int>::Empty` instantiates
/// the generic enum. The enum that comes back has to be the scrutinee's, so a
/// pattern qualified with another enum is reported instead of matching the
/// variant of the same name. Only a bare `Circle(r)` is looked up by name in
/// the scrutinee's enum.
pub(crate) fn resolve_variant_pattern(
    enum_id: u16,
    pattern: &Expr,
    fallback_span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> (u16, Vec<SmolStr>) {
    let (path, type_args, binders, span): (&[SmolStr], &[TypeExpr], Vec<SmolStr>, Span) =
        match pattern {
            Expr::Var(name, span) => (std::slice::from_ref(name), &[], Vec::new(), *span),
            Expr::NamespacedRef(path, span, type_args) => (path, type_args, Vec::new(), *span),
            Expr::FunctionCall(args, namespace, span, _, type_args) => {
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
                (&**namespace, type_args, binders, *span)
            }
            _ => compiler_errors::error_enum(
                "Invalid match pattern",
                "A match on an enum expects variant patterns, e.g. Circle(r) or Unit",
                fallback_span,
                ctx.file_idx,
                state.sources,
            ),
        };
    let variant_name = path[path.len() - 1].clone();
    let variant_idx = if path.len() >= 2 {
        let (pattern_enum, variant_idx) = if type_args.is_empty() {
            resolve_qualified_pattern_enum(path, enum_id, span, ctx, state)
        } else {
            resolve_generic_variant(path, type_args, span, ctx, state)
        };
        if pattern_enum != enum_id {
            compiler_errors::error_pattern_enum_mismatch(
                &path.join("::"),
                &state.enums[pattern_enum as usize].name,
                &state.enums[enum_id as usize].name,
                span,
                ctx.file_idx,
                state.sources,
            );
        }
        variant_idx
    } else {
        let e = &state.enums[enum_id as usize];
        let Some(variant_idx) = e.variants.iter().position(|vt| vt.name == variant_name) else {
            compiler_errors::error_enum(
                "Unknown enum variant",
                &format!("{} is not a variant of enum {}", variant_name, e.name),
                span,
                ctx.file_idx,
                state.sources,
            );
        };
        variant_idx as u16
    };
    let e = &state.enums[enum_id as usize];
    let expected_arity = e.variants[variant_idx as usize].payload.len();
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
    (variant_idx, binders)
}

/// The enum and variant a qualified pattern with no type arguments names.
///
/// The segments in front of the variant are the enum name and, before it, the
/// module path it is reached through, so a qualifier that leads nowhere is an
/// unknown enum rather than a pattern silently taken apart by its last segment.
fn resolve_qualified_pattern_enum(
    path: &[SmolStr],
    scrut_enum: u16,
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> (u16, u16) {
    let variant = &path[path.len() - 1];
    let enum_name = &path[path.len() - 2];
    let module = &path[..path.len() - 2];
    let Some(pattern_enum) = state.scope(ctx.file_idx).find_enum(module, enum_name) else {
        compiler_errors::error_enum(
            "Unknown enum",
            &format!(
                "{} does not name an enum in scope, and this match is on enum {}",
                path[..path.len() - 1].join("::"),
                state.enums[scrut_enum as usize].name
            ),
            span,
            ctx.file_idx,
            state.sources,
        );
    };
    let e = &state.enums[pattern_enum];
    let Some(variant_idx) = e.variants.iter().position(|vt| &vt.name == variant) else {
        compiler_errors::error_enum(
            "Unknown enum variant",
            &format!("{} is not a variant of enum {}", variant, e.name),
            span,
            ctx.file_idx,
            state.sources,
        );
    };
    (pattern_enum as u16, variant_idx as u16)
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
        cell: false,
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
                let captured = code_captures_variable(binder, body);
                let binder_reg = if captured {
                    let cell_id = state.alloc_reg();
                    output.push(Instr::NewCell(binder_reg, cell_id));
                    cell_id
                } else {
                    binder_reg
                };
                v.push(Variable {
                    name: binder.clone(),
                    register_id: binder_reg,
                    cell: captured,
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

/// The enum an arm pattern names a variant of, or `None` for a pattern the
/// equality lowering compares as an ordinary value. Resolution follows what
/// the pattern would compile to: a variable in scope wins over a variant of
/// the same name, and a user function wins over a bare variant name.
fn pattern_variant_enum(
    pattern: &Expr,
    v: &[Variable],
    ctx: Ctx,
    state: &State<'_>,
) -> Option<u16> {
    let path: &[SmolStr] = match pattern {
        Expr::Var(name, _) if v.iter().all(|var| &var.name != name) => std::slice::from_ref(name),
        Expr::NamespacedRef(path, _, _) => path,
        Expr::FunctionCall(_, namespace, _, _, _)
            if namespace.len() >= 2
                || namespace.first().is_some_and(|name| {
                    state
                        .scope(ctx.file_idx)
                        .find_function(&[], name.as_str())
                        .is_none()
                }) =>
        {
            namespace
        }
        _ => return None,
    };
    resolve_enum_variant(path, ctx.file_idx, state).map(|(enum_id, _)| enum_id)
}

/// Rejects a `match` whose arms name variants of an enum the scrutinee is not.
/// Only an enum scrutinee carries the variant tag the arms dispatch on; the
/// equality lowering would compile each pattern as a variant construction and
/// compare it, which leaves every payload binder unbound.
///
/// Both the return-flow pass and the lowering call this, because either can be
/// the first to read an arm body.
pub(crate) fn check_match_scrutinee_is_enum(
    scrut_type: &DataType,
    arms: &[(Expr, Box<[Expr]>)],
    span: Span,
    v: &[Variable],
    ctx: Ctx,
    state: &State<'_>,
) {
    if let Some(enum_id) = arms
        .iter()
        .find_map(|(pattern, _)| pattern_variant_enum(pattern, v, ctx, state))
    {
        compiler_errors::error_match_not_enum(
            &state.enums[enum_id as usize].name,
            scrut_type,
            span,
            ctx.file_idx,
            state.sources,
            state.type_names(),
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
    let scrut_type = scrutinee.infer_type(v, ctx, state);
    if let DataType::Enum(enum_id) = scrut_type {
        compile_enum_match(
            enum_id, scrutinee, arms, wildcard, span, v, ctx, state, output,
        );
    } else {
        check_match_scrutinee_is_enum(&scrut_type, arms, span, v, ctx, state);
        let obj_var = SmolStr::new_static("[MATCH TEMP]");
        let (first_pat, first_body) = &arms[0];
        let mut output_code: Vec<Expr> = Vec::with_capacity(arms.len());
        output_code.extend(first_body.iter().cloned());
        for (pat, body) in &arms[1..] {
            output_code.push(Expr::ElseIfBlock(
                Box::new(Expr::Eq(
                    Box::new(Expr::Var(obj_var.clone(), span)),
                    Box::new(pat.clone()),
                    span,
                    span,
                )),
                body.clone(),
                span,
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
                    span,
                    span,
                )),
                Box::from(output_code),
                span,
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
    // A map whose values are functions holds function values, the same as a
    // list does: no value is the function the map holds, so a call through a
    // key dispatches on what came out of it.
    let value_types = kv_pairs
        .iter()
        .map(|(_, _, val, _)| val.infer_type(v, ctx, state))
        .collect::<Vec<DataType>>();
    let as_fn_values = holds_functions(&value_types);
    let map_id = state.pools.maps.len();
    // Entries are inserted in source order and the map keeps that order, so a
    // literal prints the way it was written.
    state.pools.maps.push(CandelaMap::with_capacity_and_hasher(
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
                        state.type_names(),
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
                        state.type_names(),
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
            let id = compile_element(val, as_fn_values, v, ctx, state, output);
            if val.is_constant_literal() {
                map_insert(
                    &mut state.pools.maps[map_id],
                    key_val,
                    state.registers[id as usize],
                    &state.pools.strings,
                );
            } else {
                map_insert(
                    &mut state.pools.maps[map_id],
                    key_val,
                    NULL,
                    &state.pools.strings,
                );
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
                        state.type_names(),
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
                        state.type_names(),
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
            let val_id = compile_element(val, as_fn_values, v, ctx, state, output);
            if val.is_constant_literal() {
                map_insert(
                    &mut state.pools.maps[map_id],
                    key_val,
                    state.registers[val_id as usize],
                    &state.pools.strings,
                );
            } else {
                map_insert(
                    &mut state.pools.maps[map_id],
                    key_val,
                    NULL,
                    &state.pools.strings,
                );
                dynamic.push((key_val, val_id));
            }
        }

        let template_reg = state.template_register(Data::map(map_id as u32));
        // `CloneMap` writes the destination register, so until it runs the
        // register holds a placeholder. It names this literal's own template
        // entry, not map-pool slot 0, which belongs to the first map the
        // program built.
        let dest_reg = {
            state.registers.push(Data::map(map_id as u32));
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
            state.type_names(),
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
        error_type_not_indexable(
            &inferred,
            span,
            false,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }

    let id = array
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    let index_inferred = index.infer_type(v, ctx, state);
    if index_inferred != DataType::Int {
        error_invalid_index_type(
            &index_inferred,
            span,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
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
        error_type_not_indexable(
            &inferred,
            span,
            false,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }
    let id = array
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let idx_start_inferred = idx_start.infer_type(v, ctx, state);
    if idx_start_inferred != DataType::Int {
        error_invalid_index_type(
            &idx_start_inferred,
            span,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }
    let idx_start_id = idx_start
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let idx_end_inferred = idx_end.infer_type(v, ctx, state);
    if idx_end_inferred != DataType::Int {
        error_invalid_index_type(
            &idx_end_inferred,
            span,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
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
        if let Some(id) = compile_operator_method(
            symbol,
            l,
            Some(r),
            &t_l,
            &t_r,
            span_l,
            span_r,
            tgt_id,
            v,
            ctx,
            state,
            output,
        ) {
            return id;
        }
        compiler_errors::error_op(
            &t_l,
            &t_r,
            symbol,
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
            state.type_names(),
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

/// Compiles one of `<`, `<=`, `>`, `>=`.
///
/// Both operands have to be the same type, and that type has to be one the
/// comparison orders: `int`, `float`, or `string`. Strings order by their
/// bytes, which is the order `sort` puts a list of strings in.
fn compile_cmp_op(
    float_instr: fn(u16, u16, u16) -> Instr,
    int_instr: fn(u16, u16, u16) -> Instr,
    str_instr: fn(u16, u16, u16) -> Instr,
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
    if t_l != t_r || !matches!(t_l, DataType::Float | DataType::Int | DataType::String) {
        if let Some(id) = compile_operator_method(
            symbol,
            l,
            Some(r),
            &t_l,
            &t_r,
            span_l,
            span_r,
            tgt_id,
            v,
            ctx,
            state,
            output,
        ) {
            return id;
        }
        compiler_errors::error_op(
            &t_l,
            &t_r,
            symbol,
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
            state.type_names(),
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
    output.push(if t_l == DataType::String {
        str_instr(id_l, id_r, id)
    } else if t_l == DataType::Float {
        float_instr(id_l, id_r, id)
    } else {
        int_instr(id_l, id_r, id)
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
        && let t_l = l.infer_type(v, ctx, state)
        && t_l != DataType::Float
        && !matches!(t_l, DataType::Struct(_) | DataType::Enum(_))
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
        if let Some(id) = compile_operator_method(
            "+",
            l,
            Some(r),
            &t_l,
            &t_r,
            span_l,
            span_r,
            tgt_id,
            v,
            ctx,
            state,
            output,
        ) {
            return id;
        }
        compiler_errors::error_op(
            &t_l,
            &t_r,
            "+",
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
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
        && !src_var.cell
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
        if let Some(id) = compile_operator_method(
            "-",
            l,
            Some(r),
            &t_l,
            &t_r,
            span_l,
            span_r,
            tgt_id,
            v,
            ctx,
            state,
            output,
        ) {
            return id;
        }
        compiler_errors::error_op(
            &t_l,
            &t_r,
            "-",
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }
    // var-1 uses the dedicated DecInt/DecIntTo instructions
    if t_l == DataType::Int
        && matches!(r, Expr::Int(1))
        && let Expr::Var(src_name, _) = l
        && let Some(src_var) = v.iter().rfind(|x| x.name == *src_name)
        && !src_var.cell
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
        && let t_l = l.infer_type(v, ctx, state)
        && t_l != DataType::Float
        && !matches!(t_l, DataType::Struct(_) | DataType::Enum(_))
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
            state.type_names(),
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

/// Whether the two sides of an equality compare by contents.
///
/// A list, a map, a struct and an enum are handles into a pool, so comparing
/// the registers answers whether the two sides are the same object rather than
/// whether they hold the same thing. `ObjEq` walks the contents instead.
const fn compares_by_contents(l_type: &DataType, r_type: &DataType) -> bool {
    const fn is_object(t: &DataType) -> bool {
        matches!(
            t,
            DataType::Array(_) | DataType::Map(_) | DataType::Struct(_) | DataType::Enum(_)
        )
    }
    is_object(l_type) && is_object(r_type)
}

fn compile_eq_op(
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
    let l_type = l.infer_type(v, ctx, state);
    let r_type = r.infer_type(v, ctx, state);
    // A type that defines `==` answers for its own values; without one, two
    // structs still compare field by field, which is what they always did.
    if let Some(id) = compile_operator_method(
        "==",
        l,
        Some(r),
        &l_type,
        &r_type,
        span_l,
        span_r,
        tgt_id,
        v,
        ctx,
        state,
        output,
    ) {
        return id;
    }
    let by_contents = compares_by_contents(&l_type, &r_type);
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
    output.push(if by_contents {
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
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let l_type = l.infer_type(v, ctx, state);
    let r_type = r.infer_type(v, ctx, state);
    // `!=` has no method of its own: it is the type's `==` with the answer
    // flipped.
    if let Some(id) = compile_operator_method(
        "!=",
        l,
        Some(r),
        &l_type,
        &r_type,
        span_l,
        span_r,
        tgt_id,
        v,
        ctx,
        state,
        output,
    ) {
        return id;
    }
    let by_contents = compares_by_contents(&l_type, &r_type);
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
    if by_contents {
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
    if let Some(id) = compile_operator_method(
        UNARY_MINUS_METHOD,
        l,
        None,
        &operand_type,
        &operand_type,
        span_l,
        span_r,
        tgt_id,
        v,
        ctx,
        state,
        output,
    ) {
        return id;
    }
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
            state.type_names(),
        );
    }
    id
}

/// Compiles an operator whose operands are both `int` and nothing else: the
/// bitwise operators and the shifts.
///
/// [`uniform_op2`] covers the operators that take a pair of `int` or a pair of
/// `float`; this one has a single accepted pair, so a `float` operand reaches
/// the same report a `string` one does.
#[allow(clippy::too_many_arguments)]
fn int_op2(
    instr: fn(u16, u16, u16) -> Instr,
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
    if t_l != DataType::Int || t_r != DataType::Int {
        if let Some(id) = compile_operator_method(
            symbol,
            l,
            Some(r),
            &t_l,
            &t_r,
            span_l,
            span_r,
            tgt_id,
            v,
            ctx,
            state,
            output,
        ) {
            return id;
        }
        compiler_errors::error_op(
            &t_l,
            &t_r,
            symbol,
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
            state.type_names(),
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
    output.push(instr(id_l, id_r, id));
    id
}

/// Compiles `<<` or `>>`, refusing a count written as a literal that `int` has
/// no room for. A count only known while the program runs is checked by the
/// instruction instead, which raises `shift_count_out_of_range`.
#[allow(clippy::too_many_arguments)]
fn compile_shift_op(
    instr: fn(u16, u16, u16) -> Instr,
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
    if let Expr::Int(count) = r
        && !shift_count_in_range(*count)
        && !matches!(
            l.infer_type(v, ctx, state),
            DataType::Struct(_) | DataType::Enum(_)
        )
    {
        error_shift_count_out_of_range(*count, span_l.extend(span_r), ctx.file_idx, state.sources);
    }
    int_op2(
        instr, symbol, l, r, span_l, span_r, tgt_id, v, ctx, state, output,
    )
}

/// Compiles `~`, which flips the bits of an `int`.
fn compile_bit_not_op(
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
    if let Some(id) = compile_operator_method(
        "~",
        l,
        None,
        &operand_type,
        &operand_type,
        span_l,
        span_r,
        tgt_id,
        v,
        ctx,
        state,
        output,
    ) {
        return id;
    }
    let id_l = l
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id_l, v);
    let id = state.alloc_reg_tgt(tgt_id);
    if operand_type != DataType::Int {
        compiler_errors::error_op(
            &DataType::Null,
            &operand_type,
            "~",
            span_l,
            span_r,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }
    output.push(Instr::BitNotInt(id_l, id));
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
            state.type_names(),
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
    condition_span: Span,
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
        .position(|x| matches!(x, Expr::ElseIfBlock(_, _, _) | Expr::ElseBlock(_)))
        .unwrap_or(code.len());

    let condition_blocks_count = code.len() - main_code_limit;
    let mut cmp_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);
    let mut jmp_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);
    let mut condition_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);

    // parse the main condition
    let condition_id =
        compile_condition_operand(main_condition, condition_span, v, ctx, state, output);
    add_cmp_false(condition_id, &mut 0, output, false);
    cmp_markers.push(output.len() - 1);

    compile_inline_condition_branch(&code[..main_code_limit], v, ctx, state, output, return_id);
    if main_code_limit != code.len() {
        output.push(Instr::Jmp(0));
        jmp_markers.push(output.len() - 1);
    }

    let mut else_exists = false;
    for elem in &code[main_code_limit..] {
        if let Expr::ElseIfBlock(condition, code, condition_span) = elem {
            condition_markers.push(output.len());
            let condition_id =
                compile_condition_operand(condition, *condition_span, v, ctx, state, output);
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
        error_type_not_indexable(
            &array_type,
            index_span,
            false,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
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
            state.type_names(),
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
            state.type_names(),
        );
    };
    let field_struct = &state.structs[struct_id as usize];
    let Some(field_index) = field_struct
        .fields
        .iter()
        .position(|(expected_field_name, _, _)| expected_field_name == field)
    else {
        compiler_errors::error_struct_unknown_field(
            ctx.file_idx,
            field_span,
            field,
            &field_struct.name,
            &field_struct.fields,
            state.sources,
        );
    };
    let struct_name = field_struct.name.clone();
    // The value is checked against the field the way a struct literal checks
    // it, so a field that holds a function takes what the literal would.
    let as_fn_value = compile_struct_field_type(
        &struct_name,
        struct_id as usize,
        field_index,
        new_val,
        value_span,
        v,
        ctx,
        state,
        output,
    );
    let field_index = field_index as u16;
    let id = struct_expr
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    // `s.f += x` on a float field reads, adds and writes the field in one
    // instruction. The field is read after `x` is worked out rather than
    // before, which only a call inside `x` could tell apart, so `x` may hold
    // none.
    if let Expr::Add(read, added, _, _) = new_val
        && let (Expr::GetStructField(read_struct, read_field, _, _), Expr::Var(name, _)) =
            (read.as_ref(), struct_expr)
        && read_field == field
        && matches!(read_struct.as_ref(), Expr::Var(read_name, _) if read_name == name)
        && state.structs[struct_id as usize].fields[field_index as usize].1 == DataType::Float
        && new_val_type == DataType::Float
        && calls_nothing(added, v, ctx, state)
    {
        let added_id = added
            .compile(v, ctx, state, output, None, false, true)
            .unwrap_id();
        state.free_reg(added_id, v);
        output.push(Instr::AddFieldFloat(id, added_id, field_index));
        return;
    }
    let new_elem_reg_id = compile_element(new_val, as_fn_value, v, ctx, state, output);
    output.push(Instr::SetFieldStruct(id, new_elem_reg_id, field_index));
}

/// Whether `expr` is arithmetic on numbers, names, literals and field reads
/// alone, so working it out runs no candela code and writes nothing.
fn calls_nothing(expr: &Expr, v: &mut Vec<Variable>, ctx: Ctx, state: &mut State<'_>) -> bool {
    match expr {
        Expr::Var(..) | Expr::Float(_) | Expr::Int(_) => true,
        Expr::GetStructField(inner, _, _, _) => calls_nothing(inner, v, ctx, state),
        Expr::Add(l, r, _, _)
        | Expr::Sub(l, r, _, _)
        | Expr::Mul(l, r, _, _)
        | Expr::Div(l, r, _, _) => {
            // An operator on anything but numbers can be a method written in
            // candela.
            [l, r].into_iter().all(|side| {
                matches!(
                    side.infer_type(v, ctx, state),
                    DataType::Float | DataType::Int
                ) && calls_nothing(side, v, ctx, state)
            })
        }
        _ => false,
    }
}

fn compile_condition(
    main_condition: &Expr,
    code: &[Expr],
    condition_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    // get first code limit (after which there are only else(if) blocks)
    let main_code_limit = code
        .iter()
        .position(|x| matches!(x, Expr::ElseIfBlock(_, _, _) | Expr::ElseBlock(_)))
        .unwrap_or(code.len());

    let condition_blocks_count = code.len() - main_code_limit;
    // Each entry is the list of false-jump instruction indices for one condition block.
    let mut conditional_false_jmp_idxs: Vec<Vec<usize>> =
        Vec::with_capacity(condition_blocks_count + 1);
    let mut jmp_instr_idx: Vec<usize> = Vec::with_capacity(condition_blocks_count);
    let mut condition_markers: Vec<usize> = Vec::with_capacity(condition_blocks_count);

    // Compile the main condition
    let (true_jump_idxs, false_jump_idxs) = compile_short_circuit_condition(
        main_condition,
        condition_span,
        v,
        ctx,
        state,
        output,
        false,
    );
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
        if let Expr::ElseIfBlock(condition, code, condition_span) = elem {
            condition_markers.push(output.len());
            let condition_id =
                compile_condition_operand(condition, *condition_span, v, ctx, state, output);
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
    condition_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let output_len_before = output.len();

    let (true_jump_idxs, false_jump_idxs) =
        compile_short_circuit_condition(condition, condition_span, v, ctx, state, output, false);

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
    if state.optimize && compile_slice_for_loop(var_name, array, code, span, v, ctx, state, output)
    {
        return;
    }

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
    compile_walk(
        var_name,
        &array_type,
        array,
        index_id,
        array_len_id,
        code,
        span,
        v,
        ctx,
        state,
        output,
    );
}

/// Compiles `for x in list[start..end]` to walk `list` itself from `start` to
/// `end`, without building the slice, and answers whether it did. A release
/// profile pass.
///
/// A slice is a copy, so the loop sees the elements as they were when it
/// began. Walking the list itself sees the same elements as long as nothing
/// rewrites one of them, removes one or reorders them during the loop, so this
/// is done only for a body that can do none of that to any list: it runs no
/// candela code (no call, no method but `push`, and no operator a program
/// could have written as a method) and writes no element of a list. A `push`
/// adds past the end of a list and moves nothing, so the elements the walk
/// reads stay put even when it pushes onto the list being walked. It pushes
/// only onto variables declared outside the loop, whose type says they hold a
/// list and not a struct with a `push` method of its own.
///
/// A range the list cannot supply sends the loop down the path that builds
/// the slice, which raises the out-of-range error the slice always raised,
/// from the same place.
fn compile_slice_for_loop(
    var_name: &SmolStr,
    array: &Expr,
    code: &[Expr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> bool {
    let Expr::ArrayGetSlice(list, start, end, slice_span) = array else {
        return false;
    };
    // An operator on a struct or an enum can be a method written in candela,
    // which the body could not be seen not to call without typing it.
    if state.generics.defines_operator_methods() {
        return false;
    }
    let mut receivers: Vec<SmolStr> = Vec::new();
    let mut declared: Vec<SmolStr> = vec![var_name.clone()];
    if !block_changes_only_pushed_lists(code, &mut receivers, &mut declared) {
        return false;
    }
    for name in &receivers {
        // A name the body declares is not the variable found out here.
        if declared.contains(name)
            || !v
                .iter()
                .rfind(|var| var.name == *name)
                .is_some_and(|var| matches!(var.var_type, DataType::Array(_)))
        {
            return false;
        }
    }
    let array_type = list.infer_type(v, ctx, state);
    if !matches!(array_type, DataType::Array(_))
        || start.infer_type(v, ctx, state) != DataType::Int
        || end.infer_type(v, ctx, state) != DataType::Int
    {
        return false;
    }

    let list_id = list
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let start_id = start
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let end_id = end
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    // The walk reads the list, the index and the end out of registers of its
    // own, so a body that reassigns a variable they came from changes nothing.
    let walked = state.alloc_reg();
    let index_id = state.alloc_reg();
    let end_at = state.alloc_reg();
    let zero = state.const_int_register(0);

    // In range means 0 <= start <= end <= len. Each test jumps to the path
    // that builds the slice, patched once its place is known.
    let mut to_copy: Vec<usize> = Vec::new();
    output.push(Instr::CallLibFunc(LibFunc::Len, list_id, end_at));
    to_copy.push(output.len());
    output.push(Instr::InfIntJmp(start_id, zero, 0));
    to_copy.push(output.len());
    output.push(Instr::SupIntJmp(end_id, end_at, 0));
    to_copy.push(output.len());
    output.push(Instr::SupIntJmp(start_id, end_id, 0));
    output.push(Instr::Mov(list_id, walked));
    output.push(Instr::Mov(start_id, index_id));
    output.push(Instr::Mov(end_id, end_at));
    let skip_copy = output.len();
    output.push(Instr::Jmp(0));

    let copy_at = output.len();
    for at in to_copy {
        set_jmp_size(&mut output[at], (copy_at - at) as u16);
    }
    output.push(Instr::StoreFuncArg(end_id));
    output.push(Instr::GetSliceArray(list_id, start_id, walked));
    state.add_to_src(ctx, output, *slice_span);
    output.push(Instr::SetInt(index_id, 0));
    output.push(Instr::CallLibFunc(LibFunc::Len, walked, end_at));
    let copy_len = (output.len() - skip_copy) as u16;
    set_jmp_size(&mut output[skip_copy], copy_len);

    state.free_reg(start_id, v);
    state.free_reg(end_id, v);
    compile_walk(
        var_name,
        &array_type,
        walked,
        index_id,
        end_at,
        code,
        span,
        v,
        ctx,
        state,
        output,
    );
    true
}

/// Whether running `code` can change no list but by `push` onto a variable,
/// collecting those variables in `receivers` and every name the code declares
/// in `declared`. See [`compile_slice_for_loop`].
fn block_changes_only_pushed_lists(
    code: &[Expr],
    receivers: &mut Vec<SmolStr>,
    declared: &mut Vec<SmolStr>,
) -> bool {
    code.iter()
        .all(|expr| changes_only_pushed_lists(expr, receivers, declared))
}

/// [`block_changes_only_pushed_lists`] for one expression or statement.
fn changes_only_pushed_lists(
    expr: &Expr,
    receivers: &mut Vec<SmolStr>,
    declared: &mut Vec<SmolStr>,
) -> bool {
    match expr {
        Expr::Var(..)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Bool(_)
        | Expr::String(_)
        | Expr::Null
        | Expr::Break
        | Expr::Continue => true,
        Expr::GetStructField(inner, _, _, _)
        | Expr::BoolNeg(inner, _, _)
        | Expr::Neg(inner, _, _)
        | Expr::BitNot(inner, _, _) => changes_only_pushed_lists(inner, receivers, declared),
        Expr::ArrayGetIndex(l, r, _)
        | Expr::Mul(l, r, _, _)
        | Expr::Div(l, r, _, _)
        | Expr::Add(l, r, _, _)
        | Expr::Sub(l, r, _, _)
        | Expr::Mod(l, r, _, _)
        | Expr::Pow(l, r, _, _)
        | Expr::BitAnd(l, r, _, _)
        | Expr::BitOr(l, r, _, _)
        | Expr::BitXor(l, r, _, _)
        | Expr::Shl(l, r, _, _)
        | Expr::Shr(l, r, _, _)
        | Expr::Eq(l, r, _, _)
        | Expr::NotEq(l, r, _, _)
        | Expr::Sup(l, r, _, _)
        | Expr::SupEq(l, r, _, _)
        | Expr::Inf(l, r, _, _)
        | Expr::InfEq(l, r, _, _)
        | Expr::BoolAnd(l, r, _, _)
        | Expr::BoolOr(l, r, _, _) => {
            changes_only_pushed_lists(l, receivers, declared)
                && changes_only_pushed_lists(r, receivers, declared)
        }
        Expr::VarDeclare(name, value) => {
            declared.push(name.clone());
            changes_only_pushed_lists(value, receivers, declared)
        }
        Expr::VarAssign(_, value, _) => changes_only_pushed_lists(value, receivers, declared),
        Expr::ReturnVal(value) => value
            .as_ref()
            .as_ref()
            .is_none_or(|value| changes_only_pushed_lists(value, receivers, declared)),
        Expr::ObjFunctionCall(obj, args, namespace, _, _, _, type_args) => {
            let Expr::Var(name, _) = obj.as_ref() else {
                return false;
            };
            if !matches!(&**namespace, [method] if method == "push")
                || !type_args.is_empty()
                || !args
                    .iter()
                    .all(|arg| changes_only_pushed_lists(arg, receivers, declared))
            {
                return false;
            }
            if !receivers.contains(name) {
                receivers.push(name.clone());
            }
            true
        }
        Expr::Condition(condition, body, _, _)
        | Expr::ElseIfBlock(condition, body, _)
        | Expr::WhileBlock(condition, body, _) => {
            changes_only_pushed_lists(condition, receivers, declared)
                && block_changes_only_pushed_lists(body, receivers, declared)
        }
        Expr::ElseBlock(body) | Expr::EvalBlock(body) | Expr::LoopBlock(body) => {
            block_changes_only_pushed_lists(body, receivers, declared)
        }
        _ => false,
    }
}

/// Compiles the turns of a `for` loop over a list, a string or a map's keys:
/// the loop variable takes `array[index]` for each `index` from where
/// `index_id` starts up to the `int` in `end_id`, and `code` runs for each.
fn compile_walk(
    var_name: &SmolStr,
    array_type: &DataType,
    array: u16,
    index_id: u16,
    array_len_id: u16,
    code: &[Expr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let real_var = var_name.as_str() != "_";

    // set up the variable for the current element (for current_element_id in ... {}) => current_element_id = array[index]
    let current_element_id = if real_var { state.alloc_reg() } else { 0 };

    // A closure written in the body takes the element of the turn it was
    // written on, so the cell is allocated inside the loop and the body reads
    // the variable through it.
    let captured = real_var && code_captures_variable(var_name, code);
    let element_cell_id = if captured { state.alloc_reg() } else { 0 };

    let v_len = v.len();

    let is_str = *array_type == DataType::String;

    if real_var {
        v.push(Variable {
            name: var_name.clone(),
            register_id: if captured {
                element_cell_id
            } else {
                current_element_id
            },
            cell: captured,
            var_type: match array_type {
                DataType::String => DataType::String,
                DataType::Array(a_type) => a_type.clone().map_or(DataType::Null, |t| *t),
                // A map iterates its keys; the loop variable is a key.
                DataType::Map(m) => m.0.clone().unwrap_or(DataType::Unknown),
                t => {
                    error_type_not_indexable(
                        t,
                        span,
                        true,
                        ctx.file_idx,
                        state.sources,
                        state.type_names(),
                    );
                }
            },
        });
    }
    let loop_id = ctx.block_id + 1;

    // accounts for the GetIndexArray/GetIndexString instruction and, where the
    // body captures the element, the cell it goes into
    let pending = u16::from(real_var) + u16::from(captured);

    // The loop is laid out like a counted range:
    // ----
    // (1) if i >= len jump out
    // (2) element = array[i], then the body
    // (3) i += 1, and if i < len jump back to (2)
    // ----
    // so a turn runs one test rather than a test at the top and a jump back
    // to it. The length is read once, before the first turn, either way.
    let jmp_idx = output.len();
    output.push(Instr::SupEqIntJmp(index_id, array_len_id, 0));

    let regs_before = state.registers.len() as u16;
    let body = compile_expr(
        code,
        v,
        ctx.no_single_run()
            .advance_offset(output.len() as u16 + pending),
        state,
    );
    // Clean up variables
    v.truncate(v_len);
    state.free_scope_registers(regs_before, &body, v);
    let body_len = body.len() as u16;

    // load the element's value into the current_element_id register
    if real_var {
        if is_str {
            output.push(Instr::GetIndexString(array, index_id, current_element_id));
            // A body that assigns a shorter string to what is being walked
            // leaves the index past its end, so this is the one loop whose
            // element read can raise: the span points the report at it.
            state.add_to_src(ctx, output, span);
        } else {
            output.push(Instr::GetIndexArray(array, index_id, current_element_id));
        }
        if captured {
            output.push(Instr::NewCell(current_element_id, element_cell_id));
        }
    }
    output.extend(body);
    // step the index and jump back to the element read while elements remain
    output.push(Instr::StepIntJmpBack(
        index_id,
        array_len_id,
        body_len + pending,
    ));

    let exit_size = (output.len() - jmp_idx) as u16;
    output[jmp_idx] = Instr::SupEqIntJmp(index_id, array_len_id, exit_size);
    parse_loop_flow_control(&mut output[jmp_idx + 1..], loop_id, exit_size, true, false);

    if ctx.single_run {
        state.free_reg(array_len_id, v);
        state.free_reg(index_id, v);
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
    // (3) i += 1, and if i < end_elem jump back to body
    // ----
    //
    //
    // Check start and elem type
    let t1 = start_elem.infer_type(v, ctx, state);
    let t2 = end_elem.infer_type(v, ctx, state);
    if t1 != DataType::Int {
        error_range_invalid_type(span1, &t1, ctx.file_idx, state.sources, state.type_names());
    }
    if t2 != DataType::Int {
        error_range_invalid_type(span2, &t2, ctx.file_idx, state.sources, state.type_names());
    }
    // The counter is always its own register, filled from the start
    // expression. Counting in the start's register was fine for a loop that
    // ran once, until the register was shared: a literal start is the
    // constant every earlier-compiled function reads for that literal, and a
    // variable start is the variable itself, so the loop would advance both.
    let output_len = output.len();
    let start_elem_id = start_elem
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let start_val = state.registers[start_elem_id as usize];
    let elem_id = state.alloc_reg();
    // An `int` wider than the immediate operand comes out of its register.
    if let Some(n) = int_immediate(start_val)
        && state.const_registers.values().any(|&v| v == start_elem_id)
    {
        output.push(Instr::SetInt(elem_id, n));
    } else if !write_scratch_into(output, output_len, start_elem_id, elem_id, v, state) {
        output.push(Instr::Mov(start_elem_id, elem_id));
    }
    let end_elem_id = end_elem
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    // A closure written in the body takes the counter of the turn it was
    // written on, so the cell is filled at the top of each turn.
    let captured = code_captures_variable(var_name, code);
    let elem_cell_id = if captured { state.alloc_reg() } else { 0 };

    let v_len = v.len();
    v.push(Variable {
        name: var_name.clone(),
        register_id: if captured { elem_cell_id } else { elem_id },
        cell: captured,
        var_type: DataType::Int,
    });
    let loop_id = ctx.block_id + 1;

    // (1) if i >= end_elem jump out -> push placeholder first so that compile_expr sees the correct offset
    let jmp_idx = output.len();
    output.push(Instr::SupEqIntJmp(elem_id, end_elem_id, 0));
    if captured {
        output.push(Instr::NewCell(elem_id, elem_cell_id));
    }

    let regs_before = state.registers.len() as u16;
    let compiled_loop_code = compile_expr(
        code,
        v,
        ctx.no_single_run().advance_offset(output.len() as u16),
        state,
    );
    state.free_scope_registers(regs_before, &compiled_loop_code, v);
    let compiled_loop_code_len = compiled_loop_code.len() as u16;

    // (2) loop_body
    output.extend(compiled_loop_code);

    // (3) i += 1, and if i < end_elem jump back to the top of the turn,
    // which is the cell where a body that captures the counter reads it from
    output.push(Instr::StepIntJmpBack(
        elem_id,
        end_elem_id,
        compiled_loop_code_len + u16::from(captured),
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
    state.free_scope_registers(regs_before, &compiled, v);
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
    // The handler starts where the catch jumps to, so a cell for a caught
    // error a closure reads is filled there, once per error caught.
    let captured = code_captures_variable(err_var, catch_code);
    let err_cell_id = if captured { state.alloc_reg() } else { 0 };
    v.push(Variable {
        name: err_var.clone(),
        register_id: if captured { err_cell_id } else { err_reg_id },
        cell: captured,
        var_type: DataType::String,
    });
    output[err_catch_instr] =
        Instr::StartErrorCatch((output.len() - err_catch_instr) as u16, err_reg_id);
    if captured {
        output.push(Instr::NewCell(err_reg_id, err_cell_id));
    }
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

/// The register holding the cell of a variable a closure captures.
///
/// Every binding a closure can reach puts a captured variable in a cell where
/// it is bound, so that each turn around a loop and each call of the declaring
/// function gets its own. A binding the analysis did not reach is moved into a
/// cell here instead, where the closure is written.
fn capture_cell(
    name: &SmolStr,
    span: Span,
    v: &mut [Variable],
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let Some(pos) = v.iter().rposition(|var| var.name == *name) else {
        compiler_errors::error_unknown_variable(name, span, v, ctx.file_idx, state.sources);
    };
    if v[pos].cell {
        return v[pos].register_id;
    }
    let cell_id = state.alloc_reg();
    output.push(Instr::NewCell(v[pos].register_id, cell_id));
    v[pos].register_id = cell_id;
    v[pos].cell = true;
    cell_id
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
    // A variable a later assignment writes a function into holds a function
    // value: the name no longer says which function, so a call through it
    // dispatches on what the variable holds by then. The assignment is looked
    // for in the code, not in the types, because a closure written further down
    // reads the scope where it is written, not the scope here.
    let var_type = match &var_type {
        DataType::Fn(fn_id) if code_modifies_variable(name, remaining_code) => {
            DataType::FnValue(Box::new(FnValue::from_set(state.new_fn_value_set(*fn_id))))
        }
        _ => var_type,
    };
    // A `let` has no annotation to read a function against, so the binding
    // takes the function's own signature: the name stops naming one call target
    // and holds a value the call dispatches on.
    let var_type = if matches!(var_type, DataType::Fn(_)) {
        bare_fn_value_type(value, v, ctx, state).unwrap_or(var_type)
    } else {
        var_type
    };
    let as_fn_value = matches!(var_type, DataType::FnValue(_));
    // A variable a closure reads lives in a cell instead of a register, so the
    // scope that declared it and the closures that took it share one slot. The
    // cell is allocated where the declaration runs, which gives each turn
    // around a loop and each call of the declaring function its own.
    let captured = code_captures_variable(name, remaining_code);

    let output_len = output.len();
    let src_id = if as_fn_value {
        compile_fn_value(value, v, ctx, state, output, None)
    } else {
        value
            .compile(v, ctx, state, output, None, ctx.single_run, true)
            .unwrap_id()
    };
    // The value can come back in a register something else owns: another
    // variable's, or a constant's. The two names may share it only while
    // neither is written, or writing one would change the other.
    let owned_elsewhere = v.iter().any(|var| var.register_id == src_id)
        || state.const_registers.values().any(|&reg| reg == src_id);
    let owner_written = v
        .iter()
        .filter(|var| var.register_id == src_id)
        .any(|var| code_modifies_variable(&var.name, remaining_code));
    let written = code_modifies_variable(name, remaining_code);
    // Code that runs once gives a literal a register of its own, so only a
    // shared register needs a copy there.
    let needs_own =
        !captured && (owner_written || (written && (owned_elsewhere || !ctx.single_run)));
    let var_id = if needs_own {
        let mutable_id = state.alloc_reg();
        if !write_scratch_into(output, output_len, src_id, mutable_id, v, state) {
            move_reg_to_reg(output, src_id, mutable_id, state.registers[src_id as usize]);
        }
        mutable_id
    } else {
        src_id
    };
    let var_id = if captured {
        let cell_id = state.alloc_reg();
        output.push(Instr::NewCell(var_id, cell_id));
        state.free_reg(var_id, v);
        cell_id
    } else {
        var_id
    };

    if let DataType::Fn(fn_id) = &var_type {
        let fn_id = *fn_id as usize;
        // A closure has no name of its own, so the one it is bound to is the
        // one its body calls itself by. The binding is what makes that call
        // reach the closure being built rather than whatever else the name
        // resolved to, and what makes it a recursive call, which is what saves
        // the registers the closure still reads once it returns.
        if state.fns[fn_id].direct_calls.contains(name) {
            state.fns[fn_id].is_recursive = Some(true);
        }
        let fn_symbol = SymbolKind::Fn(fn_id as u16);
        state
            .scope_mut(ctx.file_idx)
            .symbols
            .push((name.clone(), fn_symbol));
    }
    v.push(Variable {
        name: name.clone(),
        register_id: var_id,
        cell: captured,
        var_type,
    });
}

/// Adds `fn_id` to the set a variable of type `held` dispatches over, and
/// compiles it for every call already made through that set.
fn join_fn_value_set(
    held: &FnValue,
    fn_id: u16,
    span: Span,
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) {
    let Some(set_idx) = held.set else {
        return;
    };
    let Some(set) = state
        .indirect_registers
        .value_sets
        .get_mut(set_idx as usize)
    else {
        return;
    };
    if set.candidates.contains(&fn_id) {
        return;
    }
    set.candidates.push(fn_id);
    let used_at = set.used_at.clone();
    for arg_types in used_at {
        ensure_indirect_impl(fn_id as usize, &arg_types, span, output, v, ctx, state);
    }
}

/// Makes the instructions from `from` on write their value into `dest` instead
/// of `scratch`, so it does not take a move to get there, and answers whether
/// it did.
///
/// It does when `scratch` holds nothing but that value: the last of those
/// instructions writes it, and it is not a variable's register, a constant or
/// a register the compiler keeps for itself. A variable written more than once
/// needs a register of its own, and this is how its first value lands there
/// directly. `scratch` is free again afterwards.
fn write_scratch_into(
    output: &mut [Instr],
    from: usize,
    scratch: u16,
    dest: u16,
    v: &[Variable],
    state: &mut State<'_>,
) -> bool {
    let fresh = output.len() > from
        && output[from..]
            .iter()
            .rev()
            .find_map(|instr| instr.get_tgt_id())
            == Some(scratch)
        && !v.iter().any(|var| var.register_id == scratch)
        && !state.const_registers.values().any(|&reg| reg == scratch)
        && !state.reserved_registers.contains(&scratch);
    if !fresh || !move_to_id(&mut output[from..], dest) {
        return false;
    }
    state.free_reg(scratch, v);
    true
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

    let var_type = match (&v[var_pos].var_type, &var_type) {
        (DataType::FnValue(held), DataType::Fn(fn_id)) if held.sig.is_none() => {
            let held = held.clone();
            join_fn_value_set(&held, *fn_id, span, output, v, ctx, state);
            DataType::FnValue(held)
        }
        _ => var_type,
    };
    let as_fn_value = matches!(var_type, DataType::FnValue(_));

    // A captured variable is written through its cell, which is the slot the
    // closures that took it read.
    if v[var_pos].cell {
        let value_id = if as_fn_value {
            compile_fn_value(value, v, ctx, state, output, None)
        } else {
            value
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id()
        };
        output.push(Instr::StoreCell(id, value_id));
        state.free_reg(value_id, v);
        v[var_pos].var_type = var_type;
        return;
    }

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
                            .filter(|x| x.var_type == DataType::Int && !x.cell)
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
                            .filter(|x| x.var_type == DataType::Int && !x.cell)
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
        if !move_to_id(output, id) && obj_id != id {
            output.push(Instr::Mov(obj_id, id));
        }
    } else if state.const_registers.values().any(|&v| v == obj_id) {
        move_reg_to_reg(output, obj_id, id, state.registers[obj_id as usize]);
    } else if obj_id != id {
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
        let template = state.generics.add_struct_template(
            name.clone(),
            type_params.clone(),
            ctx.file_idx,
            Box::from(fields),
        );
        state
            .scope_mut(ctx.file_idx)
            .symbols
            .push((name.clone(), SymbolKind::Template(template)));
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
    let struct_symbol = SymbolKind::Struct((state.structs.len() - 1) as u16);
    state
        .scope_mut(ctx.file_idx)
        .symbols
        .push((name.clone(), struct_symbol));
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
    collect_direct_fn_calls(fn_code, self_type_of(fn_name), &mut callees);
    let fn_symbol = SymbolKind::Fn(state.fns.len() as u16);
    state
        .scope_mut(ctx.file_idx)
        .symbols
        .push((fn_name.clone(), fn_symbol));
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
        captures: Box::from([]),
        entry_register: None,
    });
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
    let mut id = x
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    // A value typed `any` leaving a function declared to return a concrete type
    // is checked here, the one place it can be: the call site reads the
    // declared type and compiles against it. The wrong value raises the
    // catchable `bad_downcast`.
    if let Some(check) = ctx.return_downcast
        && matches!(x.infer_type(v, ctx, state), DataType::Unknown)
    {
        state.free_reg(id, v);
        let checked_id = state.alloc_reg();
        match check {
            ReturnCheck::Builtin(downcast) => {
                output.push(Instr::CallLibFunc(downcast, id, checked_id));
            }
            ReturnCheck::Types(codes) => {
                let codes = state.type_codes_register(codes.as_slice());
                output.push(Instr::StoreFuncArg(codes));
                output.push(Instr::CallLibFunc(LibFunc::AsTypeVal, id, checked_id));
            }
            ReturnCheck::TypesIn(codes) => {
                output.push(Instr::StoreFuncArg(codes));
                output.push(Instr::CallLibFunc(LibFunc::AsTypeVal, id, checked_id));
            }
        }
        id = checked_id;
    }
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

/// The type a function's body compiles against, read from its own name: a
/// method is registered under the mangled `Type#method`, and an ordinary
/// function's name holds no separator.
fn self_type_of(fn_name: &str) -> Option<&str> {
    fn_name
        .split_once(METHOD_SEP)
        .map(|(type_name, _)| type_name)
}

pub fn compile_expr(
    input: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<Instr> {
    let v_len = v.len();
    let symbols_len = state.scope(ctx.file_idx).symbols.len();
    let mut output: Vec<Instr> = Vec::with_capacity(input.len());
    for (idx, x) in input.iter().enumerate() {
        // Nothing reads the value of a statement, except where the form has no
        // statement arm to compile through: `x;` and `1 + 2;` are compiled as
        // values, and the register is freed again on the next line.
        if let Some(id) = x.compile_with_code_context(
            v,
            ctx,
            state,
            &mut output,
            None,
            false,
            &input[idx + 1..],
            x.is_value_only(),
        ) {
            state.free_reg(id, v);
        }
    }
    v.truncate(v_len);
    // The block's variables and the names it declared go out of scope with it.
    // The function table does not: an entry in it is named by index, by the
    // bytecode that calls it and by the `DataType::Fn` a closure's type is, and
    // an index nothing can hand back out is an index nothing else can be given.
    // A closure is registered under its own source span, so the same body
    // compiled again for a second specialisation finds the entry it made the
    // first time instead of making a second one.
    state.scope_mut(ctx.file_idx).symbols.truncate(symbols_len);
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
                if let Some(var) = v.iter().rfind(|v_temp| *name == v_temp.name) {
                    if var.cell {
                        // A captured variable holds a cell; its value is what
                        // the cell holds now, which a closure may have written.
                        let cell = var.register_id;
                        let dest = state.alloc_reg_tgt(tgt_id);
                        output.push(Instr::LoadCell(cell, dest));
                        Some(dest)
                    } else if let DataType::Fn(fn_id) = var.var_type
                        && state.fns[fn_id as usize].captures.is_empty()
                    {
                        // A parameter the call settled on one function that
                        // captures nothing is passed nothing, so reading it as
                        // a value builds that function's value.
                        Some(compile_fn_entry_value(
                            fn_id as usize,
                            v,
                            ctx,
                            state,
                            output,
                            tgt_id,
                        ))
                    } else {
                        Some(var.register_id)
                    }
                } else if let Some(fn_id) = state.scope(ctx.file_idx).find_function(&[], name) {
                    // A bare name that names a function is the function itself
                    // wherever it is read rather than called, so a `let`, a
                    // return, a collection element and an argument all hand on
                    // a value a later call can dispatch on.
                    Some(compile_fn_entry_value(fn_id, v, ctx, state, output, tgt_id))
                } else if let Some((enum_id, variant_idx)) =
                    variant_constructor(std::slice::from_ref(name), &[], *span, v, ctx, state)
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
            Self::BitAnd(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(int_op2(
                    Instr::BitAndInt,
                    "&",
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
            Self::BitOr(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(int_op2(
                    Instr::BitOrInt,
                    "|",
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
            Self::BitXor(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(int_op2(
                    Instr::BitXorInt,
                    "^^",
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
            Self::Shl(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_shift_op(
                    Instr::ShlInt,
                    "<<",
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
            Self::Shr(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_shift_op(
                    Instr::ShrInt,
                    ">>",
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
            Self::BitNot(l, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_bit_not_op(
                    l, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::Eq(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_eq_op(
                    l, r, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::NotEq(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_neq_op(
                    l, r, *span1, *span2, tgt_id, v, ctx, state, output,
                ))
            }
            Self::Sup(l, r, span1, span2) => {
                debug_assert!(uses_id);
                Some(compile_cmp_op(
                    Instr::SupFloat,
                    Instr::SupInt,
                    Instr::SupStr,
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
                Some(compile_cmp_op(
                    Instr::SupEqFloat,
                    Instr::SupEqInt,
                    Instr::SupEqStr,
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
                Some(compile_cmp_op(
                    Instr::InfFloat,
                    Instr::InfInt,
                    Instr::InfStr,
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
                Some(compile_cmp_op(
                    Instr::InfEqFloat,
                    Instr::InfEqInt,
                    Instr::InfEqStr,
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
            Self::InlineCondition(main_condition, code, span, condition_span) => {
                debug_assert!(uses_id);
                Some(compile_inline_condition(
                    main_condition,
                    code,
                    *span,
                    *condition_span,
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
            Self::CallValue(callee, args, span, args_indexes) if uses_id => Some(
                handle_value_call(
                    output,
                    v,
                    ctx,
                    state,
                    tgt_id,
                    callee,
                    args,
                    *span,
                    args_indexes,
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
            Self::AnonymousFunction(_, _, span) => {
                debug_assert!(uses_id);
                let DataType::Fn(fn_id) = self.infer_type(v, ctx, state) else {
                    unsafe { unreachable_unchecked() }
                };
                let fn_id = fn_id as usize;
                let captures = state.fns[fn_id].captures.clone();
                if captures.is_empty() {
                    // A closure that reads nothing around it carries no
                    // environment, so its value is where its body starts and
                    // nothing else. A call that names it never reads the value,
                    // but a position the compiler does not follow, such as an
                    // `any`, holds it.
                    Some(compile_fn_entry_value(fn_id, v, ctx, state, output, tgt_id))
                } else {
                    // The value of a closure that captures is where its body
                    // starts followed by its environment: the cells of the
                    // variables its body reads, in the order the body expects
                    // them. Two closures written in one scope take the same
                    // cells, so what one writes the other reads.
                    let entry_id = state.fn_entry_register(fn_id);
                    let env_id = state.alloc_reg_tgt(tgt_id);
                    output.push(Instr::EmptyFnValue(env_id));
                    output.push(Instr::Push(env_id, entry_id));
                    for (name, _) in &captures {
                        let cell_id = capture_cell(name, *span, v, ctx, state, output);
                        output.push(Instr::Push(env_id, cell_id));
                    }
                    Some(env_id)
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
            Self::Condition(main_condition, code, _, condition_span) => {
                debug_assert!(!uses_id);
                compile_condition(main_condition, code, *condition_span, v, ctx, state, output);
                None
            }
            Self::WhileBlock(condition, code, condition_span) => {
                debug_assert!(!uses_id);
                compile_while_loop(condition, code, *condition_span, v, ctx, state, output);
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
                if let Some((enum_id, variant_idx)) =
                    variant_constructor(path, &[], *span, v, ctx, state)
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
                    compiler_errors::error_enum(
                        "Unknown enum variant",
                        &format!("{} does not name an enum variant", path.join("::")),
                        *span,
                        ctx.file_idx,
                        state.sources,
                    );
                }
            }
            Self::CallValue(callee, args, span, args_indexes) if !uses_id => {
                let output_id = handle_value_call(
                    output,
                    v,
                    ctx,
                    state,
                    tgt_id,
                    callee,
                    args,
                    *span,
                    args_indexes,
                );
                if let Some(id) = output_id {
                    state.free_reg(id, v);
                }
                None
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
    /// A generic `struct` or `enum` declaration, by its index in
    /// `Generics::templates`. It names no type of its own: each set of type
    /// arguments it is applied to becomes an ordinary struct or enum. The
    /// symbol is what makes `g::Slot<int>` reach the `Slot` the module behind
    /// `g` declares rather than whichever module declared that name last.
    Template(u32),
}

/// Whether two symbols are the same underlying definition (same kind, same
/// id in the global fn/struct/enum/template tables).
const fn symbol_ids_equal(a: SymbolKind, b: SymbolKind) -> bool {
    match (a, b) {
        (SymbolKind::Fn(x), SymbolKind::Fn(y))
        | (SymbolKind::Struct(x), SymbolKind::Struct(y))
        | (SymbolKind::Enum(x), SymbolKind::Enum(y)) => x == y,
        (SymbolKind::Template(x), SymbolKind::Template(y)) => x == y,
        _ => false,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Namespace {
    pub symbols: Vec<(SmolStr, SymbolKind)>,
    pub children: Vec<(SmolStr, Self)>,
}

/// The scope of every file in the program, keyed by its index in `sources`.
///
/// A name written in a file resolves in that file's scope, whether it is
/// written in a signature or in a function body: the file's own declarations,
/// the symbols its bare imports merged in, and the modules it bound with `as`.
/// A module imported under an alias therefore still sees its own declarations
/// while its bodies compile, which the file that imported it does not.
///
/// File 0 is the entry file. Its scope is the program's outward surface: what
/// an embedding host can call and what the artifact exporter writes entry
/// points for.
#[derive(Debug, Clone, Default)]
pub struct FileNamespaces {
    files: FxHashMap<u16, Namespace>,
    /// Handed out for a file index no source was parsed for, so a lookup
    /// against one reports an unknown name instead of panicking.
    empty: Namespace,
}

impl FileNamespaces {
    pub fn insert(&mut self, file_idx: u16, namespace: Namespace) {
        self.files.insert(file_idx, namespace);
    }
    #[must_use]
    pub fn get(&self, file_idx: u16) -> &Namespace {
        self.files.get(&file_idx).unwrap_or(&self.empty)
    }
    pub fn get_mut(&mut self, file_idx: u16) -> &mut Namespace {
        self.files.entry(file_idx).or_default()
    }
    /// The entry file's scope.
    #[must_use]
    pub fn root(&self) -> &Namespace {
        self.get(0)
    }
    /// The symbol count of every file's scope, so a compile attempt that
    /// declares into one can be undone with [`FileNamespaces::rollback_to`].
    ///
    /// Every file is captured, not only the entry one: a compile writes into
    /// the scope of the file the body it is compiling was written in. A
    /// parameter inferred to be a function is declared in the scope the callee
    /// was written in, and a `let` bound to a function in the scope of the
    /// file the `let` sits in, so a call that reaches into an imported module
    /// grows that module's scope. Both declarations remove themselves once the
    /// body they belong to is compiled, which is why only an attempt that
    /// aborts partway leaves one behind.
    ///
    /// A namespace's children are not captured: one is pushed only where an
    /// import is parsed, and that happens before any of this state can be
    /// rolled back.
    #[must_use]
    pub fn checkpoint(&self) -> FileNamespacesCheckpoint {
        FileNamespacesCheckpoint {
            symbols: self
                .files
                .iter()
                .map(|(file_idx, namespace)| (*file_idx, namespace.symbols.len()))
                .collect(),
        }
    }
    /// Truncates every file's scope back to `checkpoint`.
    ///
    /// A file the checkpoint does not name had no symbols of its own to keep:
    /// a lookup against a file index nothing was parsed for creates its entry,
    /// empty, on demand.
    pub fn rollback_to(&mut self, checkpoint: &FileNamespacesCheckpoint) {
        for (file_idx, namespace) in &mut self.files {
            let len = checkpoint.symbols.get(file_idx).copied().unwrap_or(0);
            namespace.symbols.truncate(len);
        }
    }
}

/// A checkpoint of the per-file scopes, taken by [`FileNamespaces::checkpoint`]
/// and undone by [`FileNamespaces::rollback_to`].
#[derive(Debug, Clone, Default)]
pub struct FileNamespacesCheckpoint {
    symbols: FxHashMap<u16, usize>,
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
    /// Resolves a function id by name (with an optional module path). Returns
    /// `None` both when `path` names no namespace and when the namespace it
    /// names declares no such function; raising is left to the caller, which
    /// is the only place that knows whether another resolution can still
    /// match. A call into a `host` or `dylib` namespace is one that can: those
    /// functions are declared in `State::dyn_libs` and never appear in this
    /// tree.
    #[must_use]
    pub fn find_function(&self, path: &[SmolStr], function_name: &str) -> Option<usize> {
        self.resolve(path)?.symbols.iter().find_map(|(name, kind)| {
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
        self.resolve(path)?.symbols.iter().find_map(|(name, kind)| {
            if name.as_str() == enum_name
                && let SymbolKind::Enum(enum_id) = kind
            {
                Some(*enum_id as usize)
            } else {
                None
            }
        })
    }
    /// Resolves a generic declaration by name (with an optional module path)
    /// to its index in `Generics::templates`. Returns `None` when the path
    /// names no namespace or the namespace it names declares no generic type
    /// by that name; the caller decides whether another resolution can still
    /// match.
    #[must_use]
    pub fn find_template(&self, path: &[SmolStr], type_name: &str) -> Option<usize> {
        self.resolve(path)?.symbols.iter().find_map(|(name, kind)| {
            if name.as_str() == type_name
                && let SymbolKind::Template(idx) = kind
            {
                Some(*idx as usize)
            } else {
                None
            }
        })
    }
    /// The namespace `path` names, or `None` when any segment of it is not
    /// declared. Resolution alone is not an error: a path can also name a
    /// `host` or `dylib` namespace, which lives in `State::dyn_libs`.
    #[must_use]
    pub fn resolve(&self, path: &[SmolStr]) -> Option<&Self> {
        let mut current = self;
        for sub in path {
            current = &current.children.iter().find(|(name, _)| name == sub)?.1;
        }
        Some(current)
    }
    /// The namespace `path` names, for a caller that has nowhere else to look:
    /// an unresolvable path ends the compile with the unknown-namespace error.
    #[must_use]
    pub fn walk_to_namespace(
        &self,
        path: &[SmolStr],
        span: Span,
        file_idx: u16,
        sources: &[Source],
    ) -> &Self {
        match self.resolve(path) {
            Some(namespace) => namespace,
            None => error_unknown_namespace(path, span, file_idx, sources),
        }
    }
}

/// The key a loaded module is remembered under, so a module two files reach is
/// parsed once and its functions, structs and enums are registered once.
///
/// Two imports of one file can spell its path differently: a library or package
/// import resolves through the library directory (`import "strutil";`) while a
/// source-relative one resolves next to the importing file (`import
/// "../src/main.cdl";`), and either can arrive through a symbolic link or a
/// `..`. Resolving both to the same real path is what makes the two spellings
/// one module. A path with no real file behind it is kept as written, so the
/// error that follows names the place that was tried.
fn loaded_file_key(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

/// The source of a standard library module the browser build carries, by the
/// path a library import resolves to (`std/math.cdl`). These are the modules
/// whose native half the runtime provides itself; see
/// [`candela_vm::intrinsics`].
#[cfg(target_arch = "wasm32")]
fn embedded_std_module(path: &str) -> Option<&'static str> {
    match path {
        "std/math.cdl" => Some(include_str!("../../libs/std/math.cdl")),
        "std/time.cdl" => Some(include_str!("../../libs/std/time.cdl")),
        "std/random.cdl" => Some(include_str!("../../libs/std/random.cdl")),
        "std/hash.cdl" => Some(include_str!("../../libs/std/hash.cdl")),
        _ => None,
    }
}

/// The places an import was looked for, for the error that says it was found in
/// none of them.
///
/// `chosen` is where resolution settled and `library` is the library or package
/// path, which is the same file when the import is a library one and a second
/// place to name when it is a file import that fell back.
fn tried_paths(chosen: &Path, library: Option<&Path>) -> Vec<PathBuf> {
    let mut tried = vec![chosen.to_path_buf()];
    if let Some(library) = library
        && library != chosen
    {
        tried.push(library.to_path_buf());
    }
    tried
}

/// Loads the `std/list` module implicitly so its `impl list` methods
/// (`arr.map(f)`, `arr.sum()`, ...) work with no explicit import. Resolution
/// goes through the same resolver every other library import uses; a missing
/// library directory is not an error, the prelude is absent.
#[cfg(not(target_arch = "wasm32"))]
fn load_auto_prelude(
    fns: &mut Vec<Function>,
    structs: &mut Vec<Struct>,
    enums: &mut Vec<EnumType>,
    dynamic_libs: &mut Vec<Dynamiclib>,
    sources: &mut Vec<Source>,
    namespace: &mut Namespace,
    files: &mut FxHashMap<PathBuf, Namespace>,
    file_namespaces: &mut FileNamespaces,
    pending_structs: &mut Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)>,
    pending_enums: &mut PendingEnums,
    pending_fns: &mut PendingFns,
    pending_dylibs: &mut Vec<PendingDylib>,
    pending_host: &mut Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )>,
    generics: &mut Generics,
    resolver: &ImportResolver,
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

    let Some(path) = resolver.library_path(PRELUDE_REL).map(loaded_file_key) else {
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
        resolver,
    );
    file_namespaces.insert(child_src_idx, child_namespace.clone());
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

/// Whether a value of type `ty` has a C shape a `dylib` call can pass or
/// return: an `int`, a `float`, a `string`, an array, `null` for a function
/// that returns nothing, or a struct whose fields all have one. `depth` stops a
/// struct that holds itself from recursing without end.
fn has_c_representation(ty: &DataType, structs: &[Struct], depth: usize) -> bool {
    match ty {
        DataType::Int
        | DataType::Float
        | DataType::String
        | DataType::Array(_)
        | DataType::Null => true,
        DataType::Struct(id) => {
            depth < structs.len()
                && structs.get(*id as usize).is_some_and(|s| {
                    s.fields
                        .iter()
                        .all(|(_, field, _)| has_c_representation(field, structs, depth + 1))
                })
        }
        _ => false,
    }
}

/// The namespace a `dylib` block's functions live under: a logical name as
/// written (`z`), and for a path the file's name without its directory or
/// extension (`../native/mylib` is `mylib`).
#[cfg(not(target_arch = "wasm32"))]
fn dylib_namespace(spec: &str, is_logical: bool) -> SmolStr {
    if is_logical {
        return SmolStr::from(spec);
    }
    Path::new(spec)
        .file_prefix()
        .and_then(|s| s.to_str())
        .unwrap_or(spec)
        .to_smolstr()
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
    let name = dylib_namespace(spec, false);

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

/// The spec to record for a library the standard library names, relative to the
/// `libs` directory, and `None` for every other `dylib` import.
///
/// A std module reaches its C library by a path from its own source file
/// (`../std_src/math/math`), and an artifact carries no source tree to resolve
/// that against. Recording the path from `libs` instead is what lets
/// `candela-vm` find the file through the toolchain. Both ends have to sit
/// under the library directory: the module doing the import, and the library it
/// names.
#[cfg(not(target_arch = "wasm32"))]
fn std_library_spec(lib_dir: Option<&Path>, file_path: &Path, spec: &str) -> Option<SmolStr> {
    let lib_dir = lib_dir?.canonicalize().ok()?;
    if !file_path.canonicalize().ok()?.starts_with(&lib_dir) {
        return None;
    }
    let target = file_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(spec);
    // The spec leaves the platform extension off, so the file is not there to
    // canonicalize; its directory is.
    let relative = target
        .parent()?
        .canonicalize()
        .ok()?
        .strip_prefix(&lib_dir)
        .ok()?
        .join(target.file_name()?);
    // Written with forward slashes, so an artifact built on Windows names the
    // same file on Linux.
    let mut recorded = String::new();
    for part in relative.components() {
        if !recorded.is_empty() {
            recorded.push('/');
        }
        recorded.push_str(part.as_os_str().to_str()?);
    }
    Some(SmolStr::from(recorded))
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

/// Declares everything `code` defines into `namespace`, parsing each file it
/// imports the same way.
///
/// Registering the finished scope in `file_namespaces` is the caller's: an
/// imported file's scope is bound into the importer as well, while the entry
/// file's is needed nowhere else and can be handed over whole.
fn parse_toplevel(
    code: Vec<Expr>,
    file_path: &Path,
    src_file_idx: u16,
    fns: &mut Vec<Function>,
    structs: &mut Vec<Struct>,
    enums: &mut Vec<EnumType>,
    dynamic_libs: &mut Vec<Dynamiclib>,
    sources: &mut Vec<Source>,
    namespace: &mut Namespace,
    files: &mut FxHashMap<PathBuf, Namespace>,
    file_namespaces: &mut FileNamespaces,
    pending_structs: &mut Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)>,
    pending_enums: &mut PendingEnums,
    pending_fns: &mut PendingFns,
    pending_dylibs: &mut Vec<PendingDylib>,
    pending_host: &mut Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )>,
    generics: &mut Generics,
    resolver: &ImportResolver,
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
                check_operator_method(&fn_name, &fn_args, span, src_file_idx, sources);
                let returns_void = check_if_returns_void(&fn_code);
                let mut callees = Vec::new();
                collect_direct_fn_calls(&fn_code, self_type_of(&fn_name), &mut callees);

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
                    captures: Box::from([]),
                    generics: (!type_params.is_empty()).then(|| {
                        Box::new(FnGenerics {
                            params: type_params.clone(),
                            arg_types: fn_args.iter().map(|(_, t)| t.clone()).collect(),
                            return_type: fn_return_type.clone(),
                            bindings: Box::from([]),
                            file_idx: src_file_idx,
                        })
                    }),
                    entry_register: None,
                });
                pending_fns.push((fn_id, src_file_idx, fn_args, fn_return_type, type_params));
                namespace.symbols.push((fn_name, SymbolKind::Fn(fn_id)));
            }
            Expr::StructDeclare(name, fields, span, type_params) => {
                // A generic declaration registers no type of its own: each
                // instantiation of it becomes an ordinary struct.
                if !type_params.is_empty() {
                    let template = generics.add_struct_template(
                        name.clone(),
                        type_params,
                        src_file_idx,
                        fields,
                    );
                    namespace
                        .symbols
                        .push((name, SymbolKind::Template(template)));
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
                    let template = generics.add_enum_template(
                        name.clone(),
                        type_params,
                        src_file_idx,
                        variants,
                    );
                    namespace
                        .symbols
                        .push((name, SymbolKind::Template(template)));
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
            // A browser has no file system and no library loader. The std
            // modules the runtime carries import, and their own `dylib` blocks
            // bind to its intrinsics; nothing else does.
            #[cfg(target_arch = "wasm32")]
            Expr::ImportDylib(..) if file_path.to_str().and_then(embedded_std_module).is_none() => {
                wasm_error("WASM does not support loading dynamic libraries")
            }
            #[cfg(target_arch = "wasm32")]
            Expr::ImportFile(ref path, _, is_logical, _)
                if !is_logical || embedded_std_module(path).is_none() =>
            {
                wasm_error(
                    "WASM does not support importing files. The standard library modules std/math, std/time, std/random and std/hash import",
                )
            }
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
            resolver,
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
                // the directories the embedding host named, the root of every
                // package this program depends on, then the importing file's
                // own. A host whose libraries sit apart from its sources
                // (`lib/` beside `src/`) names that directory and both forms
                // find it; a package that ships a native library beside its
                // `.cdl` sources is found the same way.
                let host_dirs = crate::rt::dylib_dirs();
                let package_dirs = resolver.root_dirs();
                let file_dir = file_path.parent().unwrap_or_else(|| Path::new("."));
                let mut dirs: Vec<&Path> = Vec::with_capacity(host_dirs.len() + 2);
                dirs.extend(host_dirs.iter().map(PathBuf::as_path));
                dirs.extend(package_dirs.iter().map(PathBuf::as_path));
                dirs.push(file_dir);

                // A check-only compile binds the declared signatures and opens
                // nothing, so a library that is not there yet is no error.
                let (lib, dylib_name) = if is_check_only() {
                    (None, dylib_namespace(spec.as_str(), is_logical))
                } else {
                    let (lib, dylib_name) = if is_logical {
                        // e.g. `z` -> `libz.so` / `libz.dylib` / `z.dll`.
                        let filename = resolve_library_filename(spec.as_str(), TargetOs::CURRENT);
                        (open_logical_dylib(&dirs, &filename), spec.clone())
                    } else {
                        open_path_dylib(&dirs, spec.as_str())
                    };
                    let lib = lib
                        .unwrap_or_else(|| error_cannot_load_dynlib(span, src_file_idx, sources));
                    (Some(Rc::new(lib)), dylib_name)
                };
                // A standard-library module names its C library by a path
                // relative to its own source file, which an artifact never
                // carries. Record that one relative to the `libs` directory
                // instead, so `candela-vm` reaches it through the toolchain
                // from whatever directory the artifact runs in.
                let recorded = match std_library_spec(resolver.lib_dir(), file_path, spec.as_str())
                {
                    Some(relative) => (relative, LibraryOrigin::StandardLibrary),
                    None => (spec, LibraryOrigin::Program),
                };
                pending_dylibs.push((
                    src_file_idx,
                    dynamic_libs.len() as u16,
                    fn_signatures,
                    lib,
                    recorded,
                    span,
                ));
                dynamic_libs.push(Dynamiclib {
                    name: dylib_name,
                    fns: Box::new([]),
                    is_host: false,
                });
            }
            // Only a std module's block gets here (see the first pass), and it
            // names its library relative to its own file, as on every target.
            // The binding is recorded the way a desktop build records it, so
            // an intrinsic resolves it by the same name.
            #[cfg(target_arch = "wasm32")]
            Expr::ImportDylib(path, fn_signatures, span) => {
                let recorded = path.strip_prefix("../").unwrap_or(path.as_str());
                let dylib_name = Path::new(recorded)
                    .file_prefix()
                    .and_then(|s| s.to_str())
                    .unwrap_or(recorded)
                    .to_smolstr();
                pending_dylibs.push((
                    src_file_idx,
                    dynamic_libs.len() as u16,
                    fn_signatures,
                    (),
                    (SmolStr::from(recorded), LibraryOrigin::StandardLibrary),
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
                // In a browser the module is one the runtime carries, keyed by
                // its library path; the first pass let no other through.
                #[cfg(target_arch = "wasm32")]
                let file_path = PathBuf::from(path.as_str());
                // Where the import reads from, and, for the error when it reads
                // from nowhere, every place that was tried.
                #[cfg(not(target_arch = "wasm32"))]
                let library_path = resolver.library_path(path.as_str()).map(loaded_file_key);
                #[cfg(not(target_arch = "wasm32"))]
                let file_path = if is_logical {
                    // A library import (`import "std/string";`, extensionless)
                    // resolves against a package root or the shipped library
                    // directory, never source-relative, so it works from any
                    // working directory with nothing set.
                    library_path.clone().unwrap_or_else(|| {
                        error_cannot_read_file(span, src_file_idx, sources, &[]);
                    })
                } else {
                    // A `.cdl` file import resolves next to the importing file first,
                    // then falls back to the shipped library directory.
                    let beside = file_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(path.as_str());
                    beside.canonicalize().unwrap_or_else(|_| {
                        library_path.clone().unwrap_or_else(|| {
                            error_cannot_read_file(
                                span,
                                src_file_idx,
                                sources,
                                std::slice::from_ref(&beside),
                            );
                        })
                    })
                };

                let child_namespace = if let Some(cached) = files.get(&file_path) {
                    cached.clone()
                } else {
                    #[cfg(target_arch = "wasm32")]
                    let file_contents =
                        String::from(embedded_std_module(path.as_str()).unwrap_or_default());
                    #[cfg(not(target_arch = "wasm32"))]
                    let file_contents = std::fs::read_to_string(&file_path).unwrap_or_else(|_| {
                        error_cannot_read_file(
                            span,
                            src_file_idx,
                            sources,
                            &tried_paths(&file_path, library_path.as_deref()),
                        );
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
                        resolver,
                    );
                    file_namespaces.insert(child_src_idx, child_namespace.clone());
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
                        // Only the entry file's `main` runs, so a module's own
                        // `main` function is not one of the names it exports. A
                        // package that keeps one would otherwise collide with
                        // the importer's the moment it is bare-imported. Only
                        // the function is held back: a struct or an enum named
                        // `main` is a separate symbol and merges like any
                        // other.
                        if name == "main" && matches!(kind, SymbolKind::Fn(_)) {
                            continue;
                        }
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
}

fn resolve_types(
    structs: &mut Vec<Struct>,
    enums: &mut Vec<EnumType>,
    fns: &mut Vec<Function>,
    generics: &mut Generics,
    pending_structs: Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)>,
    pending_enums: PendingEnums,
    pending_fns: PendingFns,
    pending_dylibs: Vec<PendingDylib>,
    pending_host: Vec<(
        u16,
        u16,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        SmolStr,
        Span,
    )>,
    file_namespaces: &FileNamespaces,
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
        let resolved_fields = fields
            .iter()
            .map(|(field_name, field_type, field_span)| {
                (
                    field_name.clone(),
                    field_type.to_datatype(&mut TypeCtx {
                        file_idx: src_file_idx,
                        namespaces: file_namespaces,
                        sources,
                        structs,
                        enums,
                        fns,
                        generics,
                    }),
                    *field_span,
                )
            })
            .collect();
        structs[struct_id as usize].fields = resolved_fields;
    }
    for (enum_id, src_file_idx, variants) in pending_enums {
        let resolved_variants = variants
            .iter()
            .map(|(variant_name, payload, name_span)| EnumVariant {
                name: variant_name.clone(),
                payload: payload
                    .iter()
                    .map(|t| {
                        t.to_datatype(&mut TypeCtx {
                            file_idx: src_file_idx,
                            namespaces: file_namespaces,
                            sources,
                            structs,
                            enums,
                            fns,
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
                                namespaces: file_namespaces,
                                sources,
                                structs,
                                enums,
                                fns,
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
                        namespaces: file_namespaces,
                        sources,
                        structs,
                        enums,
                        fns,
                        generics,
                    }),
                    t_span,
                )
            });
    }
    // The signatures a check-only compile bound without a library, which take
    // ids past the end of the bound ones so each still names one function.
    #[cfg(not(target_arch = "wasm32"))]
    let mut unbound_dylib_fns = 0_usize;
    #[cfg(target_arch = "wasm32")]
    let unbound_dylib_fns = 0_usize;
    for (src_file_idx, dynlib_id, fn_signatures, lib, (library_spec, origin), span) in
        pending_dylibs
    {
        // An intrinsic binding holds no library open.
        #[cfg(target_arch = "wasm32")]
        let () = lib;
        let resolved: Vec<FnSignature> = fn_signatures
            .iter()
            .map(|(fn_name, fn_args, fn_return_type, fn_name_span)| {
                let fn_args = fn_args
                    .iter()
                    .map(|t| {
                        t.to_datatype(&mut TypeCtx {
                            file_idx: src_file_idx,
                            namespaces: file_namespaces,
                            sources,
                            structs,
                            enums,
                            fns,
                            generics,
                        })
                    })
                    .collect::<Vec<DataType>>()
                    .into_boxed_slice();
                let fn_return_type = fn_return_type.to_datatype(&mut TypeCtx {
                    file_idx: src_file_idx,
                    namespaces: file_namespaces,
                    sources,
                    structs,
                    enums,
                    fns,
                    generics,
                });
                // Every type a C function takes or returns needs a C shape;
                // one that has none is refused here, at the signature, rather
                // than when the calling interface is built from it.
                if let Some(ty) = fn_args
                    .iter()
                    .chain(std::iter::once(&fn_return_type))
                    .find(|ty| !has_c_representation(ty, structs, 0))
                {
                    error_type_has_no_c_representation(
                        fn_name,
                        ty,
                        *fn_name_span,
                        src_file_idx,
                        sources,
                        TypeNames { structs, enums },
                    );
                }
                let return_val = FnSignature {
                    name: fn_name.clone(),
                    args: fn_args.clone(),
                    return_type: fn_return_type.clone(),
                    id: (dynamic_libs_fns.len() + unbound_dylib_fns) as u16,
                    variadic: false,
                };
                // A check-only compile opened no library. The declared
                // signature is all a call site reads, so the binding stops at
                // it: no symbol, no calling interface, nothing to run.
                #[cfg(not(target_arch = "wasm32"))]
                let Some(lib) = lib.as_ref() else {
                    unbound_dylib_fns += 1;
                    return return_val;
                };
                #[cfg(not(target_arch = "wasm32"))]
                let arg_types: Vec<_> = fn_args.iter().map(|t| t.to_c_type(structs)).collect();
                #[cfg(not(target_arch = "wasm32"))]
                let return_type = fn_return_type.to_c_type(structs);
                #[cfg(not(target_arch = "wasm32"))]
                let cif = libffi::middle::Cif::new(arg_types, return_type);
                #[cfg(not(target_arch = "wasm32"))]
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

                // Where nothing loads, the only bindings are the standard
                // library's, and each is one of the runtime's intrinsics.
                #[cfg(target_arch = "wasm32")]
                let intrinsic = candela_vm::intrinsics::lookup(&library_spec, fn_name)
                    .unwrap_or_else(|| {
                        error_cannot_find_dynlib_symbol(
                            fn_name,
                            *fn_name_span,
                            span,
                            src_file_idx,
                            sources,
                        );
                    });

                dynamic_libs_fns.push(DynamicLibFn {
                    types: Box::from(types),
                    library: library_spec.clone(),
                    origin,
                    symbol: fn_name.clone(),
                    #[cfg(not(target_arch = "wasm32"))]
                    _lib: Rc::clone(lib),
                    #[cfg(not(target_arch = "wasm32"))]
                    ptr,
                    #[cfg(not(target_arch = "wasm32"))]
                    cif,
                    #[cfg(target_arch = "wasm32")]
                    intrinsic,
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
                                namespaces: file_namespaces,
                                sources,
                                structs,
                                enums,
                                fns,
                                generics,
                            })
                        })
                        .collect::<Vec<DataType>>()
                        .into_boxed_slice()
                };
                let fn_return_type = fn_return_type.to_datatype(&mut TypeCtx {
                    file_idx: src_file_idx,
                    namespaces: file_namespaces,
                    sources,
                    structs,
                    enums,
                    fns,
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
    pub callsite_registers: Vec<Vec<u16>>,
    pub dyn_lib_fns: Vec<DynamicLibFn>,
    pub host_fns: Vec<HostFnSig>,
    pub allocated_arg_count: usize,
    pub allocated_call_depth: usize,
    pub sources: Vec<Source>,
    pub structs: Vec<Struct>,
    pub enums: Vec<EnumType>,
    pub functions: Vec<Function>,
    pub dyn_libs: Vec<Dynamiclib>,
    /// Every file's scope, so a later compile against this program resolves a
    /// name the way the file it is written in does.
    pub namespaces: FileNamespaces,
    pub const_registers: FxHashMap<Data, u16>,
    pub free_registers: Vec<u16>,
    /// The registers an indirect call passes its arguments in, kept so a later
    /// compile against this program uses the same ones.
    pub indirect_registers: IndirectRegisters,
    /// The generic declarations and the instantiations made from them, kept so
    /// a later compile against this program (an embedding host calling in, a
    /// REPL line) resolves them the same way.
    pub generics: Generics,
    /// Whether the program was compiled in the release profile, so a later
    /// compile against it (an export entry point) uses the same one.
    pub optimize: bool,
}

impl CompileOutput {
    /// Reports that the program has no entry point, unless it has one.
    ///
    /// Only the entry file's `main` counts: an imported module's own `main` is
    /// ignored. Compiling a file without one is not an error, which is what
    /// lets `candela check` pass on a library whose entry has nothing to run;
    /// running, building or embedding the program calls this first.
    pub fn require_main(&self) {
        if !self
            .functions
            .iter()
            .any(|func| func.name == "main" && func.src_file == 0)
        {
            compiler_errors::error_no_main(&self.sources);
        }
    }
}

/// Compiles `contents` and everything it imports into one program.
///
/// `resolver` says where a library import reads from: the standard library, and
/// the root of every package the program depends on.
#[must_use]
pub fn compile(
    contents: String,
    filename: &str,
    debug: bool,
    resolver: &ImportResolver,
) -> CompileOutput {
    compile_profile(contents, filename, debug, resolver, false)
}

/// [`compile`] in the profile `optimize` names.
///
/// `false` is the debug profile every run from source uses, `true` the release
/// profile `candela build` uses. The two produce programs that behave alike,
/// with the same diagnostics, runtime errors and spans; the release one runs
/// faster.
#[must_use]
pub fn compile_profile(
    contents: String,
    filename: &str,
    debug: bool,
    resolver: &ImportResolver,
    optimize: bool,
) -> CompileOutput {
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
    let mut pools: Pools = Pools::new(
        Pool::with_capacity(10),
        Pool::with_capacity(2),
        StringPool::with_capacity(10),
    );
    let mut instr_src: Vec<InstrSrc> = Vec::new();
    let mut callsite_registers: Vec<Vec<u16>> = Vec::new();
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
    let main_path = loaded_file_key(PathBuf::from(filename));
    let mut namespace = Namespace::default();

    let mut files: FxHashMap<PathBuf, Namespace> = FxHashMap::default();
    let mut file_namespaces = FileNamespaces::default();
    let mut pending_structs: Vec<(u16, u16, Box<[(SmolStr, TypeExpr, Span)]>)> = Vec::new();
    let mut pending_enums: PendingEnums = Vec::new();
    let mut pending_fns: PendingFns = Vec::with_capacity(2);
    let mut generics = Generics::default();
    let mut pending_dylibs: Vec<PendingDylib> = Vec::new();
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
        &mut dyn_libs,
        &mut sources,
        &mut namespace,
        &mut files,
        &mut file_namespaces,
        &mut pending_structs,
        &mut pending_enums,
        &mut pending_fns,
        &mut pending_dylibs,
        &mut pending_host,
        &mut generics,
        resolver,
    );
    file_namespaces.insert(0, namespace);
    // Before any type is resolved: a name two modules both declare is renamed
    // here, and everything downstream reads the name it ended up with.
    qualify_duplicate_type_names(
        &mut structs,
        &mut enums,
        &mut functions,
        &pending_structs
            .iter()
            .map(|(struct_id, file_idx, _)| (*struct_id, *file_idx))
            .collect::<Vec<(u16, u16)>>(),
        &pending_enums
            .iter()
            .map(|(enum_id, file_idx, _)| (*enum_id, *file_idx))
            .collect::<Vec<(u16, u16)>>(),
        &file_namespaces,
        &sources,
    );
    resolve_types(
        &mut structs,
        &mut enums,
        &mut functions,
        &mut generics,
        pending_structs,
        pending_enums,
        pending_fns,
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
        return_downcast: None,
    };
    let mut indirect_registers = IndirectRegisters::default();
    let mut state = State {
        registers: &mut registers,
        fns: &mut functions,
        structs: &mut structs,
        enums: &mut enums,
        pools: &mut pools,
        instr_src: &mut instr_src,
        callsite_registers: &mut callsite_registers,
        dyn_libs: &mut dyn_libs,
        allocated_arg_count: &mut allocated_arg_count,
        allocated_call_depth: &mut allocated_call_depth,
        const_registers: &mut const_registers,
        free_registers: &mut free_registers,
        sources: &mut sources,
        reserved_registers: FxHashSet::default(),
        value_callsites: FxHashSet::default(),
        optimize,
        namespaces: &mut file_namespaces,
        generics: &mut generics,
        indirect_registers: &mut indirect_registers,
    };
    // The entry file's `main` is the program's top level. A file that declares
    // none compiles to nothing but the halt: its declarations and signatures are
    // still resolved, which is what lets `candela check` pass on a library whose
    // entry has nothing to run. Starting a program without one is reported where
    // the program is run or packaged, not here.
    let main_body = state
        .fns
        .iter()
        .find(|func| func.name == "main" && func.src_file == 0)
        .map(|func| func.code.clone());
    let mut instructions = if let Some(code) = main_body {
        compile_expr(&code, &mut variables, ctx, &mut state)
    } else {
        Vec::new()
    };
    instructions.push(Instr::Halt(0));
    if optimize {
        scalar::replace_scalars(&mut flow::Program {
            instructions: &mut instructions,
            registers: &mut registers,
            objs: &pools.objs,
            const_registers: &mut const_registers,
            instr_src: &mut instr_src,
            callsite_registers: &mut callsite_registers,
            functions: &mut functions,
            indirect: &indirect_registers,
        });
        copies::forward_copies(&mut flow::Program {
            instructions: &mut instructions,
            registers: &mut registers,
            objs: &pools.objs,
            const_registers: &mut const_registers,
            instr_src: &mut instr_src,
            callsite_registers: &mut callsite_registers,
            functions: &mut functions,
            indirect: &indirect_registers,
        });
    }
    use_immediates(&mut instructions, &[], &registers, &const_registers);
    copies::chain_moves(&mut flow::Program {
        instructions: &mut instructions,
        registers: &mut registers,
        objs: &pools.objs,
        const_registers: &mut const_registers,
        instr_src: &mut instr_src,
        callsite_registers: &mut callsite_registers,
        functions: &mut functions,
        indirect: &indirect_registers,
    });

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
        callsite_registers,
        dyn_lib_fns,
        host_fns,
        allocated_arg_count,
        allocated_call_depth,
        sources,
        structs,
        enums,
        functions,
        dyn_libs,
        namespaces: file_namespaces,
        const_registers,
        free_registers,
        indirect_registers,
        generics,
        optimize,
    }
}
