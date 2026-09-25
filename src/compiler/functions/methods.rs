// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use super::expr::{Expr, Span};
use crate::cold_path;
use crate::compiler::UnwrapId;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_data::{Ctx, State};
use crate::compiler::compiler_errors::error_no_such_method;
use crate::compiler::compiler_errors::error_operator_method;
use crate::compiler::compiler_errors::error_type_args_on_builtin_method;
use crate::compiler::expr::OperatorForm;
use crate::compiler::expr::is_default_call;
use crate::compiler::expr::mangle_method;
use crate::compiler::expr::operator_method;
use crate::compiler::expr::operator_method_answers_bool;
use crate::compiler::functions::fill_default_args;
use crate::compiler::functions::handle_functions;
use crate::compiler::functions::handle_value_call;
use crate::compiler::functions::user_functions::handle_user_function;
use crate::compiler::type_system::DataType;
use crate::compiler::type_system::TypeExpr;
use crate::compiler::type_system::infer_user_fn_return_type;
use crate::compiler::type_system::resolve_call_type_args;
use crate::compiler::type_system::struct_field_holds_fn;
use crate::instr::Instr;
use builtin_methods::builtin_methods;
use builtin_methods::is_builtin_method;
use smol_strc::SmolStr;

#[path = "builtin/builtin_methods.rs"]
mod builtin_methods;

/// The type name an `impl` block uses to add methods to a builtin-typed
/// receiver: `string`, `list`, `map`, `int`, `float`, or `bool`. A union
/// resolves only when every member maps to the same name. Struct, enum, and
/// `any` receivers return `None`; the first two dispatch through their own
/// paths and the last stays on the builtin table.
fn builtin_receiver_name(obj_type: &DataType) -> Option<&'static str> {
    match obj_type {
        DataType::String => Some("string"),
        DataType::Array(_) => Some("list"),
        DataType::Map(_) => Some("map"),
        DataType::Int => Some("int"),
        DataType::Float => Some("float"),
        DataType::Bool => Some("bool"),
        DataType::Union(union) => {
            let first = builtin_receiver_name(union.first()?)?;
            union
                .iter()
                .all(|t| builtin_receiver_name(t) == Some(first))
                .then_some(first)
        }
        _ => None,
    }
}

/// Resolves a builtin-typed receiver to a user-visible `impl` method: an
/// `impl list { fn sum(self) ... }` lowers to the mangled free function
/// `list#sum`, and `arr.sum()` finds it here by the receiver's type name. The
/// builtin method table keeps precedence, so an `impl` cannot shadow core
/// methods like `len` or `push`. The one carve-out is `find` on an array with
/// a function argument: the builtin `find` is the index search by value, and
/// the predicate form only exists as the `std/list` method. Returns `None`
/// when the receiver is not a builtin type, the name belongs to the builtin
/// table, or no impl method with the mangled name is loaded.
pub fn impl_method_on_builtin(
    name: &str,
    obj_type: &DataType,
    args: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<usize> {
    let type_name = builtin_receiver_name(obj_type)?;
    let find_predicate = name == "find"
        && type_name == "list"
        && args.len() == 1
        && matches!(args[0].infer_type(v, ctx, state), DataType::Fn(_));
    if is_builtin_method(name, type_name) && !find_predicate {
        return None;
    }
    let mangled = mangle_method(type_name, name);
    state.fns.iter().position(|f| f.name == mangled)
}

/// The `host` or `dylib` block a dot call's receiver names, when that receiver
/// is a namespace rather than a value: `app.rows(id)` with a `host "app"` block
/// in scope and no variable called `app`.
///
/// Both spellings of such a call mean the same thing, so the dot form routes to
/// the path `app::rows(id)` already takes. A variable wins: once `app` names a
/// value in scope, the dot is that value's method call and the block is
/// reachable only through `::`.
pub fn dyn_lib_receiver<'a>(
    obj: &'a Expr,
    v: &[Variable],
    state: &State<'_>,
) -> Option<&'a SmolStr> {
    let Expr::Var(name, _) = obj else {
        return None;
    };
    // The block table is checked before the locals because it is empty in a
    // program that declares no `host` or `dylib` block, so an ordinary method
    // call pays one emptiness check here and nothing else.
    if !state.dyn_libs.iter().any(|lib| lib.name == *name) {
        return None;
    }
    (!v.iter().any(|var| var.name == *name)).then_some(name)
}

/// The arguments of a method call with each `Default::default()` a
/// struct-typed parameter of the method receives resolved to that struct's
/// default, or `None` when there is nothing to resolve. A method is reached
/// through the receiver's type, so the receiver is typed first.
pub fn fill_method_defaults(
    obj: &Expr,
    args: &[Expr],
    namespace: &[SmolStr],
    args_indexes: &[Span],
    type_args: &[TypeExpr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<Box<[Expr]>> {
    if !type_args.is_empty() || namespace.len() != 1 || !args.iter().any(is_default_call) {
        return None;
    }
    // A dot call on a `host` or `dylib` block is the path call, which
    // resolves its own arguments.
    if dyn_lib_receiver(obj, v, state).is_some() {
        return None;
    }
    let type_name = match obj.infer_type(v, ctx, state) {
        DataType::Struct(id) => state.structs[id as usize].name.clone(),
        DataType::Enum(id) => state.enums[id as usize].name.clone(),
        _ => return None,
    };
    let mangled = mangle_method(&type_name, &namespace[0]);
    let fn_id = state.fns.iter().position(|f| f.name == mangled)?;
    let params: Vec<Option<DataType>> = state.fns[fn_id]
        .args
        .iter()
        .skip(1)
        .map(|(_, t)| t.clone())
        .collect();
    fill_default_args(args, args_indexes, &params)
}

pub fn handle_method_calls(
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    tgt_id: Option<u16>,
    obj: &Expr,
    args: &[Expr],
    namespace: &[SmolStr],
    obj_span: Span,
    fn_span: Span,
    args_indexes: &[Span],
    type_args: &[TypeExpr],
) -> Option<u16> {
    if let Some(filled) =
        fill_method_defaults(obj, args, namespace, args_indexes, type_args, v, ctx, state)
    {
        return handle_method_calls(
            output,
            v,
            ctx,
            state,
            tgt_id,
            obj,
            &filled,
            namespace,
            obj_span,
            fn_span,
            args_indexes,
            type_args,
        );
    }
    let name = namespace[namespace.len() - 1].as_str();

    // A dot call whose receiver names a `host` or `dylib` block is the
    // namespaced call written with a dot. Prepend the block's name and hand it
    // to the path `app::rows(id)` takes. This comes before the receiver is
    // typed as a value, where a block's name resolves to no variable.
    if namespace.len() == 1
        && let Some(lib_name) = dyn_lib_receiver(obj, v, state)
    {
        let path = [lib_name.clone(), namespace[0].clone()];
        return handle_functions(
            output,
            v,
            ctx,
            state,
            tgt_id,
            args,
            &path,
            fn_span,
            args_indexes,
            type_args,
        );
    }

    let obj_type = obj.infer_type(v, ctx, state);

    // A method call `recv.method(args)` on a struct value lowers to a call of
    // the mangled free function `Type#method(recv, args...)`. The receiver's
    // static type picks the type-unique symbol, so `Point.len()` and `Str.len()`
    // resolve to distinct functions and neither collides with a free `fn len`.
    // Field access (`recv.method` without parens) never reaches here; the
    // parser only produces an `ObjFunctionCall` when the call parentheses are
    // present.
    if let DataType::Struct(struct_id) = obj_type {
        let struct_name = state.structs[struct_id as usize].name.clone();
        let mangled = mangle_method(&struct_name, name);
        if let Some(fn_id) = state.fns.iter().position(|f| f.name == mangled) {
            // Prepend the receiver as argument 0, then reuse the ordinary
            // user-function call path; the VM sees a normal function call. Type
            // arguments the call is written with bind the method's own type
            // parameters, on top of the bindings its `impl` block already
            // carries for the receiver's instantiation.
            let call_type_args = if type_args.is_empty() {
                Vec::new()
            } else {
                resolve_call_type_args(fn_id, name, type_args, fn_span, ctx, state)
            };
            let mut call_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
            call_args.push(obj.clone());
            call_args.extend_from_slice(args);
            let mut call_arg_spans: Vec<Span> = Vec::with_capacity(args_indexes.len() + 1);
            call_arg_spans.push(obj_span);
            call_arg_spans.extend_from_slice(args_indexes);
            return handle_user_function(
                name,
                fn_id,
                output,
                v,
                ctx,
                state,
                tgt_id,
                &call_args,
                fn_span,
                &call_arg_spans,
                &call_type_args,
                None,
            );
        }
        // No method of that name: a field holding a function is called through
        // the same dot, so `obj.field(x)` calls what the field holds. The
        // method lookup above wins, so a field never shadows one.
        if struct_field_holds_fn(struct_id, name, state) {
            let callee =
                Expr::GetStructField(Box::new(obj.clone()), SmolStr::new(name), obj_span, fn_span);
            return handle_value_call(
                output,
                v,
                ctx,
                state,
                tgt_id,
                &callee,
                args,
                fn_span,
                args_indexes,
            );
        }
        // Neither a method nor a field holding a function: builtin methods only
        // apply to strings/arrays/maps/numbers, so this is unambiguously a
        // missing method rather than a mistyped builtin.
        error_no_such_method(name, &struct_name, fn_span, ctx.file_idx, state.sources);
    }

    // An enum receiver dispatches to an `impl` method exactly like a struct: the
    // mangled free function `Enum#method(recv, args...)`.
    if let DataType::Enum(enum_id) = obj_type {
        let enum_name = state.enums[enum_id as usize].name.clone();
        let mangled = mangle_method(&enum_name, name);
        if let Some(fn_id) = state.fns.iter().position(|f| f.name == mangled) {
            let call_type_args = if type_args.is_empty() {
                Vec::new()
            } else {
                resolve_call_type_args(fn_id, name, type_args, fn_span, ctx, state)
            };
            let mut call_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
            call_args.push(obj.clone());
            call_args.extend_from_slice(args);
            let mut call_arg_spans: Vec<Span> = Vec::with_capacity(args_indexes.len() + 1);
            call_arg_spans.push(obj_span);
            call_arg_spans.extend_from_slice(args_indexes);
            return handle_user_function(
                name,
                fn_id,
                output,
                v,
                ctx,
                state,
                tgt_id,
                &call_args,
                fn_span,
                &call_arg_spans,
                &call_type_args,
                None,
            );
        }
        error_no_such_method(name, &enum_name, fn_span, ctx.file_idx, state.sources);
    }

    // A builtin-typed receiver (string/list/map/number) picks up methods from
    // `impl` blocks naming the builtin type: `arr.sum()` lowers to `list#sum`
    // with the receiver as argument 0, reusing the ordinary user-function call
    // path. The `std` collection modules define their helpers this way. Type
    // arguments resolve against the method's own type parameters, exactly as
    // on a struct method.
    if let Some(fn_id) = impl_method_on_builtin(name, &obj_type, args, v, ctx, state) {
        let call_type_args = if type_args.is_empty() {
            Vec::new()
        } else {
            resolve_call_type_args(fn_id, name, type_args, fn_span, ctx, state)
        };
        let mut call_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
        call_args.push(obj.clone());
        call_args.extend_from_slice(args);
        let mut call_arg_spans: Vec<Span> = Vec::with_capacity(args_indexes.len() + 1);
        call_arg_spans.push(obj_span);
        call_arg_spans.extend_from_slice(args_indexes);
        return handle_user_function(
            name,
            fn_id,
            output,
            v,
            ctx,
            state,
            tgt_id,
            &call_args,
            fn_span,
            &call_arg_spans,
            &call_type_args,
            None,
        );
    }

    // Not a struct receiver: fall back to the builtin methods (string/array/
    // map/number library calls). An unknown name there reports a clean error.
    // None of them is generic, so type arguments written on one are an error.
    if !type_args.is_empty() {
        error_type_args_on_builtin_method(fn_span, ctx.file_idx, name, state.sources);
    }
    let id = obj
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();

    // The receiver's register stays allocated for as long as the method is
    // being lowered, because the method's arguments are compiled inside that
    // call. Releasing it first hands the allocator a register the method still
    // reads, and an argument that needs one takes it: `s.items.push(s.n)` then
    // writes `s.n` over the list the push was meant to grow.
    let result = builtin_methods(
        name,
        id,
        obj_type,
        output,
        v,
        ctx,
        state,
        tgt_id,
        obj,
        args,
        obj_span,
        fn_span,
        args_indexes,
    );
    state.free_reg(id, v);
    result
}

/// The operator method a call site resolved to, and how the call reaches it.
pub struct OperatorCall {
    fn_id: usize,
    /// The mangled symbol, which is what a diagnostic about the call names.
    name: SmolStr,
    /// The operands in the order the call passes them, which is the order they
    /// are evaluated in. The derived `>` swaps them, so its right operand runs
    /// first.
    args: Vec<Expr>,
    arg_spans: Vec<Span>,
    /// The type the operator produces.
    return_type: DataType,
    /// `!=` is `==` with the answer flipped.
    negated: bool,
}

/// Resolves an operator on a struct or enum operand to the method that type
/// defines for it, or answers `None` when it defines none.
///
/// Only the left operand's type is consulted, so `1 + v` is the ordinary
/// operator error however `Vec2` defines `+`: there is one spelling of an
/// operator method and it belongs to the type on the left. A `None` answer
/// leaves the caller to report the operator the way it always has.
#[allow(clippy::too_many_arguments)]
pub fn resolve_operator_method(
    symbol: &str,
    l: &Expr,
    r: Option<&Expr>,
    t_l: &DataType,
    t_r: &DataType,
    span_l: Span,
    span_r: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<OperatorCall> {
    let (method, form) = operator_method(symbol)?;
    let type_name = match t_l {
        DataType::Struct(struct_id) => state.structs[*struct_id as usize].name.clone(),
        DataType::Enum(enum_id) => state.enums[*enum_id as usize].name.clone(),
        _ => return None,
    };
    let mangled = mangle_method(&type_name, method);
    let fn_id = state.fns.iter().position(|f| f.name == mangled)?;

    let (args, arg_spans) = match (r, form) {
        // A unary operator names its operand with the second span, the way
        // its report does: the first covers the operator itself.
        (None, _) => (vec![l.clone()], vec![span_r]),
        // A derived operator is the method applied to the same two types, so
        // both operands have to be of the receiver's type whatever the
        // method's second parameter says.
        (Some(r), OperatorForm::Swapped | OperatorForm::Negated) => {
            if t_l != t_r {
                return None;
            }
            if form == OperatorForm::Swapped {
                (vec![r.clone(), l.clone()], vec![span_r, span_l])
            } else {
                (vec![l.clone(), r.clone()], vec![span_l, span_r])
            }
        }
        (Some(r), OperatorForm::Direct) => {
            // A second parameter left un-annotated means the other operand is
            // of the receiver's type; a declared one accepts what it declares,
            // which is how `v * 2.0` reaches `fn *(self, k: float)`.
            let declared = state.fns[fn_id].args.get(1).and_then(|(_, t)| t.as_ref());
            if declared.is_none() && t_l != t_r {
                return None;
            }
            (vec![l.clone(), r.clone()], vec![span_l, span_r])
        }
    };

    let span = span_l.extend(span_r);
    if state.fns[fn_id].returns_null {
        cold_path();
        error_operator_method(
            "Operator method returns nothing",
            &format!(
                "The {method} method on {type_name} hands nothing back, so this operator has no value"
            ),
            "An operator produces a value, so the method it reaches returns one.",
            "operator_method_return_type",
            span,
            ctx.file_idx,
            state.sources,
        );
    }

    let arg_types: Vec<DataType> = args.iter().map(|a| a.infer_type(v, ctx, state)).collect();
    let return_type = infer_user_fn_return_type(fn_id, &arg_types, &[], &mangled, v, ctx, state);
    if operator_method_answers_bool(method) && return_type != DataType::Bool {
        cold_path();
        let actual = state.type_names().of(&return_type);
        error_operator_method(
            "Operator method returns the wrong type",
            &format!("The {method} method on {type_name} returns {actual}, not bool"),
            "The ==, < and <= methods answer a question about two values, so each hands back a bool. !=, > and >= are derived from them.",
            "operator_method_return_type",
            span,
            ctx.file_idx,
            state.sources,
        );
    }

    Some(OperatorCall {
        fn_id,
        name: mangled,
        args,
        arg_spans,
        return_type,
        negated: form == OperatorForm::Negated,
    })
}

/// The type an operator produces on a receiver that defines a method for it.
#[allow(clippy::too_many_arguments)]
pub fn infer_operator_method(
    symbol: &str,
    l: &Expr,
    r: Option<&Expr>,
    t_l: &DataType,
    t_r: &DataType,
    span_l: Span,
    span_r: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<DataType> {
    resolve_operator_method(symbol, l, r, t_l, t_r, span_l, span_r, v, ctx, state)
        .map(|call| call.return_type)
}

/// Lowers an operator on a struct or enum operand to a call of the method that
/// type defines for it, through the path an ordinary method call takes: the
/// VM sees a plain function call and needs no instruction of its own.
#[allow(clippy::too_many_arguments)]
pub fn compile_operator_method(
    symbol: &str,
    l: &Expr,
    r: Option<&Expr>,
    t_l: &DataType,
    t_r: &DataType,
    span_l: Span,
    span_r: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> Option<u16> {
    let call = resolve_operator_method(symbol, l, r, t_l, t_r, span_l, span_r, v, ctx, state)?;
    let span = span_l.extend(span_r);
    let result = handle_user_function(
        &call.name,
        call.fn_id,
        output,
        v,
        ctx,
        state,
        if call.negated { None } else { tgt_id },
        &call.args,
        span,
        &call.arg_spans,
        &[],
        None,
    );
    if !call.negated {
        return result;
    }
    let answered = result.unwrap_id();
    state.free_reg(answered, v);
    let id = state.alloc_reg_tgt(tgt_id);
    output.push(Instr::NegBool(answered, id));
    Some(id)
}
