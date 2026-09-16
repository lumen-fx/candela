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
//! of time, with the arguments still to come, and `candela build` records where
//! each host-callable one starts in the `.cdlb` export table.
//!
//! Compiling an entry point ahead of time is also what type-checks the body of
//! a function nothing in the program calls: its declared parameter types stand
//! in for the call site it never gets.

use crate::compiler::CompileOutput;
use crate::compiler::SymbolKind;
use crate::compiler::compile;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::expr::Expr;
use crate::compiler::imports::ImportResolver;
use crate::instr::Instr;
use candela_vm::artifact::ExportImage;
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
pub fn compile_checked(
    source: String,
    filename: &str,
    resolver: &ImportResolver,
) -> (CompileOutput, Vec<ExportImage>) {
    let mut out = compile(source, filename, false, resolver);
    let exports = compile_entry_points(&mut out);
    (out, exports)
}

/// Compiles the entry point of every fully annotated function in the file being
/// built, and returns the export table describing the host-callable ones.
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
    exports
}

/// The functions an entry point is compiled for, in declaration order, paired
/// with the parameter types it is compiled against.
///
/// A function qualifies when it is defined in the file being built, reachable
/// by its bare name, is not `main`, and annotates every parameter. A bare
/// parameter has no declared type to specialise on, so such a function stays
/// lazy and is first checked by the call that reaches it.
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
        fn_registers: &mut out.fn_registers,
        dyn_libs: &mut out.dyn_libs,
        allocated_arg_count: &mut out.allocated_arg_count,
        allocated_call_depth: &mut out.allocated_call_depth,
        const_registers: &mut out.const_registers,
        free_registers: &mut out.free_registers,
        generics: &mut out.generics,
        sources: &mut out.sources,
        reserved_registers: FxHashSet::default(),
        namespaces: &mut out.namespaces,
    }
}
