use super::super::expr::Expr;
use super::super::expr::Span;
use super::super::type_system::DataType;
use super::super::type_system::format_detailed;
use super::check_arg_type;
use super::user_functions::handle_user_function;
use crate::compiler::UnwrapId;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::check_args;
use crate::compiler::compiler_errors::check_args_range;
use crate::compiler::compiler_errors::error_unknown_function;
use crate::data::Data;
use crate::instr::Instr;
use crate::instr::LibFunc;
use smol_strc::SmolStr;

pub fn builtin_functions(
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
    match name {
        "print" => {
            for arg in args {
                let id = arg
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id();
                output.push(Instr::Print(id));
                state.free_reg(id, v);
            }
            None
        }
        "type" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            let infered = args[0].infer_type(v, ctx, state);
            state.registers.push(Data::p_str(
                format_detailed(&infered, state).as_str(),
                &mut state.pools.strings,
            ));
            Some((state.registers.len() - 1) as u16)
        }
        "float" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            check_arg_type(
                name,
                v,
                ctx,
                state,
                args,
                args_indexes,
                0,
                &[DataType::String, DataType::Int],
            );
            let id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Float, id, output_id));
            state.add_to_src(ctx, output, span);
            Some(output_id)
        }
        "int" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            check_arg_type(
                name,
                v,
                ctx,
                state,
                args,
                args_indexes,
                0,
                &[DataType::String, DataType::Float],
            );
            let id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Int, id, output_id));
            state.add_to_src(ctx, output, span);
            Some(output_id)
        }
        "str" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            let id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Str, id, output_id));
            Some(output_id)
        }
        "bool" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            check_arg_type(
                name,
                v,
                ctx,
                state,
                args,
                args_indexes,
                0,
                &[DataType::String],
            );
            let id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Bool, id, output_id));
            state.add_to_src(ctx, output, span);
            Some(output_id)
        }
        "input" => {
            check_args_range(
                args,
                0,
                1,
                name,
                args_indexes,
                ctx.file_idx,
                state.sources,
                span,
            );
            let id = if args.is_empty() {
                state
                    .registers
                    .push(Data::p_str("", &mut state.pools.strings));
                (state.registers.len() - 1) as u16
            } else {
                check_arg_type(
                    name,
                    v,
                    ctx,
                    state,
                    args,
                    args_indexes,
                    0,
                    &[DataType::String],
                );
                args[0]
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id()
            };
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Input, id, output_id));
            Some(output_id)
        }
        "range" => {
            check_args_range(
                args,
                1,
                2,
                name,
                args_indexes,
                ctx.file_idx,
                state.sources,
                span,
            );
            check_arg_type(name, v, ctx, state, args, args_indexes, 0, &[DataType::Int]);
            if args.len() != 1 {
                check_arg_type(name, v, ctx, state, args, args_indexes, 1, &[DataType::Int]);
            }

            let id_first_arg = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            let source_reg_id = if args.len() == 1 {
                id_first_arg
            } else {
                let id_second_arg = args[1]
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id();
                output.push(Instr::StoreFuncArg(id_first_arg));
                *state.allocated_arg_count += 1;
                id_second_arg
            };
            state.free_reg(id_first_arg, v);
            state.free_reg(source_reg_id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Range, source_reg_id, output_id));
            Some(output_id)
        }
        "the_answer" => {
            check_args(args, 0, name, span, state.sources, ctx.file_idx);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::TheAnswer, 0, output_id));
            Some(output_id)
        }
        "argv" => {
            check_args(args, 0, name, span, state.sources, ctx.file_idx);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::Argv, 0, output_id));
            Some(output_id)
        }
        "exit" => {
            check_args_range(
                args,
                0,
                1,
                name,
                args_indexes,
                ctx.file_idx,
                state.sources,
                span,
            );
            let halt_code = if args.is_empty() {
                0
            } else {
                check_arg_type(name, v, ctx, state, args, args_indexes, 0, &[DataType::Int]);
                args[0]
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id()
            };
            output.push(Instr::Halt(halt_code));
            None
        }
        // Runtime type tests and checked downcasts on an `any` value. `is_*`
        // returns a bool; `as_*` returns the value typed concretely (the type
        // checker assigns the target type) and raises a catchable error when the
        // runtime type differs. `json_parse`/`json_stringify` back `std/json`.
        "is_int" | "is_float" | "is_str" | "is_bool" | "is_list" | "is_map" | "is_null"
        | "as_int" | "as_float" | "as_str" | "as_bool" | "as_list" | "as_map"
        | "json_stringify" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            let (libfunc, throws) = match name {
                "is_int" => (LibFunc::IsIntVal, false),
                "is_float" => (LibFunc::IsFloatVal, false),
                "is_str" => (LibFunc::IsStrVal, false),
                "is_bool" => (LibFunc::IsBoolVal, false),
                "is_list" => (LibFunc::IsListVal, false),
                "is_map" => (LibFunc::IsMapVal, false),
                "is_null" => (LibFunc::IsNullVal, false),
                "as_int" => (LibFunc::AsIntVal, true),
                "as_float" => (LibFunc::AsFloatVal, true),
                "as_str" => (LibFunc::AsStrVal, true),
                "as_bool" => (LibFunc::AsBoolVal, true),
                "as_list" => (LibFunc::AsListVal, true),
                "as_map" => (LibFunc::AsMapVal, true),
                _ => (LibFunc::JsonStringify, false),
            };
            let id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(libfunc, id, output_id));
            if throws {
                state.add_to_src(ctx, output, span);
            }
            Some(output_id)
        }
        "json_parse" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            check_arg_type(
                name,
                v,
                ctx,
                state,
                args,
                args_indexes,
                0,
                &[DataType::String],
            );
            let id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(id, v);
            let output_id = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::CallLibFunc(LibFunc::JsonParse, id, output_id));
            state.add_to_src(ctx, output, span);
            Some(output_id)
        }
        "throw" => {
            check_args(args, 1, name, span, state.sources, ctx.file_idx);
            check_arg_type(
                name,
                v,
                ctx,
                state,
                args,
                args_indexes,
                0,
                &[DataType::String],
            );
            let err_reg_id = args[0]
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            output.push(Instr::ThrowError(err_reg_id));
            state.add_to_src(ctx, output, span);
            None
        }
        fn_name => {
            if let Some(fn_id) =
                state
                    .namespace
                    .find_function(&[], fn_name, span, ctx.file_idx, state.sources)
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
            } else if let Some((enum_id, variant_idx)) = crate::compiler::resolve_enum_variant(
                std::slice::from_ref(&SmolStr::new(fn_name)),
                state,
            ) {
                // An otherwise-unknown call whose name is an enum variant
                // (`Some(x)`, `Ok(v)`) is a variant construction. User functions
                // above keep priority, so a function never gets shadowed.
                Some(crate::compiler::compile_enum_construction(
                    enum_id,
                    variant_idx,
                    args,
                    span,
                    args_indexes,
                    v,
                    ctx,
                    state,
                    output,
                ))
            } else {
                error_unknown_function(fn_name, span, state.namespace, ctx.file_idx, state.sources);
            }
        }
    }
}
