use super::super::expr::Expr;
use super::super::expr::Span;
use super::super::type_system::DataType;
use super::check_arg_type;
use super::store_call_args;
use crate::compiler::UnwrapId;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::check_args;
use crate::instr::Instr;
use crate::instr::LibFunc;
use crate::instr::LibFuncVoid;

/// What a primitive compiles to: an instruction that writes a result, or one
/// that writes none.
#[derive(Clone, Copy)]
enum Lowering {
    Value(LibFunc),
    Void(LibFuncVoid),
}

/// The native primitives `std/fs` and `std/event` are written on: what each
/// compiles to, and the type each argument has to be (`Unknown` takes any).
///
/// Keep in step with `std_primitive_return_type` in the type system, which
/// types the calls.
fn primitive(name: &str) -> Option<(Lowering, &'static [DataType])> {
    use DataType::{Bool, Int, String as Str, Unknown};
    use Lowering::{Value, Void};
    const PATH: &[DataType] = &[Str];
    const TWO_PATHS: &[DataType] = &[Str, Str];
    Some(match name {
        "fs_read" => (Value(LibFunc::FsRead), PATH),
        "fs_exists" => (Value(LibFunc::FsExists), PATH),
        "fs_is_dir" => (Value(LibFunc::FsIsDir), PATH),
        "fs_list" => (Value(LibFunc::FsList), PATH),
        "fs_read_bytes" => (Value(LibFunc::FsReadBytes), PATH),
        "fs_digest" => (Value(LibFunc::FsDigest), TWO_PATHS),
        "fs_write" => (Void(LibFuncVoid::FsWrite), TWO_PATHS),
        "fs_append" => (Void(LibFuncVoid::FsAppend), TWO_PATHS),
        "fs_delete" => (Void(LibFuncVoid::FsDelete), PATH),
        "fs_delete_dir" => (Void(LibFuncVoid::FsDeleteDir), PATH),
        "fs_mkdir" => (Void(LibFuncVoid::FsMkdir), PATH),
        "fs_copy" => (Void(LibFuncVoid::FsCopy), TWO_PATHS),
        "fs_write_bytes" => (Void(LibFuncVoid::FsWriteBytes), &[Str, Unknown]),
        "event_source" => (Value(LibFunc::EventSource), &[]),
        "event_next" => (Value(LibFunc::EventNext), &[Bool]),
        "event_on" => (Void(LibFuncVoid::EventOn), &[Int, Unknown]),
        _ => return None,
    })
}

/// Whether `name` is one of the primitives [`std_lib_functions`] lowers.
#[must_use]
pub fn is_std_primitive(name: &str) -> bool {
    primitive(name).is_some()
}

/// Lowers a call to one of the primitives a standard library module is written
/// on. Answers the register the result is in, or `None` for a primitive that
/// returns nothing (and for a name that is not a primitive, which the caller
/// has ruled out).
pub fn std_lib_functions(
    name: &str,
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    tgt_id: Option<u16>,
    args: &[Expr],
    span: Span,
    args_indexes: &[Span],
) -> Option<u16> {
    let (lowering, params) = primitive(name)?;
    check_args(args, params.len(), name, span, state.sources, ctx.file_idx);
    for (idx, expected) in params.iter().enumerate() {
        check_arg_type(
            name,
            v,
            ctx,
            state,
            args,
            args_indexes,
            idx,
            std::slice::from_ref(expected),
        );
    }
    let ids: Vec<u16> = args
        .iter()
        .map(|arg| {
            arg.compile(v, ctx, state, output, None, false, true)
                .unwrap_id()
        })
        .collect();
    let first = ids.first().copied().unwrap_or(0);
    let result = match lowering {
        Lowering::Value(func) => {
            // A second argument travels as a stored argument the instruction
            // pops.
            if let Some(&second) = ids.get(1) {
                store_call_args(&[second], v, state, output);
            }
            state.free_reg(first, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(func, first, output_id));
            Some(output_id)
        }
        Lowering::Void(func) => {
            let second = ids.get(1).copied().unwrap_or(0);
            for &id in &ids {
                state.free_reg(id, v);
            }
            output.push(Instr::CallLibFuncVoid(func, first, second));
            None
        }
    };
    state.add_to_src(ctx, output, span);
    result
}
