// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use super::super::expr::Expr;
use super::super::expr::Span;
use super::super::registers::get_tgt_ids;
use super::super::registers::move_to_id;
use super::super::type_system::DataType;
use super::super::type_system::arg_types_specialize_equal;
use super::super::type_system::can_reach;
use super::super::type_system::check_if_returns_void;
use super::super::type_system::fn_bindings;
use super::super::type_system::indirect_return_type;
use super::super::type_system::infer_user_fn_return_type;
use super::super::type_system::instantiations_line_up;
use super::super::type_system::param_type_matches;
use super::super::type_system::pin_empty_literal_bindings;
use super::super::type_system::pinned_arg_types;
use super::super::type_system::push_capture_scope;
use super::super::type_system::specialization_key;
use super::super::type_system::specialized_arg_types;
use super::super::type_system::specialized_return_type;
use super::super::type_system::track_returns;
use crate::compiler::SymbolKind;
use crate::compiler::UnwrapId;
use crate::compiler::compile_expr;
use crate::compiler::compile_fn_value;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::FunctionImpl;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::check_args;
use crate::compiler::compiler_errors::check_args_user_fn;
use crate::compiler::compiler_errors::error_enum;
use crate::compiler::compiler_errors::error_function_arg_invalid_type;
use crate::compiler::compiler_errors::error_invalid_type;
use crate::compiler::flow::branch_target;
use crate::compiler::flow::ends_straight_run;
use crate::compiler::functions::compile_call_args;
use crate::compiler::functions::store_call_args;
use crate::data::Data;
use crate::data::NULL;
use crate::instr::Instr;
use crate::rt::FnValue;
use crate::rt::InstrSrc;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use std::rc::Rc;

/// The register holding the closure value a call by name reaches: the variable
/// of that name, read out of its cell where the variable is itself captured.
///
/// `None` where the name is a declared function rather than a variable, which
/// is a callee with no environment to pass.
fn closure_value_register(
    fn_name: &str,
    v: &[Variable],
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> Option<u16> {
    let var = v
        .iter()
        .rfind(|var| var.name.as_str() == fn_name && matches!(var.var_type, DataType::Fn(_)))?;
    if !var.cell {
        return Some(var.register_id);
    }
    let cell_id = var.register_id;
    let value_id = state.alloc_reg();
    output.push(Instr::LoadCell(cell_id, value_id));
    Some(value_id)
}

#[allow(clippy::too_many_arguments)]
pub fn handle_user_function(
    fn_name: &str,
    fn_id: usize,
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    tgt_id: Option<u16>,
    args: &[Expr],
    span: Span,
    args_indexes: &[Span],
    // The type arguments a generic call named, empty for every other call.
    type_args: &[DataType],
    // The register the callee expression left the closure value in, for a call
    // whose callee is an expression rather than a name.
    env_id: Option<u16>,
) -> Option<u16> {
    let is_recursive = resolve_is_recursive(fn_id, state);

    let fn_returns_null = state.fns[fn_id].returns_null;

    // Check if the arguments are correct
    let args_len = state.fns[fn_id].args.len();
    check_args_user_fn(
        args,
        args_len,
        fn_name,
        ctx.file_idx,
        span,
        (state.fns[fn_id].name_span, state.fns[fn_id].src_file),
        state,
        args_indexes,
    );

    //This inlines dylib wrappers
    // Actual general function inlining is coming soon
    if state.fns[fn_id].code.len() == 1
        && let Expr::ReturnVal(ret) = &state.fns[fn_id].code[0]
        && let Some(Expr::FunctionCall(call_args, namespace, _, _, _)) = &**ret
        && namespace.len() >= 2
        && call_args.len() == args_len
        && call_args
            .iter()
            .zip(state.fns[fn_id].args.iter())
            .all(|(e, (p, _))| matches!(e, Expr::Var(n, _) if n == p))
        && let Some(fn_sig) = state
            .dyn_libs
            .iter()
            .find(|lib| !lib.is_host && lib.name == namespace[namespace.len() - 2])
            .and_then(|lib| {
                lib.fns
                    .iter()
                    .find(|f| f.name == namespace[namespace.len() - 1])
            })
    {
        let dyn_id = fn_sig.id;
        let returns_null = fn_sig.return_type == DataType::Null;
        let expected_arg_types = fn_sig.args.clone();
        for (i, arg) in args.iter().enumerate() {
            let inferred = arg.infer_type(v, ctx, state);
            if inferred != expected_arg_types[i] {
                error_function_arg_invalid_type(
                    &inferred,
                    &expected_arg_types[i],
                    args_indexes[i],
                    fn_name,
                    Some((state.fns[fn_id].name_span, state.fns[fn_id].src_file)),
                    ctx.file_idx,
                    state.sources,
                    state.type_names(),
                )
            }
        }
        // The `StoreFuncArg` run goes directly before the call; see
        // `store_call_args`.
        let arg_ids = compile_call_args(args, v, ctx, state, output);
        store_call_args(&arg_ids, v, state, output);

        let register_id = if returns_null {
            0
        } else {
            state.alloc_reg_tgt(tgt_id)
        };
        output.push(Instr::CallDynamicLibFunc(dyn_id, register_id));
        state.add_to_src(ctx, output, span);
        return Some(register_id);
    }

    // Infer arg types
    let mut infered_arg_types = args
        .iter()
        .map(|arg| arg.infer_type(v, ctx, state))
        .collect::<Vec<DataType>>();

    // A parameter with a `: Type` annotation pins that parameter: the argument
    // must match it. An un-annotated parameter takes whatever the call site
    // passes and specialises on it. An annotation naming a type parameter pins
    // the parameter only once the call names its type arguments; without them
    // the parameter is inferred, so a generic function keeps compiling when it
    // is called without them.
    let declared_arg_types = specialized_arg_types(fn_id, type_args, ctx, state);
    // A parameter declared `fn(...)` holds a function value: the argument is
    // compiled at the types the declaration names, and the body sees the
    // declared type rather than the one function this call happens to pass, so
    // the call inside it dispatches on the value.
    for (i, declared) in declared_arg_types.iter().enumerate() {
        if let Some(declared) = declared
            && declared_holds_fn_signature(declared)
        {
            let declared = declared.clone();
            check_declared_fn(&args[i], &declared, args_indexes[i], output, v, ctx, state);
            infered_arg_types[i] = declared;
        }
    }
    for (i, declared) in declared_arg_types.iter().enumerate() {
        if let Some(declared) = declared
            && !param_type_matches(declared, &infered_arg_types[i], state.generics)
        {
            error_function_arg_invalid_type(
                &infered_arg_types[i],
                declared,
                args_indexes[i],
                fn_name,
                Some((state.fns[fn_id].name_span, state.fns[fn_id].src_file)),
                ctx.file_idx,
                state.sources,
                state.type_names(),
            );
        }
    }

    // A constructor names only the type parameters its payload mentions, so an
    // argument can arrive at a more open instantiation than the parameter
    // declares: `None` is an `Option<any>` where the parameter says
    // `Option<P>`. The declaration is what the body is compiled against, so the
    // payload a match arm binds has the type the parameter names.
    for (i, declared) in declared_arg_types.iter().enumerate() {
        if let Some(declared) = declared
            && infered_arg_types[i] != *declared
            && instantiations_line_up(declared, &infered_arg_types[i], state.generics)
        {
            infered_arg_types[i] = declared.clone();
        }
    }

    // An argument that is an empty array literal carries no element type, so a
    // body specialised on it would see `null` elements. Where the parameter
    // declares what the elements are, that declaration pins the specialisation.
    if let Some(pinned) = pinned_arg_types(&infered_arg_types, &declared_arg_types) {
        infered_arg_types = pinned;
    }

    // The caller's own binding takes the declared type too, when the literal it
    // holds is still empty: the call is what first says what the collection
    // holds, so what the caller reads back out of it afterwards has a type.
    pin_empty_literal_bindings(args, &declared_arg_types, v);

    // Try to check if function has already been compiled for these specific arg
    // types. Function-typed arguments must match by exact Fn id: each distinct
    // function passed to a higher-order function is a distinct specialization, so
    // the loose type-compatibility `==` (which treats all Fn as equal) cannot be
    // used as the specialization key.
    // The type arguments are part of the key: a type parameter no argument
    // mentions (`fn signal<T>(name: string)`) still changes what the body
    // builds, so two calls that pass the same argument types are two
    // specialisations.
    let fn_impl_idx = state.fns[fn_id].impls.iter().position(|fn_impl| {
        arg_types_specialize_equal(&fn_impl.arg_types, &infered_arg_types)
            && arg_types_specialize_equal(&fn_impl.type_args, type_args)
    });

    if fn_impl_idx.is_none() {
        // If it hasn't, compile it (which adds it to the function's implementation list)

        // Clone only when compiling a new specialisation
        let fn_args = state.fns[fn_id]
            .args
            .iter()
            .map(|(a, _)| a.clone())
            .collect::<Vec<SmolStr>>();
        let fn_code: Rc<[Expr]> = Rc::clone(&state.fns[fn_id].code);
        compile_function(
            output,
            v,
            ctx,
            state,
            fn_id,
            &fn_args,
            fn_name,
            &infered_arg_types,
            type_args,
            args,
            &fn_code,
            is_recursive,
            state.fns[fn_id].src_file,
            false,
        );
    }
    // Re-derive index after possible mutation
    let fn_impl_idx = fn_impl_idx.unwrap_or_else(|| state.fns[fn_id].impls.len() - 1);
    let loc = state.fns[fn_id].impls[fn_impl_idx].loc;
    let args_loc_len = state.fns[fn_id].impls[fn_impl_idx].args_loc.len();

    // A specialisation a value can reach shares its parameter registers with
    // every other one, so a call into it overwrites the caller's however the
    // call was written. Saving the caller's registers is therefore decided by
    // the specialisation this call lands on, not by the function it names.
    let uses_frame = is_recursive || state.fns[fn_id].impls[fn_impl_idx].indirect;
    let saveframe_loc = output.len();
    // Only a call that saves registers is keyed by call site, and takes an
    // entry here. The list stays empty until the enclosing body finishes
    // compiling and `compile_function` can see which registers are still read
    // after the call returns.
    let callsite_id = if uses_frame {
        let id = state.callsite_registers.len() as u16;
        state.callsite_registers.push(Vec::new());
        output.push(Instr::SaveFrame(0, 0, 0));
        *state.allocated_call_depth += 2;
        Some(id)
    } else {
        None
    };
    // A release build copies a small body in place of the call. Its
    // arguments are then worked out into registers of their own, which the
    // copy reads in place of the parameters.
    let inline = !uses_frame && state.fns[fn_id].impls[fn_impl_idx].inline_body.is_some();
    let mut bindings: Vec<(u16, u16)> = Vec::new();
    // Move evaluated call args into the expected arg slots.
    // A parameter register belongs to the callee, and a later argument that
    // calls a function can reach this callee again and overwrite it. So an
    // argument before the last one that may call is held in a register of its
    // own and moved into place once every argument has been worked out. A
    // call that saves registers keeps its parameters across such a call
    // already, so only a plain call needs this.
    let last_calling = if uses_frame {
        None
    } else {
        (0..args_loc_len)
            .rev()
            .find(|&i| i > 0 && expr_may_call(&args[i], v, ctx, state))
    };
    let mut held: Vec<(u16, u16)> = Vec::new();
    #[allow(clippy::needless_range_loop)]
    for i in 0..args_loc_len {
        let tgt_id = state.fns[fn_id].impls[fn_impl_idx].args_loc[i];

        // A function argument is settled at compile time, so nothing is
        // moved for it unless it is a closure that captures: that one carries
        // an environment, and the environment is a value like any other. A
        // parameter declared `fn(...)` takes a value either way, since the body
        // dispatches on what it holds.
        if let DataType::Fn(callee_id) = infered_arg_types[i]
            && state.fns[callee_id as usize].captures.is_empty()
        {
            continue;
        }

        if inline {
            let arg_id = if matches!(infered_arg_types[i], DataType::FnValue(_)) {
                compile_fn_value(&args[i], v, ctx, state, output, None)
            } else {
                args[i]
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id()
            };
            bindings.push((tgt_id, arg_id));
            continue;
        }

        if last_calling.is_some_and(|last| i < last) {
            let arg_id = if matches!(infered_arg_types[i], DataType::FnValue(_)) {
                compile_fn_value(&args[i], v, ctx, state, output, None)
            } else {
                args[i]
                    .compile(v, ctx, state, output, None, false, true)
                    .unwrap_id()
            };
            held.push((arg_id, tgt_id));
            continue;
        }

        let start_len = output.len();
        let arg_id = if matches!(infered_arg_types[i], DataType::FnValue(_)) {
            compile_fn_value(&args[i], v, ctx, state, output, Some(tgt_id))
        } else {
            args[i]
                .compile(v, ctx, state, output, Some(tgt_id), false, true)
                .unwrap_id()
        };
        if (output.len() == start_len || !move_to_id(output, tgt_id)) && arg_id != tgt_id {
            output.push(Instr::Mov(arg_id, tgt_id));
        }
    }
    for (arg_id, tgt_id) in held {
        if arg_id != tgt_id {
            output.push(Instr::Mov(arg_id, tgt_id));
        }
    }
    // Hand the callee its environment.
    if let Some(env_loc) = state.fns[fn_id].impls[fn_impl_idx].env_loc {
        let src_id = env_id.or_else(|| closure_value_register(fn_name, v, state, output));
        if let Some(src_id) = src_id {
            output.push(Instr::Mov(src_id, env_loc));
            state.free_reg(src_id, v);
        }
    }
    let return_register_id = if fn_returns_null {
        0
    } else {
        state.alloc_reg_tgt(tgt_id)
    };
    if inline && let Some(body) = state.fns[fn_id].impls[fn_impl_idx].inline_body.clone() {
        inline_call(
            &body,
            &bindings,
            return_register_id,
            fn_returns_null,
            output,
            state,
        );
        return if fn_returns_null {
            None
        } else {
            Some(return_register_id)
        };
    }
    if uses_frame {
        output.push(Instr::CallFuncRecursive(loc, return_register_id));
    } else {
        output.push(Instr::CallFunc(loc, return_register_id));
        *state.allocated_call_depth += 2;
        state.add_to_src(ctx, output, span);
    }

    if uses_frame {
        if !fn_returns_null {
            state.value_callsites.insert(callsite_id.unwrap());
        }
        output[saveframe_loc] = Instr::SaveFrame(
            (output.len() - 1 - saveframe_loc) as u16,
            return_register_id,
            callsite_id.unwrap(),
        );
        // The frame a recursive call pushes is the one the call-depth limit
        // stops at, and the limit reports where it stopped, so the patched
        // instruction gets the call site it belongs to.
        state.add_instr_to_src(ctx, output[saveframe_loc], span);
    }

    if fn_returns_null {
        None
    } else {
        Some(return_register_id)
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_function(
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    function_id: usize,
    fn_args: &[SmolStr],
    fn_name: &str,
    infered_arg_types: &[DataType],
    type_args: &[DataType],
    _args: &[Expr],
    fn_code: &[Expr],
    is_recursive: bool,
    fn_file_idx: u16,
    // Whether a call through a value can reach this specialisation. One that
    // can takes its parameters from the shared indirect registers and returns
    // the way a recursive call does, since every caller saves its registers on
    // the way in.
    indirect: bool,
) {
    // A specialisation a value can reach returns the way a recursive call does:
    // every caller saved its registers on the way in, so every way out has to
    // put them back.
    let is_recursive = is_recursive || indirect;
    // Local vector vars and recorded_types to allow the inner body to type-check correctly
    let mut v_temp: Vec<Variable> = fn_args
        .iter()
        .enumerate()
        .map(|(i, x)| {
            // A specialisation a value can reach reads its parameters where
            // every indirect call writes them; one only reachable by name gets
            // registers of its own.
            let register_id = if indirect {
                state.indirect_arg_register(i)
            } else {
                state.registers.push(NULL);
                (state.registers.len() - 1) as u16
            };
            Variable {
                name: x.clone(),
                register_id,
                cell: false,
                var_type: infered_arg_types[i].clone(),
            }
        })
        .collect();

    // Get the arg destination ids
    let args_loc = v_temp.iter().map(|x| x.register_id).collect::<Vec<u16>>();
    let args_loc_saved = args_loc.clone();

    // Temporarily jump over function to prevent executing it right now
    // This is a placeholder that's modified later on
    output.push(Instr::Jmp(0));
    let jump_idx = output.len() - 1;

    // Record start location for the compiled func body
    let fn_start = output.len();
    let loc = fn_start as u16 + ctx.offset;
    // A value built for this function carries where its body starts, which is
    // known here and read by every indirect call that reaches it.
    if indirect {
        let entry_id = state.fn_entry_register(function_id);
        state.registers[entry_id as usize] = Data::int(i64::from(loc));
    }

    // A closure that captures reads its environment out of one register the
    // call site fills, and takes the cells out of it once, on entry, so the
    // body reads a captured variable through a cell exactly as the scope that
    // declared it does. The captures come first in scope, so a parameter of
    // the same name shadows one.
    let captures = state.fns[function_id].captures.clone();
    let env_loc = (!captures.is_empty()).then(|| {
        if indirect {
            state.indirect_env_register()
        } else {
            state.alloc_reg()
        }
    });
    // A function value starts with where its body begins, so the cells it
    // carries sit one slot further in.
    let mut capture_regs: Vec<u16> = Vec::with_capacity(captures.len());
    if let Some(env_loc) = env_loc {
        let mut with_captures = Vec::with_capacity(captures.len() + v_temp.len());
        for (i, (name, capture_type)) in captures.iter().enumerate() {
            let idx_id = state.const_int_register(i as i64 + 1);
            let cell_id = state.alloc_reg();
            output.push(Instr::GetIndexArray(env_loc, idx_id, cell_id));
            capture_regs.push(cell_id);
            with_captures.push(Variable {
                name: name.clone(),
                register_id: cell_id,
                cell: true,
                var_type: capture_type.clone(),
            });
        }
        with_captures.append(&mut v_temp);
        v_temp = with_captures;
    }

    let v_len_before_args = v.len();
    push_capture_scope(v, &captures);
    let mut anon_fns: Vec<usize> = Vec::new();
    infered_arg_types
        .iter()
        .enumerate()
        .for_each(|(i, infered_type)| {
            if let DataType::Fn(fn_id) = infered_type {
                // The parameter is named in the body, so it is declared in the
                // scope that body resolves in: the file the function was
                // written in, not the one the call site sits in.
                let scope = state.scope_mut(fn_file_idx);
                anon_fns.push(scope.symbols.len());
                scope
                    .symbols
                    .push((fn_args[i].clone(), SymbolKind::Fn(*fn_id)));
                v.push(Variable {
                    name: fn_args[i].clone(),
                    register_id: 0,
                    cell: false,
                    var_type: DataType::Fn(*fn_id),
                });
            } else {
                // 0 => placeholder id, it's never used
                v.push(Variable {
                    name: fn_args[i].clone(),
                    register_id: 0,
                    cell: false,
                    var_type: infered_type.clone(),
                });
            }
        });
    state
        .generics
        .push_frame(fn_bindings(function_id, type_args, state));
    // The body's names resolve in the file the function was written in, the
    // same file its signature resolved in, not the one the call site sits in.
    let fn_ctx = Ctx {
        file_idx: fn_file_idx,
        ..ctx
    };
    let fn_type = track_returns(fn_code, v, fn_ctx, state, fn_name, function_id);
    let return_type = if fn_type.is_empty() {
        // No tracked type means either no value is returned at all, or every
        // returned value was itself dynamic (return-type tracking records no
        // type for `Unknown`). A function handing back an `any` payload is
        // dynamic, not null.
        if check_if_returns_void(fn_code) {
            DataType::Null
        } else {
            DataType::Unknown
        }
    } else {
        // If function returns anything, check if it returns the same thing each time
        DataType::Union(Box::from(fn_type)).check_poly()
    };

    // A `-> Type` annotation pins what the body may hand back. Each
    // specialisation is checked separately, so an un-annotated parameter that
    // makes one call site return a different type is caught at that call site.
    if let Some((declared, declared_span)) = specialized_return_type(function_id, ctx, state)
        && !param_type_matches(&declared, &return_type, state.generics)
    {
        let types = state.type_names();
        error_invalid_type(
            &declared,
            &return_type,
            declared_span,
            None,
            Some(format_args!(
                "Function {fn_name} is declared to return {}",
                types.of(&declared)
            )),
            fn_file_idx,
            state.sources,
            types,
        );
    }

    v.truncate(v_len_before_args);

    // Add this func specialization to the func's metadata
    let func = state.fns.get_mut(function_id).unwrap();
    func.impls.push(FunctionImpl {
        env_loc,
        indirect,
        loc,
        args_loc: Box::from(args_loc.as_slice()),
        arg_types: Box::from(infered_arg_types),
        type_args: Box::from(type_args),
        inline_body: None,
    });
    // Cache the return type
    let key = specialization_key(type_args, infered_arg_types);
    if !func
        .return_type_cache
        .iter()
        .any(|(args, _)| arg_types_specialize_equal(args, &key))
    {
        func.return_type_cache.push((key, return_type));
    }

    // Compile the function into instructions using local vars
    let parsed = compile_expr(
        fn_code,
        &mut v_temp,
        Ctx {
            is_compiling_recursive: is_recursive,
            file_idx: fn_file_idx,
            single_run: false,
            in_function: true,
            offset: ctx.offset + output.len() as u16,
            ..ctx
        },
        state,
    );
    state.generics.pop_bindings();
    for i in anon_fns.into_iter().rev() {
        state.scope_mut(fn_file_idx).symbols.remove(i);
    }

    let mut reserved_registers = get_tgt_ids(&parsed);
    reserved_registers.extend(args_loc);
    // The environment register and the cells taken out of it are written on
    // entry and read for the whole body, so the allocator must not hand them
    // to anything else.
    reserved_registers.extend(env_loc);
    reserved_registers.extend(capture_regs.iter().copied());
    for instr in &parsed {
        match instr {
            Instr::CloneArray(template_reg, _, _)
            | Instr::CloneStruct(template_reg, _)
            | Instr::CloneEnum(template_reg, _)
            | Instr::CloneMap(template_reg, _) => {
                reserved_registers.push(*template_reg);
            }
            _ => {}
        }
    }
    reserved_registers.sort_unstable();
    reserved_registers.dedup();
    state.reserved_registers.extend(reserved_registers);
    state
        .free_registers
        .retain(|reg| !state.reserved_registers.contains(reg));

    // A body that calls through a value saves registers at that call whether or
    // not the static call graph says the function recurses: the value can hold
    // the function making the call.
    let saves_registers = is_recursive
        || parsed
            .iter()
            .any(|instr| matches!(instr, Instr::CallIndirect(_)));
    if saves_registers {
        // The cells the entry took out of the environment count as written by
        // the body: a recursive call fills them again for its own environment,
        // so the caller has to get its own back.
        let mut all_written_regs: Vec<u16> = get_tgt_ids(&parsed);
        all_written_regs.extend(capture_regs.iter().copied());
        // The parameters of a specialisation a value can reach live in
        // registers every other such specialisation uses too, so a call made
        // from this body overwrites them whether or not this body ever wrote
        // them itself.
        if indirect {
            all_written_regs.extend(args_loc_saved.iter().copied());
        }
        all_written_regs.sort_unstable();
        all_written_regs.dedup();

        let args_loc_saved = args_loc_saved.clone();
        set_saved_registers(&parsed, &all_written_regs, &args_loc_saved, state);
    }

    // A release build copies a small body into its call sites instead of
    // calling it. The body stays where it is too, for a call made in a profile
    // or a place that does not inline.
    if state.optimize && !is_recursive && env_loc.is_none() {
        let body = inlinable_body(&parsed);
        let fn_impl = state.fns[function_id].impls.last_mut().unwrap();
        fn_impl.inline_body = body;
    }

    output.extend(parsed);

    output.push(Instr::VoidReturn);

    // Fix the placeholder Jmp(0) to skip over the function body
    *output.get_mut(jump_idx).unwrap() = Instr::Jmp((output.len() - fn_start + 1) as u16);
}

/// Compiles `fn_id` at `arg_types` so a call through a value can reach it.
///
/// The specialisation takes its parameters from the shared indirect registers
/// and records where its body starts in the function's entry register, which is
/// the first slot of every value built for it. A specialisation already
/// compiled for those types is left alone.
pub fn ensure_indirect_impl(
    fn_id: usize,
    arg_types: &[DataType],
    span: Span,
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) {
    if state.fns[fn_id].impls.iter().any(|fn_impl| {
        fn_impl.indirect && arg_types_specialize_equal(&fn_impl.arg_types, arg_types)
    }) {
        return;
    }
    // A value carries one body, so the function behind it is compiled once. A
    // second set of argument types would need a second body and the value has
    // nowhere to say which, so it is reported here rather than running the
    // wrong one.
    if let Some(compiled) = state.fns[fn_id]
        .impls
        .iter()
        .find(|fn_impl| fn_impl.indirect)
    {
        let types = state.type_names();
        let written = compiled
            .arg_types
            .iter()
            .map(|t| types.of(t).to_string())
            .collect::<Vec<String>>()
            .join(", ");
        error_enum(
            "Function value called at two argument types",
            &format!(
                "This function is held in a value and already compiled for ({written}). \
                 A value carries one body, so give each call its own function."
            ),
            span,
            ctx.file_idx,
            state.sources,
        );
    }
    let is_recursive = resolve_is_recursive(fn_id, state);
    let fn_name = state.fns[fn_id].name.clone();
    let fn_args = state.fns[fn_id]
        .args
        .iter()
        .map(|(a, _)| a.clone())
        .collect::<Vec<SmolStr>>();
    let fn_code: Rc<[Expr]> = Rc::clone(&state.fns[fn_id].code);
    let src_file = state.fns[fn_id].src_file;
    compile_function(
        output,
        v,
        ctx,
        state,
        fn_id,
        &fn_args,
        &fn_name,
        arg_types,
        &[],
        &[],
        &fn_code,
        is_recursive,
        src_file,
        true,
    );
}

/// Lowers a call through a function value: the callee is compiled, its
/// arguments go into the registers every such call passes them in, and the jump
/// reads where to go from the value itself.
///
/// The frame is pushed by a `SaveFrame`, so the caller gets its registers back
/// whichever function the value turned out to hold.
#[allow(clippy::too_many_arguments)]
pub fn handle_indirect_call(
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    tgt_id: Option<u16>,
    callee: &Expr,
    fn_type: &FnValue,
    args: &[Expr],
    span: Span,
    args_indexes: &[Span],
) -> Option<u16> {
    let returns_null = matches!(
        indirect_return_type(fn_type, args, v, ctx, state),
        DataType::Null
    );
    let arg_types = indirect_arg_types(fn_type, args, args_indexes, span, v, ctx, state);
    // A variable's set grows as the code writes more functions into it, so a
    // call through one records what it was compiled at: a function that joins
    // the set later is compiled for it too.
    if let Some(set) = fn_type.set
        && let Some(set) = state.indirect_registers.value_sets.get_mut(set as usize)
        && !set.used_at.iter().any(|used| **used == *arg_types)
    {
        set.used_at.push(Box::from(arg_types.as_slice()));
    }
    for candidate in state.fn_value_candidates(fn_type) {
        check_args_user_fn(
            args,
            state.fns[candidate as usize].args.len(),
            &state.fns[candidate as usize].name.clone(),
            ctx.file_idx,
            span,
            (
                state.fns[candidate as usize].name_span,
                state.fns[candidate as usize].src_file,
            ),
            state,
            args_indexes,
        );
        ensure_indirect_impl(candidate as usize, &arg_types, span, output, v, ctx, state);
    }

    // The value is read before the arguments so that an argument expression
    // that calls something cannot land on top of it.
    let callee_id = callee
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    let arg_ids = compile_call_args(args, v, ctx, state, output);

    let saveframe_loc = output.len();
    let callsite_id = state.callsite_registers.len() as u16;
    state.callsite_registers.push(Vec::new());
    output.push(Instr::SaveFrame(0, 0, 0));
    *state.allocated_call_depth += 2;

    for (i, &arg_id) in arg_ids.iter().enumerate() {
        let dest = state.indirect_arg_register(i);
        output.push(Instr::Mov(arg_id, dest));
    }
    for &arg_id in &arg_ids {
        state.free_reg(arg_id, v);
    }
    // The callee finds the value it was called through where every indirect
    // call leaves it, which is where the cells it captured come from.
    let env_dest = state.indirect_env_register();
    output.push(Instr::Mov(callee_id, env_dest));

    let return_register_id = if returns_null {
        0
    } else {
        state.alloc_reg_tgt(tgt_id)
    };
    output.push(Instr::CallIndirect(callee_id));
    state.add_to_src(ctx, output, span);
    if !returns_null {
        state.value_callsites.insert(callsite_id);
    }
    output[saveframe_loc] = Instr::SaveFrame(
        (output.len() - 1 - saveframe_loc) as u16,
        return_register_id,
        callsite_id,
    );
    state.add_instr_to_src(ctx, output[saveframe_loc], span);
    state.free_reg(callee_id, v);

    if returns_null {
        None
    } else {
        Some(return_register_id)
    }
}

/// The types the functions behind a value are compiled at.
///
/// A declared `fn(A, B) -> R` says them, and each argument is checked against
/// what it declares. A type inferred from the functions written into the
/// position says nothing, so the call site's own argument types are what the
/// bodies are compiled against.
pub fn indirect_arg_types(
    fn_type: &FnValue,
    args: &[Expr],
    args_indexes: &[Span],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<DataType> {
    let Some((params, _)) = &fn_type.sig else {
        return args
            .iter()
            .map(|arg| arg.infer_type(v, ctx, state))
            .collect();
    };
    check_args(
        args,
        params.len(),
        "this function",
        span,
        state.sources,
        ctx.file_idx,
    );
    let params = params.clone();
    for (i, declared) in params.iter().enumerate() {
        let inferred = args[i].infer_type(v, ctx, state);
        if !param_type_matches(declared, &inferred, state.generics) {
            error_function_arg_invalid_type(
                &inferred,
                declared,
                args_indexes[i],
                "this function",
                None,
                ctx.file_idx,
                state.sources,
                state.type_names(),
            );
        }
    }
    params.to_vec()
}

/// Checks a function written into a position declared `fn(...) -> R`, and
/// compiles it at the types that declaration names.
///
/// The declaration is what the body is compiled against, so the check is the
/// compile: a closure whose body does not work at the declared parameter types,
/// or hands back something else, is reported where it was written.
/// Whether this function can reach itself, resolved the first time it is
/// compiled and remembered on the function after that.
fn resolve_is_recursive(fn_id: usize, state: &mut State<'_>) -> bool {
    if let Some(is_recursive) = state.fns[fn_id].is_recursive {
        return is_recursive;
    }
    let name = state.fns[fn_id].name.clone();
    let mut visited = FxHashSet::default();
    visited.insert(name.clone());
    let is_recursive = can_reach(&name, &name, state.fns, &mut visited);
    state.fns[fn_id].is_recursive = Some(is_recursive);
    is_recursive
}

pub fn check_declared_fn(
    value: &Expr,
    declared: &DataType,
    span: Span,
    output: &mut Vec<Instr>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) {
    // A `fn(...)` inside a list or a map type reaches the functions written
    // into the literal, so the check follows the declaration into it.
    match (declared, value) {
        (DataType::Array(Some(element)), Expr::Array(items, spans)) => {
            for (i, item) in items.iter().enumerate() {
                let item_span = spans.get(i + 1).copied().unwrap_or(span);
                check_declared_fn(item, element, item_span, output, v, ctx, state);
            }
            return;
        }
        (DataType::Map(entry), Expr::Map(pairs, _)) => {
            if let Some(declared_value) = &entry.1 {
                for (_, _, pair_value, value_span) in pairs {
                    check_declared_fn(
                        pair_value,
                        declared_value,
                        *value_span,
                        output,
                        v,
                        ctx,
                        state,
                    );
                }
            }
            return;
        }
        _ => {}
    }
    let DataType::FnValue(declared) = declared else {
        return;
    };
    let Some((params, declared_return)) = &declared.sig else {
        return;
    };
    let DataType::Fn(fn_id) = value.infer_type(v, ctx, state) else {
        return;
    };
    let fn_id = fn_id as usize;
    let declared_type = DataType::FnValue(declared.clone());
    if state.fns[fn_id].args.len() != params.len() {
        let inferred = DataType::FnValue(Box::new(FnValue::inferred(Box::from([fn_id as u16]))));
        error_invalid_type(
            &declared_type,
            &inferred,
            span,
            None,
            Some(format_args!(
                "This function takes {} argument(s)",
                state.fns[fn_id].args.len()
            )),
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }
    ensure_indirect_impl(fn_id, params, span, output, v, ctx, state);
    let fn_name = state.fns[fn_id].name.clone();
    let return_type = infer_user_fn_return_type(fn_id, params, &[], &fn_name, v, ctx, state);
    if !param_type_matches(declared_return, &return_type, state.generics) {
        let inferred = DataType::FnValue(Box::new(FnValue {
            sig: Some((params.clone(), return_type)),
            candidates: Box::from([fn_id as u16]),
            set: None,
        }));
        error_invalid_type(
            &declared_type,
            &inferred,
            span,
            None,
            None,
            ctx.file_idx,
            state.sources,
            state.type_names(),
        );
    }
}

/// Whether a declared type says the shape of a function anywhere inside it.
#[must_use]
pub fn declared_holds_fn_signature(declared: &DataType) -> bool {
    match declared {
        DataType::FnValue(fn_type) => fn_type.sig.is_some(),
        DataType::Array(Some(element)) => declared_holds_fn_signature(element),
        DataType::Map(entry) => entry.1.as_ref().is_some_and(declared_holds_fn_signature),
        _ => false,
    }
}

/// Decides, for every call in `body` that saves registers, which registers it
/// saves: the ones the body may still read after the call returns.
///
/// A register counts as read after the call when any instruction past the call
/// reads it, with two exceptions that hold on every path out of the call. The
/// register the call returns into is written by the return itself, so the value
/// saved there would never be read. And in the straight run of instructions
/// that follows the call, up to the first branch or the first place a branch
/// lands, a register written before anything reads it holds a fresh value from
/// then on. Neither exception applies in a body that catches errors: a throw
/// out of the call resumes at the catch without running the return or the run
/// after it, so everything the body reads later is saved.
///
/// Each call is paired with its own `SaveFrame` through the frame's return
/// offset, so a call made while working out another call's arguments gets its
/// own list. The calls are visited last to first, which lets a call see the
/// list of a later call it runs into, since that later call's frame reads
/// those registers when it saves them.
fn set_saved_registers(
    body: &[Instr],
    all_written_regs: &[u16],
    params: &[u16],
    state: &mut State<'_>,
) {
    let catches = body
        .iter()
        .any(|instr| matches!(instr, Instr::StartErrorCatch(_, _)));
    let mut lands = vec![false; body.len() + 1];
    for (pos, instr) in body.iter().enumerate() {
        if let Some(target) = branch_target(pos, *instr)
            && let Some(slot) = lands.get_mut(target)
        {
            *slot = true;
        }
    }
    // (call position, return register, call site) for every saving call.
    let mut calls: Vec<(usize, u16, u16)> = body
        .iter()
        .enumerate()
        .filter_map(|(pos, instr)| match instr {
            Instr::SaveFrame(offset, return_reg, callsite) => {
                Some((pos + *offset as usize, *return_reg, *callsite))
            }
            _ => None,
        })
        .collect();
    calls.sort_unstable_by_key(|&(call_pos, _, _)| std::cmp::Reverse(call_pos));

    let mut reads: Vec<u16> = Vec::new();
    for &(call_pos, return_reg, callsite) in &calls {
        let mut live: Vec<u16> = Vec::new();
        let mut written: Vec<u16> = Vec::new();
        let mut straight = !catches;
        if straight && state.value_callsites.contains(&callsite) {
            written.push(return_reg);
        }
        for (pos, instr) in body.iter().enumerate().skip(call_pos + 1) {
            if lands[pos] {
                straight = false;
            }
            reads.clear();
            match instr {
                // Saving reads the registers the later call keeps.
                Instr::SaveFrame(_, _, later) => {
                    reads.extend_from_slice(&state.callsite_registers[*later as usize]);
                }
                // A recursive call reads its parameters where they already
                // are: an argument that is the parameter itself is not moved
                // there first, so no instruction names the read.
                Instr::CallFuncRecursive(target, _) => {
                    reads.extend_from_slice(params);
                    for fn_impl in state.fns.iter().flat_map(|f| f.impls.iter()) {
                        if fn_impl.loc == *target {
                            reads.extend_from_slice(&fn_impl.args_loc);
                        }
                    }
                }
                _ => instr.for_each_read_reg(|reg| reads.push(reg)),
            }
            for &reg in &reads {
                if !written.contains(&reg) && all_written_regs.binary_search(&reg).is_ok() {
                    live.push(reg);
                }
            }
            if !straight {
                continue;
            }
            let write = match instr {
                // A call writes its return register when it comes back, and
                // only if it returns a value.
                Instr::CallFuncRecursive(_, _) | Instr::CallIndirect(_) => calls
                    .iter()
                    .find(|(pos_of_call, _, site)| {
                        *pos_of_call == pos && state.value_callsites.contains(site)
                    })
                    .map(|(_, reg, _)| *reg),
                // These name a register they write only later, or maybe not
                // at all.
                Instr::SaveFrame(_, _, _)
                | Instr::CallFunc(_, _)
                | Instr::StartErrorCatch(_, _) => None,
                _ => instr.get_tgt_id(),
            };
            if let Some(reg) = write
                && !live.contains(&reg)
                && !written.contains(&reg)
            {
                written.push(reg);
            }
            if ends_straight_run(*instr) {
                straight = false;
            }
        }
        live.sort_unstable();
        live.dedup();
        state.callsite_registers[callsite as usize] = live;
    }
}

/// The most instructions a body may have and still be copied into its call
/// sites. Past a handful, a copy grows the program by more than the call it
/// saves costs.
const INLINE_LIMIT: usize = 16;

/// The body of a function a release build can copy into its call sites in
/// place of calling it, or `None` when it cannot be.
///
/// The body has to be short, call no candela function (a call out of it would
/// need a frame, and a body that calls itself would never stop being copied),
/// and leave in exactly one way: by falling off its end, or by one `return`
/// that is its last instruction. A copy then runs exactly as the call would
/// have: it reads the arguments the call would have passed, works on
/// registers nothing outside it reads, and leaves its value in the register
/// the call would have returned it into. A host or library call inside it is
/// an ordinary instruction and does not stop it being copied.
fn inlinable_body(body: &[Instr]) -> Option<Box<[Instr]>> {
    if body.len() > INLINE_LIMIT {
        return None;
    }
    let (last, rest) = body
        .split_last()
        .map_or((None, body), |(l, r)| (Some(*l), r));
    let ends_in_return = matches!(last, Some(Instr::Return(_)));
    let inside = if ends_in_return { rest } else { body };
    let simple = inside.iter().all(|instr| {
        !matches!(
            instr,
            Instr::CallFunc(_, _)
                | Instr::CallFuncRecursive(_, _)
                | Instr::CallIndirect(_)
                | Instr::SaveFrame(_, _, _)
                | Instr::Return(_)
                | Instr::RecursiveReturn(_)
                | Instr::VoidReturn
                | Instr::StartErrorCatch(_, _)
                | Instr::StopErrorCatch
                | Instr::Halt(_)
        )
    });
    // Every jump has to land inside the body or on its end, which is where the
    // call would have returned to.
    let jumps_stay = body
        .iter()
        .enumerate()
        .all(|(pos, instr)| branch_target(pos, *instr).is_none_or(|target| target <= body.len()));
    (simple && jumps_stay).then(|| Box::from(body))
}

/// Gives `new`, an instruction rewritten from `old`, the span `old` has. A
/// runtime error names its place by the instruction that raised it, so a
/// rewritten one has to be found where the original was.
fn carry_span(old: Instr, new: Instr, state: &mut State<'_>) {
    if let Some(src) = state.instr_src.iter().find(|src| src.instr == old) {
        let (span, file_id) = (src.span, src.file_id);
        state.instr_src.push(InstrSrc {
            instr: new,
            span,
            file_id,
        });
    }
}

/// Gives a body about to be copied into a call site the registers it runs on
/// there. Each parameter becomes the register its argument was worked out
/// into, or, for a parameter the body writes, a fresh register the argument is
/// moved into first. Everything else the body writes gets a fresh register.
///
/// A copy that shared the callee's registers with the body left where it is,
/// and with every other copy, would tie them together: whatever one of them
/// does with a register, a pass looking at the program has to assume of all.
/// Nothing outside the body reads what it writes, so fresh registers change
/// nothing the program can see.
fn bind_body_registers(
    body: &[Instr],
    bindings: &[(u16, u16)],
    output: &mut Vec<Instr>,
    state: &mut State<'_>,
) -> Vec<Instr> {
    let writes = |reg: u16| body.iter().any(|instr| instr.get_tgt_id() == Some(reg));
    let mut renamed: Vec<(u16, u16)> = Vec::new();
    for &(param, arg) in bindings {
        if writes(param) {
            state.registers.push(NULL);
            let fresh = (state.registers.len() - 1) as u16;
            output.push(Instr::Mov(arg, fresh));
            renamed.push((param, fresh));
        } else {
            renamed.push((param, arg));
        }
    }
    for instr in body {
        if let Some(reg) = instr.get_tgt_id()
            && !renamed.iter().any(|&(from, _)| from == reg)
        {
            state.registers.push(NULL);
            renamed.push((reg, (state.registers.len() - 1) as u16));
        }
    }
    let rename = |reg: &mut u16| {
        if let Some(&(_, to)) = renamed.iter().find(|&&(from, _)| from == *reg) {
            *reg = to;
        }
    };
    body.iter()
        .map(|&old| {
            let mut new = old;
            new.read_regs_mut(rename);
            if let Some(reg) = new.tgt_id_mut() {
                rename(reg);
            }
            if new != old {
                carry_span(old, new, state);
            }
            new
        })
        .collect()
}

/// Copies an inlinable `body` in place of a call, reading each parameter from
/// the register `bindings` names for it (see [`bind_body_registers`]).
///
/// The `return` at the end of the body, if it has one, becomes a move of the
/// returned value into the register the call would have written. When what
/// the body returns is built by its last instructions alone (one instruction,
/// or a struct construction and its field writes) and nothing else reads it,
/// those instructions build it in the call's register directly instead.
fn inline_call(
    body: &[Instr],
    bindings: &[(u16, u16)],
    return_register: u16,
    returns_null: bool,
    output: &mut Vec<Instr>,
    state: &mut State<'_>,
) {
    let body = &bind_body_registers(body, bindings, output, state)[..];
    let start = output.len();
    output.extend_from_slice(body);
    let Some(&Instr::Return(returned)) = body.last() else {
        return;
    };
    output.pop();
    if returns_null || returned == return_register {
        return;
    }
    let before = body.len() - 1;
    // The instructions that build the returned value: the last one, or a
    // construction followed by its field writes.
    let mut first = before;
    while first > 0
        && matches!(body[first - 1], Instr::SetFieldStruct(r, value, _)
            if r == returned && value != returned && value != return_register)
    {
        first -= 1;
    }
    let builder = first.checked_sub(1);
    let builds = builder.is_some_and(|at| match body[at] {
        Instr::CloneStruct(_, r) => r == returned,
        other => first == before && other.get_tgt_id() == Some(returned),
    });
    let written_once = body
        .iter()
        .filter(|instr| instr.get_tgt_id() == Some(returned))
        .count()
        == 1;
    let mut reads = 0;
    for instr in &body[..before] {
        instr.for_each_read_reg(|reg| {
            if reg == returned {
                reads += 1;
            }
        });
    }
    let field_writes = before - first;
    let landed_on = body.iter().enumerate().any(|(pos, instr)| {
        branch_target(pos, *instr).is_some_and(|target| target >= first && target <= before)
    });
    if builds && written_once && reads == field_writes && !landed_on {
        let at = start + builder.unwrap_or_default();
        let old = output[at];
        if move_to_id(&mut output[start..start + before], return_register) {
            carry_span(old, output[at], state);
            return;
        }
    }
    output.push(Instr::Mov(returned, return_register));
}

/// Whether working out `expr` may call a candela function, directly or through
/// an operator a type defines. A literal, a variable, a field or an element
/// read, and an operator on primitive values call nothing; anything else is
/// taken to call.
fn expr_may_call(expr: &Expr, v: &mut Vec<Variable>, ctx: Ctx, state: &mut State<'_>) -> bool {
    fn primitive(e: &Expr, v: &mut Vec<Variable>, ctx: Ctx, state: &mut State<'_>) -> bool {
        matches!(
            e.infer_type(v, ctx, state),
            DataType::Int | DataType::Float | DataType::Bool | DataType::String | DataType::Null
        )
    }
    match expr {
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Bool(_)
        | Expr::String(_)
        | Expr::Null
        | Expr::Var(_, _) => false,
        Expr::GetStructField(obj, _, _, _) => expr_may_call(obj, v, ctx, state),
        Expr::ArrayGetIndex(base, index, _) => {
            expr_may_call(base, v, ctx, state) || expr_may_call(index, v, ctx, state)
        }
        Expr::Neg(x, _, _) | Expr::BoolNeg(x, _, _) | Expr::BitNot(x, _, _) => {
            expr_may_call(x, v, ctx, state) || !primitive(x, v, ctx, state)
        }
        Expr::Mul(l, r, _, _)
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
            expr_may_call(l, v, ctx, state)
                || expr_may_call(r, v, ctx, state)
                || !primitive(l, v, ctx, state)
                || !primitive(r, v, ctx, state)
        }
        _ => true,
    }
}
