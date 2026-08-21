use super::expr::{Expr, Span};
use crate::compiler::UnwrapId;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_data::{Ctx, State};
use crate::compiler::compiler_errors::error_no_such_method;
use crate::compiler::compiler_errors::error_type_args_on_builtin_method;
use crate::compiler::expr::mangle_method;
use crate::compiler::functions::user_functions::handle_user_function;
use crate::compiler::type_system::DataType;
use crate::compiler::type_system::TypeExpr;
use crate::compiler::type_system::resolve_call_type_args;
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
//
// `pub(crate)` (not private) so the sibling `type_system` module can reuse the
// routing decision for return-type inference; clippy's `redundant_pub_crate`
// does not account for that cross-module access.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn impl_method_on_builtin(
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
    let name = namespace[namespace.len() - 1].as_str();

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
            );
        }
        // A struct value with no matching impl method: builtin methods only
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
    state.free_reg(id, v);

    builtin_methods(
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
    )
}
