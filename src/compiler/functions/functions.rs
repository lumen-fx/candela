use super::expr::Expr;
use super::expr::Span;
use super::type_system::DataType;
use super::type_system::TypeExpr;
use super::type_system::resolve_generic_call;
use super::type_system::resolve_generic_variant;
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

/// Compiles each argument expression and returns the register holding each
/// result, in argument order.
///
/// The registers stay allocated: freeing one before the rest are compiled lets
/// the allocator hand the same register to a later argument, which overwrites
/// the earlier value. [`store_call_args`] frees them once the operands are
/// stored.
//
// `pub(crate)` (not private) so the sibling `methods` module lowers its
// argument runs the same way. clippy's `redundant_pub_crate` does not account
// for that cross-module access.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn compile_call_args(
    args: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> Vec<u16> {
    args.iter()
        .map(|arg| {
            arg.compile(v, ctx, state, output, None, false, true)
                .unwrap_id()
        })
        .collect()
}

/// Emits the `StoreFuncArg` run for `arg_ids` and releases their registers.
///
/// The VM collects `StoreFuncArg` operands in one scratch list that the next
/// call instruction consumes and clears, so the run has to sit directly before
/// that instruction: any nested call emitted in between would take the operands
/// stored so far as its own arguments. Compile every argument first, then call
/// this.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn store_call_args(
    arg_ids: &[u16],
    v: &[Variable],
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    for &arg_id in arg_ids {
        output.push(Instr::StoreFuncArg(arg_id));
        state.free_reg(arg_id, v);
        *state.allocated_arg_count += 1;
    }
}

/// Checks argument `arg_idx` against the types the callee accepts.
///
/// An `any` (Unknown) expected type is a wildcard: it says the position holds a
/// dynamic value, so every argument fits. That is what a downcast collection
/// (`as_map`, `as_list`) hands its entries, and what an `any` annotation means
/// on a parameter or an enum payload.
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
    if expected.iter().any(|t| matches!(t, DataType::Unknown)) {
        return;
    }
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

#[allow(clippy::too_many_arguments)]
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
    type_args: &[TypeExpr],
) -> Option<u16> {
    // A call written with type arguments names either a variant of a generic
    // enum (`Slot<int>::Filled(x)`) or a generic function. Both resolve against
    // the instantiation the arguments give, so neither reaches the built-in and
    // dynamic-library paths below.
    if !type_args.is_empty() {
        if namespace.len() >= 2 {
            let (enum_id, variant_idx) =
                resolve_generic_variant(namespace, type_args, span, ctx, state);
            return Some(crate::compiler::compile_enum_construction(
                enum_id,
                variant_idx,
                args,
                span,
                args_indexes,
                v,
                ctx,
                state,
                output,
            ));
        }
        let fn_name = namespace[namespace.len() - 1].clone();
        let (fn_id, call_type_args) = resolve_generic_call(&fn_name, type_args, span, ctx, state);
        return handle_user_function(
            &fn_name,
            fn_id,
            output,
            v,
            ctx,
            state,
            tgt_id,
            args,
            span,
            args_indexes,
            &call_type_args,
        );
    }
    // A qualified enum-variant construction (`Color::Red(x)`, `Option::Some(v)`)
    // is intercepted before the namespaced-function resolution below, which
    // would otherwise treat the enum name as a module namespace and error.
    if namespace.len() >= 2
        && let Some((enum_id, variant_idx)) =
            crate::compiler::resolve_enum_variant(namespace, state)
    {
        return Some(crate::compiler::compile_enum_construction(
            enum_id,
            variant_idx,
            args,
            span,
            args_indexes,
            v,
            ctx,
            state,
            output,
        ));
    }
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

        // Compile every argument first, then emit the whole `StoreFuncArg` run
        // immediately before the call. The VM accumulates those operands in a
        // single scratch list that each call consumes and clears, so a nested
        // call must not run between the outer call's first stored operand and
        // the call itself. Registers stay allocated until the run is emitted so
        // one argument cannot reuse the register of an earlier one.
        let arg_ids = compile_call_args(args, v, ctx, state, output);
        store_call_args(&arg_ids, v, state, output);

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
            &[],
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
