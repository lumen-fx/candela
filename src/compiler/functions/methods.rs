use super::expr::{Expr, Span};
use crate::compiler::UnwrapId;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_data::{Ctx, State};
use crate::compiler::compiler_errors::error_no_such_method;
use crate::compiler::expr::mangle_method;
use crate::compiler::functions::user_functions::handle_user_function;
use crate::compiler::type_system::DataType;
use crate::instr::Instr;
use builtin_methods::builtin_methods;
use smol_strc::SmolStr;

#[path = "builtin/builtin_methods.rs"]
mod builtin_methods;

/// Resolves an array-receiver method call to a `std/list` helper when the
/// method is one of the collection higher-order functions (`map`, `filter`,
/// `reduce`, `each`, `any`, `all`, `sort_by`) or the reductions/slicers the
/// module provides (`first`, `last`, `sum`, ...). Returns the list function id
/// so the caller lowers `arr.map(f)` to `list::map(arr, f)`. `find` routes here
/// only when its argument is a function value (the predicate form); the value
/// form stays on the builtin index search. Returns `None` when the method is not
/// a list helper, the receiver is not an array, or the prelude did not load.
//
// `pub(crate)` (not private) so the sibling `type_system` module can reuse the
// routing decision for return-type inference; clippy's `redundant_pub_crate`
// does not account for that cross-module access.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn routed_list_method(
    name: &str,
    obj_type: &DataType,
    args: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<usize> {
    let is_array = match obj_type {
        DataType::Array(_) => true,
        DataType::Union(union) => union.iter().all(|t| matches!(t, DataType::Array(_))),
        _ => false,
    };
    if !is_array {
        return None;
    }
    let route = match name {
        "map" | "filter" | "reduce" | "each" | "any" | "all" | "sort_by" | "first" | "last"
        | "is_empty" | "sum" | "product" | "min" | "max" | "index_of" | "count" | "unique"
        | "chunk" | "take" | "drop" => true,
        // `find(value)` is the builtin index search; `find(predicate)` is the
        // list helper returning the matching element.
        "find" => args.len() == 1 && matches!(args[0].infer_type(v, ctx, state), DataType::Fn(_)),
        _ => false,
    };
    if !route {
        return None;
    }
    state
        .namespace
        .try_find_function(&[SmolStr::from("list")], name)
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
) -> Option<u16> {
    let name = namespace[namespace.len() - 1].as_str();

    let obj_type = obj.infer_type(v, ctx, state);

    // A method call `recv.method(args)` on a struct value lowers to a call of
    // the mangled free function `Type#method(recv, args...)`. The receiver's
    // static type picks the type-unique symbol, so `Point.len()` and `Str.len()`
    // resolve to distinct functions and neither collides with a free `fn len`.
    // Field access (`recv.method` without parens) never reaches here -- the
    // parser only produces an `ObjFunctionCall` when the call parentheses are
    // present.
    if let DataType::Struct(struct_id) = obj_type {
        let struct_name = state.structs[struct_id as usize].name.clone();
        let mangled = mangle_method(&struct_name, name);
        if let Some(fn_id) = state.fns.iter().position(|f| f.name == mangled) {
            // Prepend the receiver as argument 0, then reuse the ordinary
            // user-function call path -- the VM sees a normal function call.
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
            );
        }
        error_no_such_method(name, &enum_name, fn_span, ctx.file_idx, state.sources);
    }

    // An array-receiver collection method (`arr.map(f)`, `arr.reduce(init, f)`,
    // ...) lowers to the `std/list` helper of the same name with the receiver
    // as argument 0, reusing the ordinary user-function call path.
    if let Some(fn_id) = routed_list_method(name, &obj_type, args, v, ctx, state) {
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
        );
    }

    // Not a struct receiver: fall back to the builtin methods (string/array/
    // map/number library calls). An unknown name there reports a clean error.
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
