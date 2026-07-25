use super::expr::Expr;
use super::expr::Span;
use super::type_system::DataType;
use crate::compiler::UnwrapId;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::check_args;
use crate::compiler::compiler_errors::error_function_arg_invalid_type_multiple;
use crate::compiler::compiler_errors::error_unknown_function_in_namespace;
use crate::instr::Instr;
use builtin_functions::builtin_functions;
use fs_lib_functions::fs_lib_functions;
use smol_strc::SmolStr;
use std::slice;
use user_functions::handle_user_function;

// `pub(crate)` (not the default private) so the sibling `methods` module can
// reach `handle_user_function` to lower method calls. clippy's
// `redundant_pub_crate` does not account for that cross-module access.
#[allow(clippy::redundant_pub_crate)]
pub(crate) mod user_functions;

#[path = "builtin/builtin_functions.rs"]
mod builtin_functions;

#[path = "fs/fs_lib_functions.rs"]
mod fs_lib_functions;

#[cfg(target_arch = "wasm32")]
use crate::errors::wasm_error;

pub fn check_arg_type(
    fn_name: &str,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    args: &[Expr],
    args_indexes: &[Span],
    arg_idx: usize,
    expected: &[DataType],
) {
    let inferred = args[arg_idx].infer_type(v, ctx, state);
    let matches = if let DataType::Union(polytype) = &inferred {
        polytype.iter().all(|x| expected.contains(x))
    } else {
        expected.contains(&inferred)
    };
    if !matches {
        error_function_arg_invalid_type_multiple(
            &inferred,
            expected,
            args_indexes[arg_idx],
            fn_name,
            None,
            ctx.file_idx,
            state.sources,
        )
    }
}

pub fn handle_functions(
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    tgt_id: Option<u16>,
    // method call data
    args: &[Expr],
    namespace: &[SmolStr],
    span: Span,
    args_indexes: &[Span],
) -> Option<u16> {
    let len = namespace.len() - 1;
    let fn_name = namespace[len].as_str();
    let namespace = &namespace[0..len];
    if namespace.is_empty() {
        builtin_functions(
            fn_name,
            output,
            v,
            ctx,
            state,
            tgt_id,
            args,
            span,
            args_indexes,
        )
    } else if namespace == ["fs"] {
        #[cfg(target_arch = "wasm32")]
        wasm_error("WASM does not support the file system library");

        fs_lib_functions(
            fn_name,
            output,
            v,
            ctx,
            state,
            tgt_id,
            args,
            span,
            args_indexes,
        )
    } else if let Some((fn_args, returns_null, dyn_id, is_host, is_variadic)) = state
        .dyn_libs
        .iter()
        .find(|l| l.name == namespace[0])
        .and_then(|lib| {
            lib.fns.iter().find(|x| x.name == fn_name).map(|sig| {
                (
                    sig.args.clone(),
                    sig.return_type == DataType::Null,
                    sig.id,
                    lib.is_host,
                    sig.variadic,
                )
            })
        })
    {
        // A variadic host fn accepts any argument count and any types, so its
        // arity/type checks are skipped; every supplied argument is still
        // compiled and stored, and the registered closure receives them all.
        if !is_variadic {
            check_args(
                args,
                fn_args.len(),
                fn_name,
                span,
                state.sources,
                ctx.file_idx,
            );
            for (i, a) in fn_args.iter().enumerate() {
                check_arg_type(
                    fn_name,
                    v,
                    ctx,
                    state,
                    args,
                    args_indexes,
                    i,
                    slice::from_ref(a),
                );
            }
        }

        for arg in args {
            let arg_id = arg
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            output.push(Instr::StoreFuncArg(arg_id));
            // This may break stuff
            state.free_reg(arg_id, v);
            *state.allocated_arg_count += 1;
        }

        let register_id = if returns_null {
            0
        } else {
            state.alloc_reg_tgt(tgt_id)
        };
        if is_host {
            output.push(Instr::CallHostFunc(dyn_id, register_id));
        } else {
            output.push(Instr::CallDynamicLibFunc(dyn_id, register_id));
        }
        state.add_to_src(ctx, output, span);
        if returns_null {
            None
        } else {
            Some(register_id)
        }
    } else if let Some(fn_id) =
        state
            .namespace
            .find_function(namespace, fn_name, span, ctx.file_idx, state.sources)
    {
        handle_user_function(
            fn_name,
            fn_id,
            output,
            v,
            ctx,
            state,
            tgt_id,
            args,
            span,
            args_indexes,
        )
    } else {
        error_unknown_function_in_namespace(
            fn_name,
            state.namespace,
            namespace,
            span,
            ctx.file_idx,
            state.sources,
        );
    }
}
