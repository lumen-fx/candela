//! Call trampolines: the short instruction run that lets a Rust host invoke a
//! script function by name.
//!
//! A trampoline moves the call's arguments into the registers the target
//! specialisation expects, calls it, and leaves the result in one register. It
//! is compiled onto the end of the program's instruction stream, so running it
//! means executing from its entry index against the resident register and heap
//! state.
//!
//! Both halves of the toolchain build them here. `Engine`/`Program` compiles
//! one per call, with the arguments known; the compile-time pass
//! ([`compile_entry_points`]) compiles one per fully annotated function ahead
//! of time, and one per function a host would call that leaves a parameter
//! bare, with the arguments still to come, and `candela build` records where
//! each host-callable one starts in the `.cdlb` export table.
//!
//! Compiling an entry point ahead of time is also what type-checks the body of
//! a function nothing in the program calls: its declared parameter types stand
//! in for the call site it never gets.

use crate::compiler::CompileOutput;
use crate::compiler::SymbolKind;
use crate::compiler::check_only_scope;
use crate::compiler::compile_profile;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::expr::Expr;
use crate::compiler::expr::METHOD_SEP;
use crate::compiler::imports::ImportResolver;
use crate::compiler::type_system::ANON_FN_PREFIX;
use crate::compiler::use_immediates;
use crate::instr::Instr;
use crate::warnings::emit_warning;
use candela_vm::Diagnostic;
use candela_vm::artifact::ExportImage;
use candela_vm::collect_diagnostic;
use candela_vm::data::NULL;
use candela_vm::embed::is_host_callable_type;
use candela_vm::rt::DataType;
use candela_vm::rt::Span;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;

/// Compiles a call trampoline for `call_expr`, specialising the target function
/// for the argument types if that has not happened yet.
///
/// `offset` is the index the returned run will sit at once appended, which the
/// emitted jumps and call targets are absolute against. `seed_vars` are
/// pre-allocated registers standing in for arguments the expression refers to
/// by name. Returns the instruction run and the register holding the result,
/// which is `None` for a function that returns nothing.
pub fn compile_trampoline(
    state: &mut State<'_>,
    offset: u16,
    call_expr: &Expr,
    seed_vars: Vec<Variable>,
) -> (Vec<Instr>, Option<u16>) {
    // A prior trampoline that aborted mid-inference (error unwind) may have
    // left stale entries in the return-type inference thread-local; clear it so
    // this compile starts clean, exactly as `compile()` does.
    crate::compiler::type_system::reset_inference_state();

    let ctx = Ctx {
        block_id: 0,
        is_compiling_recursive: false,
        single_run: false,
        in_function: false,
        file_idx: 0,
        offset,
        return_downcast: None,
    };
    let mut variables = seed_vars;
    let mut output = Vec::new();
    let ret = call_expr.compile(&mut variables, ctx, state, &mut output, None, false, true);
    (output, ret)
}

/// Compiles `source` the way `candela build` does: the whole program, and then
/// an entry point for every fully annotated function in it.
///
/// `candela check` and [`Engine::compile`](crate::Engine::compile) are check
/// steps for that build, so they compile through here too and refuse what the
/// build would refuse. The export table comes back with the compile result for
/// the caller that writes an artifact; a caller that only wanted the check
/// drops both.
#[must_use]
pub fn compile_checked(
    source: String,
    filename: &str,
    resolver: &ImportResolver,
) -> (CompileOutput, Vec<ExportImage>) {
    compile_checked_profile(source, filename, resolver, false)
}

/// [`compile_checked`] as a check only, which opens no library.
///
/// A `dylib` block binds the signatures it declares, so a script whose C
/// library has not been built yet checks the same as one whose library is in
/// place.
///
/// The result type-checks like a full compile but cannot call into a library,
/// so it is for reporting on, not for running or packaging. `candela check`,
/// [`Engine::check`](crate::Engine::check) and the language server compile
/// through here.
#[must_use]
pub fn check_only(source: String, filename: &str, resolver: &ImportResolver) -> CompileOutput {
    check_only_scope(|| compile_checked(source, filename, resolver).0)
}

/// [`compile_checked`] in the profile `optimize` names; see
/// [`crate::compiler::compile_profile`].
pub fn compile_checked_profile(
    source: String,
    filename: &str,
    resolver: &ImportResolver,
    optimize: bool,
) -> (CompileOutput, Vec<ExportImage>) {
    let mut out = compile_profile(source, filename, false, resolver, optimize);
    let exports = compile_entry_points(&mut out);
    (out, exports)
}

/// Compiles the entry point of every fully annotated function in the file being
/// built, and returns the export table describing the host-callable ones.
///
/// A function a host would call that leaves a parameter bare gets a warning per
/// bare parameter and an entry point with those parameters typed `any`. A body
/// that does not compile that way is undone and reported as a warning, not an
/// error, so a program that built before still builds. A file with no `main`
/// is a library its importers call, so it gets the entry points and none of
/// the warnings.
///
/// A function's declared parameter types are everything the compiler needs to
/// check its body, so this pass is where a body that only a host would have
/// reached is type-checked. A diagnostic raised here travels by the error
/// funnel and ends the compile.
///
/// Being checked and being exported are separate questions. The export table
/// only lists the functions a host `Value` can be marshalled into, so a
/// function with a struct- or enum-typed parameter is compiled here and gets no
/// entry.
pub fn compile_entry_points(out: &mut CompileOutput) -> Vec<ExportImage> {
    // Register 0 is candela's void-return / null sink: a call whose result is
    // discarded writes `null` there. A normal program has register 0 occupied by
    // a constant, but an empty `main` can leave it free, which would let a
    // trampoline give it to a parameter and then have a void host call clobber
    // that parameter. Reserve it.
    if out.registers.is_empty() {
        out.registers.push(NULL);
        out.const_registers.entry(NULL).or_insert(0);
    }

    let mut exports = Vec::new();
    for (name, arg_types) in annotated_functions(out) {
        let host_callable = arg_types.iter().all(is_host_callable_type);
        let export = compile_entry_point(out, &name, arg_types);
        if host_callable {
            exports.push(export);
        }
    }

    // A file with no `main` is a library: its functions are called by the
    // files that import it, and those calls specialise the bare parameters,
    // so it is not told what a host would pass. The `any` entry point is still
    // tried for a host that loads it anyway. A file the one being built imports
    // is never looked at here, since only the built file's functions are.
    let warn = declares_main(out);

    // What nothing compiled so far calls is chosen once, before any attempt
    // below compiles a body that calls another candidate, so the choice is the
    // same in both profiles and does not depend on declaration order.
    for (name, fn_id) in host_called_bare_functions(out) {
        let function = &out.functions[fn_id];
        let name_span = function.name_span;
        let filename = out.sources[function.src_file as usize].filename.to_string();
        let arg_types: Vec<DataType> = function
            .args
            .iter()
            .map(|(_, ty)| ty.clone().unwrap_or(DataType::Unknown))
            .collect();
        let bare: Vec<SmolStr> = function
            .args
            .iter()
            .filter(|(_, ty)| warn && ty.is_none())
            .map(|(param, _)| param.clone())
            .collect();
        for param in bare {
            emit_warning(
                &out.sources,
                "Unannotated parameter",
                Diagnostic {
                    filename: filename.clone(),
                    span: (name_span.start as usize)..(name_span.end as usize),
                    message: format!(
                        "Parameter {param} of {name} has no type, so a host calling {name} passes it as any. Annotate it with the type the host passes"
                    ),
                    code: String::from("unannotated_host_parameter"),
                },
            );
        }
        // Trying the body at `any` and carrying on when it does not compile
        // means catching the error it raises, which needs an unwinding panic.
        if !cfg!(panic = "unwind") {
            continue;
        }
        let host_callable = arg_types.iter().all(is_host_callable_type);
        let checkpoint = compiler_state(out).checkpoint();
        match collect_diagnostic(|| compile_entry_point(out, &name, arg_types)) {
            Ok(export) => {
                if host_callable {
                    exports.push(export);
                }
            }
            Err(error) => {
                compiler_state(out).rollback_to(&checkpoint);
                if !warn {
                    continue;
                }
                emit_warning(
                    &out.sources,
                    "No host entry point",
                    Diagnostic {
                        filename: error.filename,
                        span: error.span,
                        message: format!(
                            "{name} gets no entry point a host can call: with its unannotated parameters as any, its body does not compile. {}",
                            error.message
                        ),
                        code: String::from("no_host_entry_point"),
                    },
                );
            }
        }
    }
    exports
}

/// Whether the file being built declares `main`, which is what makes it a
/// program a host loads rather than a library other files import.
fn declares_main(out: &CompileOutput) -> bool {
    out.namespaces.root().fns().any(|(name, kind)| {
        name.as_str() == "main"
            && matches!(kind, SymbolKind::Fn(fn_id) if out.functions[*fn_id as usize].src_file == 0)
    })
}

/// The functions a host would call by name that have a bare parameter, in
/// declaration order, by name and index.
///
/// A function qualifies when it is defined in the file being built, reachable
/// by its bare name (so not a method or a closure), is not `main`, declares no
/// type parameters, leaves at
/// least one parameter unannotated, and nothing compiled so far calls it. A
/// function the program calls is compiled by that call, at the types it
/// passes; one nothing calls is left for a host, which has no compiler to do
/// that once the program is packaged.
fn host_called_bare_functions(out: &CompileOutput) -> Vec<(SmolStr, usize)> {
    let mut seen: FxHashSet<SmolStr> = FxHashSet::default();
    let mut bare = Vec::new();
    for (name, kind) in out.namespaces.root().fns() {
        let SymbolKind::Fn(fn_id) = kind else {
            continue;
        };
        // A method is reached through a value of its type and a closure
        // through the value it was stored in, never by a bare name.
        if name.as_str() == "main"
            || name.contains(METHOD_SEP)
            || name.starts_with(ANON_FN_PREFIX)
            || !seen.insert(name.clone())
        {
            continue;
        }
        let function = &out.functions[*fn_id as usize];
        if function.src_file != 0
            || function.generics.is_some()
            || !function.impls.is_empty()
            || function.args.iter().all(|(_, ty)| ty.is_some())
        {
            continue;
        }
        bare.push((name.clone(), *fn_id as usize));
    }
    bare
}

/// The functions an entry point is compiled for, in declaration order, paired
/// with the parameter types it is compiled against.
///
/// A function qualifies when it is defined in the file being built, reachable
/// by its bare name, is not `main`, and annotates every parameter. A bare
/// parameter has no declared type to specialise on; such a function is first
/// checked by the call that reaches it, or, when nothing in the program calls
/// it, by the `any` entry point [`compile_entry_points`] tries for a host.
fn annotated_functions(out: &CompileOutput) -> Vec<(SmolStr, Vec<DataType>)> {
    let mut seen: FxHashSet<SmolStr> = FxHashSet::default();
    let mut annotated = Vec::new();
    for (name, kind) in out.namespaces.root().fns() {
        let SymbolKind::Fn(fn_id) = kind else {
            continue;
        };
        // A name resolves to the first symbol carrying it, so a later shadowed
        // definition is not the one a call would reach.
        if name.as_str() == "main" || !seen.insert(name.clone()) {
            continue;
        }
        let function = &out.functions[*fn_id as usize];
        if function.src_file != 0 {
            continue;
        }
        let arg_types: Option<Vec<DataType>> =
            function.args.iter().map(|(_, ty)| ty.clone()).collect();
        if let Some(arg_types) = arg_types {
            annotated.push((name.clone(), arg_types));
        }
    }
    annotated
}

/// Compiles one function's entry point onto the end of the instruction stream.
///
/// The run is appended whether or not the function ends up in the export table:
/// a specialisation compiled during the attempt caches the bytecode address of
/// its body, so dropping the run would leave that cache pointing at
/// instructions that were never emitted.
fn compile_entry_point(
    out: &mut CompileOutput,
    name: &str,
    arg_types: Vec<DataType>,
) -> ExportImage {
    // Each parameter gets a register of its own. The host writes its marshalled
    // argument there before running the trampoline, which is why the register
    // ids travel in the export table.
    let dummy = Span { start: 0, end: 0 };
    let mut arg_registers: Vec<u16> = Vec::with_capacity(arg_types.len());
    let mut seed_vars: Vec<Variable> = Vec::with_capacity(arg_types.len());
    let mut arg_exprs: Vec<Expr> = Vec::with_capacity(arg_types.len());
    for (idx, var_type) in arg_types.iter().enumerate() {
        let register_id = out.registers.len() as u16;
        out.registers.push(NULL);
        arg_registers.push(register_id);
        let var_name = SmolStr::from(format!("__export_arg{idx}"));
        seed_vars.push(Variable {
            name: var_name.clone(),
            register_id,
            cell: false,
            var_type: var_type.clone(),
        });
        arg_exprs.push(Expr::Var(var_name, dummy));
    }

    let call_expr = Expr::FunctionCall(
        arg_exprs.into_boxed_slice(),
        Box::from([SmolStr::from(name)]),
        dummy,
        arg_types.iter().map(|_| dummy).collect(),
        Box::from([]),
    );

    let entry = out.instructions.len() as u16;
    let (mut trampoline, ret_id) = {
        let mut state = compiler_state(out);
        compile_trampoline(&mut state, entry, &call_expr, seed_vars)
    };

    trampoline.push(Instr::Halt(0));
    use_immediates(
        &mut trampoline,
        &out.instructions,
        &out.registers,
        &out.const_registers,
    );
    out.instructions.extend(trampoline);

    ExportImage {
        name: name.to_owned(),
        entry: u64::from(entry),
        arg_registers,
        arg_types,
        // A function that returns nothing lands on register 0, the null sink.
        ret_register: ret_id.unwrap_or(0),
    }
}

/// Borrows the compile result as the mutable state a trampoline compile writes
/// into.
fn compiler_state(out: &mut CompileOutput) -> State<'_> {
    State {
        registers: &mut out.registers,
        fns: &mut out.functions,
        structs: &mut out.structs,
        enums: &mut out.enums,
        pools: &mut out.pools,
        instr_src: &mut out.instr_src,
        callsite_registers: &mut out.callsite_registers,
        dyn_libs: &mut out.dyn_libs,
        allocated_arg_count: &mut out.allocated_arg_count,
        allocated_call_depth: &mut out.allocated_call_depth,
        const_registers: &mut out.const_registers,
        free_registers: &mut out.free_registers,
        generics: &mut out.generics,
        indirect_registers: &mut out.indirect_registers,
        sources: &mut out.sources,
        reserved_registers: FxHashSet::default(),
        value_callsites: FxHashSet::default(),
        optimize: out.optimize,
        namespaces: &mut out.namespaces,
    }
}
