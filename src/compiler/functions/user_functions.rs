use super::super::expr::Expr;
use super::super::expr::Span;
use super::super::registers::get_tgt_ids;
use super::super::registers::move_to_id;
use super::super::type_system::DataType;
use super::super::type_system::arg_types_specialize_equal;
use super::super::type_system::can_reach;
use super::super::type_system::check_if_returns_void;
use super::super::type_system::fn_bindings;
use super::super::type_system::param_type_matches;
use super::super::type_system::specialization_key;
use super::super::type_system::track_returns;
use crate::compiler::SymbolKind;
use crate::compiler::UnwrapId;
use crate::compiler::compile_expr;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::FunctionImpl;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::check_args_user_fn;
use crate::compiler::compiler_errors::error_function_arg_invalid_type;
use crate::compiler::compiler_errors::error_invalid_type;
use crate::compiler::functions::compile_call_args;
use crate::compiler::functions::store_call_args;
use crate::data::NULL;
use crate::instr::Instr;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use std::rc::Rc;

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
) -> Option<u16> {
    // Lazily resolve mutual recursion the first time this function is compiled
    let is_recursive = if let Some(is_recursive) = state.fns[fn_id].is_recursive {
        is_recursive
    } else {
        let name = state.fns[fn_id].name.clone();
        let mut visited = FxHashSet::default();
        visited.insert(name.clone());
        let is_recursive = can_reach(&name, &name, state.fns, &mut visited);
        state.fns[fn_id].is_recursive = Some(is_recursive);
        is_recursive
    };

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
    let infered_arg_types = args
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
    for (i, declared) in declared_arg_types.iter().enumerate() {
        if let Some(declared) = declared
            && !param_type_matches(declared, &infered_arg_types[i])
        {
            error_function_arg_invalid_type(
                &infered_arg_types[i],
                declared,
                args_indexes[i],
                fn_name,
                Some((state.fns[fn_id].name_span, state.fns[fn_id].src_file)),
                ctx.file_idx,
                state.sources,
            );
        }
    }

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
            fn_id as u16,
            is_recursive,
            state.fns[fn_id].src_file,
        );
    }
    // Re-derive index after possible mutation
    let fn_impl_idx = fn_impl_idx.unwrap_or_else(|| state.fns[fn_id].impls.len() - 1);
    let loc = state.fns[fn_id].impls[fn_impl_idx].loc;
    let args_loc_len = state.fns[fn_id].impls[fn_impl_idx].args_loc.len();

    let saveframe_loc = output.len();
    let callsite_id = if is_recursive {
        let id = state.fn_registers.len() as u16;
        state.fn_registers.push(Vec::new());
        output.push(Instr::SaveFrame(0, 0, 0));
        *state.allocated_call_depth += 2;
        Some(id)
    } else {
        None
    };
    // Move evaluated call args into the expected arg slots
    #[allow(clippy::needless_range_loop)]
    for i in 0..args_loc_len {
        let tgt_id = state.fns[fn_id].impls[fn_impl_idx].args_loc[i];

        if matches!(infered_arg_types[i], DataType::Fn(_)) {
            continue;
        }

        let start_len = output.len();
        let arg_id = args[i]
            .compile(v, ctx, state, output, Some(tgt_id), false, true)
            .unwrap_id();
        if output.len() == start_len || !move_to_id(output, tgt_id) {
            output.push(Instr::Mov(arg_id, tgt_id));
        }
    }
    if !is_recursive {
        state
            .fn_registers
            .get_mut(fn_id)
            .unwrap()
            .extend(get_tgt_ids(&output[saveframe_loc..]));
    }

    let return_register_id = if fn_returns_null {
        0
    } else {
        state.alloc_reg_tgt(tgt_id)
    };
    if is_recursive {
        output.push(Instr::CallFuncRecursive(loc, return_register_id));
    } else {
        output.push(Instr::CallFunc(loc, return_register_id));
        *state.allocated_call_depth += 2;
    }

    if is_recursive {
        output[saveframe_loc] = Instr::SaveFrame(
            (output.len() - 1 - saveframe_loc) as u16,
            return_register_id,
            callsite_id.unwrap(),
        );
    }

    if fn_returns_null {
        None
    } else {
        Some(return_register_id)
    }
}

/// The parameter types this call specialises on.
///
/// Without type arguments these are the function's own declared types, where an
/// annotation naming a type parameter was left un-pinned. A call that names its
/// type arguments resolves the annotations again with them bound, which is what
/// makes `first<int>(xs)` reject a `float[]`.
fn specialized_arg_types(
    fn_id: usize,
    type_args: &[DataType],
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<Option<DataType>> {
    let declared = state.fns[fn_id]
        .args
        .iter()
        .map(|(_, t)| t.clone())
        .collect::<Vec<Option<DataType>>>();
    if type_args.is_empty() {
        return declared;
    }
    let Some(generics) = state.fns[fn_id].generics.as_ref() else {
        return declared;
    };
    let arg_types = generics.arg_types.clone();
    let file_idx = generics.file_idx;
    let frame = fn_bindings(fn_id, type_args, state);
    let namespace = state.generics.file_namespace(file_idx);
    let mut base = state.type_ctx(ctx.file_idx);
    let mut type_ctx = base.reborrow(file_idx, &namespace);
    type_ctx.generics.push_bindings(frame);
    let resolved = arg_types
        .iter()
        .map(|t| t.as_ref().map(|t| t.to_datatype(&mut type_ctx)))
        .collect();
    type_ctx.generics.pop_bindings();
    resolved
}

/// The `-> Type` annotation as it reads for the specialisation being compiled,
/// with the type parameters currently bound.
///
/// An annotation naming a parameter the call left unbound stays un-pinned, so
/// what the body returns is inferred rather than checked against a type that
/// has no value yet.
fn specialized_return_type(
    fn_id: usize,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<(DataType, Span)> {
    let Some(generics) = state.fns[fn_id].generics.as_ref() else {
        return state.fns[fn_id].return_type.clone();
    };
    let unbound: Vec<SmolStr> = generics
        .params
        .iter()
        .filter(|param| state.generics.bound(param).is_none())
        .cloned()
        .collect();
    let generics = state.fns[fn_id].generics.as_ref()?;
    let annotation = generics.return_type.as_deref()?;
    if annotation.0.mentions_any(&unbound) {
        return state.fns[fn_id].return_type.clone();
    }
    let (return_type, return_span) = annotation.clone();
    let file_idx = generics.file_idx;
    let namespace = state.generics.file_namespace(file_idx);
    let mut base = state.type_ctx(ctx.file_idx);
    let mut type_ctx = base.reborrow(file_idx, &namespace);
    Some((return_type.to_datatype(&mut type_ctx), return_span))
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
    fn_id: u16,
    is_recursive: bool,
    fn_file_idx: u16,
) {
    // Local vector vars and recorded_types to allow the inner body to type-check correctly
    let mut v_temp: Vec<Variable> = fn_args
        .iter()
        .enumerate()
        .map(|(i, x)| {
            // Allocate a registers slot for each func arg
            state.registers.push(NULL);
            Variable {
                name: x.clone(),
                register_id: (state.registers.len() - 1) as u16,
                var_type: infered_arg_types[i].clone(),
            }
        })
        .collect();

    // Get the arg destination ids
    let args_loc = v_temp.iter().map(|x| x.register_id).collect::<Vec<u16>>();

    // Temporarily jump over function to prevent executing it right now
    // This is a placeholder that's modified later on
    output.push(Instr::Jmp(0));
    let jump_idx = output.len() - 1;

    // Record start location for the compiled func body
    let fn_start = output.len();
    let loc = fn_start as u16 + ctx.offset;

    let v_len_before_args = v.len();
    // let fn_len = state.namespace.symbols.len();
    let mut anon_fns: Vec<usize> = Vec::new();
    infered_arg_types
        .iter()
        .enumerate()
        .for_each(|(i, infered_type)| {
            if let DataType::Fn(fn_id) = infered_type {
                anon_fns.push(state.namespace.symbols.len());
                state
                    .namespace
                    .symbols
                    .push((fn_args[i].clone(), SymbolKind::Fn(*fn_id)));
                v.push(Variable {
                    name: fn_args[i].clone(),
                    register_id: 0,
                    var_type: DataType::Fn(*fn_id),
                });
            } else {
                // 0 => placeholder id, it's never used
                v.push(Variable {
                    name: fn_args[i].clone(),
                    register_id: 0,
                    var_type: infered_type.clone(),
                });
            }
        });
    state
        .generics
        .push_bindings(fn_bindings(function_id, type_args, state));
    let fn_type = track_returns(fn_code, v, ctx, state, fn_name);
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
        && !param_type_matches(&declared, &return_type)
    {
        error_invalid_type(
            &declared,
            &return_type,
            declared_span,
            None,
            Some(format_args!(
                "Function {fn_name} is declared to return {declared}"
            )),
            fn_file_idx,
            state.sources,
        );
    }

    v.truncate(v_len_before_args);

    // Add this func specialization to the func's metadata
    let func = state.fns.get_mut(function_id).unwrap();
    func.impls.push(FunctionImpl {
        loc,
        args_loc: Box::from(args_loc.as_slice()),
        arg_types: Box::from(infered_arg_types),
        type_args: Box::from(type_args),
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
        state.namespace.symbols.remove(i);
    }

    let mut reserved_registers = get_tgt_ids(&parsed);
    reserved_registers.extend(args_loc);
    for instr in &parsed {
        match instr {
            Instr::CloneArray(template_reg, _, _)
            | Instr::CloneStruct(template_reg, _)
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

    if is_recursive {
        let all_written_regs: Vec<u16> = get_tgt_ids(&parsed);

        // For each recursive call, only save registers that are read between that call's return and the end of the function
        for (pos, instr) in parsed.iter().enumerate() {
            if matches!(instr, Instr::CallFuncRecursive(_, _)) {
                // Walk backwards to find this call's SaveFrame and its callsite_id
                let callsite_id = parsed[..pos]
                    .iter()
                    .rev()
                    .find_map(|i| match i {
                        Instr::SaveFrame(_, _, cid) => Some(*cid),
                        _ => None,
                    })
                    .unwrap_id();

                let mut live_regs: Vec<u16> = Vec::new();
                for after_instr in &parsed[pos + 1..] {
                    after_instr.for_each_read_reg(|reg| {
                        if all_written_regs.binary_search(&reg).is_ok() {
                            live_regs.push(reg);
                        }
                    });
                }
                live_regs.sort_unstable();
                live_regs.dedup();
                unsafe {
                    *state.fn_registers.get_unchecked_mut(callsite_id as usize) = live_regs;
                }
            }
        }
    } else {
        state
            .fn_registers
            .get_mut(fn_id as usize)
            .unwrap()
            .extend(get_tgt_ids(&parsed));
    }

    output.extend(parsed);

    output.push(Instr::VoidReturn);

    // Fix the placeholder Jmp(0) to skip over the function body
    *output.get_mut(jump_idx).unwrap() = Instr::Jmp((output.len() - fn_start + 1) as u16);
}
