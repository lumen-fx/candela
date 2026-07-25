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
