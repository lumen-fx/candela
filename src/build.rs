//! The `candela build` path: compile a `.cdl` source to a `.cdlb` bytecode
//! artifact.
//!
//! The artifact FORMAT and its load/run half live in the VM-only `candela-vm`
//! crate ([`candela_vm::artifact`]); this module is the compiler-side half that
//! turns a fresh [`compile`] result into a [`ProgramImage`] and serializes it.
//!
//! It is also where a host's entry points are settled. The VM carries no
//! compiler, so it cannot build a call trampoline when a host asks for a
//! function by name; the trampolines are compiled here instead and recorded in
//! the artifact's export table.

use crate::compiler::CompileOutput;
use crate::compiler::SymbolKind;
use crate::compiler::compile;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::expr::Expr;
use crate::trampoline::compile_trampoline;
use candela_vm::artifact::DynLibFnImage;
use candela_vm::artifact::EnumImage;
use candela_vm::artifact::EnumVariantImage;
use candela_vm::artifact::ExportImage;
use candela_vm::artifact::HostFnImage;
use candela_vm::artifact::InstrSrcImage;
use candela_vm::artifact::ProgramImage;
use candela_vm::artifact::SourceImage;
use candela_vm::artifact::StructImage;
use candela_vm::artifact::serialize_image;
use candela_vm::data::NULL;
use candela_vm::embed::is_host_callable_type;
use candela_vm::errors::collect_diagnostic;
use candela_vm::instr::Instr;
use candela_vm::rt::DataType;
use candela_vm::rt::Span;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;

/// Compiles a `.cdl` source string to a `.cdlb` bytecode artifact.
///
/// The artifact captures the whole program: every imported workspace `.cdl`
/// module is linked into the single serialized image, so the resulting `.cdlb`
/// runs under `candela-vm` with no source tree present. Dynamic-library `import`s
/// and `host` blocks are captured as recipes (logical name + symbol +
/// signature) that the VM re-binds by name at load time, never as embedded
/// binary bytes.
///
/// # Errors
///
/// Returns an error string if serialization fails.
pub fn build_bytecode(source: String, filename: &str) -> Result<Vec<u8>, String> {
    let mut out = compile(source, filename, false);
    let exports = compile_exports(&mut out);
    let image = image_from_output(out, exports);
    serialize_image(&image)
}

/// Compiles a call trampoline for every host-callable function and returns the
/// export table that describes them.
///
/// A function is host-callable when it is defined in the file being built,
/// reachable by its bare name, is not `main`, and annotates every parameter
/// with a type a host value can fill. Anything else has no declared signature
/// to check a host's arguments against, so it gets no entry and the VM reports
/// a call to it as unknown.
fn compile_exports(out: &mut CompileOutput) -> Vec<ExportImage> {
    // Register 0 is candela's void-return / null sink: a call whose result is
    // discarded writes `null` there. A normal program has register 0 occupied by
    // a constant, but an empty `main` can leave it free, which would let a
    // trampoline give it to a parameter and then have a void host call clobber
    // that parameter. Reserve it.
    if out.registers.is_empty() {
        out.registers.push(NULL);
        out.const_registers.entry(NULL).or_insert(0);
    }

    host_callable_functions(out)
        .into_iter()
        .filter_map(|(name, arg_types)| compile_export(out, &name, arg_types))
        .collect()
}

/// The functions that qualify for an export, in declaration order, paired with
/// the parameter types their trampoline is compiled against.
fn host_callable_functions(out: &CompileOutput) -> Vec<(SmolStr, Vec<DataType>)> {
    let mut seen: FxHashSet<SmolStr> = FxHashSet::default();
    let mut callable = Vec::new();
    for (name, kind) in out.namespace.fns() {
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
        let arg_types: Option<Vec<DataType>> = function
            .args
            .iter()
            .map(|(_, ty)| ty.clone().filter(is_host_callable_type))
            .collect();
        if let Some(arg_types) = arg_types {
            callable.push((name.clone(), arg_types));
        }
    }
    callable
}

/// Compiles one function's trampoline onto the end of the instruction stream.
///
/// Returns `None` when the function cannot be specialised for its declared
/// parameter types; that is a function no host could have called anyway, and
/// the rest of the program still builds.
fn compile_export(
    out: &mut CompileOutput,
    name: &str,
    arg_types: Vec<DataType>,
) -> Option<ExportImage> {
    // What a failed specialisation would leave behind. A specialisation records
    // where its body starts before compiling it, so an attempt that unwinds
    // partway leaves an entry pointing at instructions that were never emitted;
    // dropping the entries this attempt added keeps a later export from calling
    // into one.
    let impls_before: Vec<usize> = out.functions.iter().map(|f| f.impls.len()).collect();
    let return_types_before: Vec<usize> = out
        .functions
        .iter()
        .map(|f| f.return_type_cache.len())
        .collect();
    let instr_src_before = out.instr_src.len();
    let symbols_before = out.namespace.symbols.len();

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
    let compiled = collect_diagnostic(|| {
        let mut state = compiler_state(out);
        compile_trampoline(&mut state, entry, &call_expr, seed_vars)
    });

    let Ok((mut trampoline, ret_id)) = compiled else {
        for (function, len) in out.functions.iter_mut().zip(impls_before) {
            function.impls.truncate(len);
        }
        for (function, len) in out.functions.iter_mut().zip(return_types_before) {
            function.return_type_cache.truncate(len);
        }
        out.instr_src.truncate(instr_src_before);
        out.namespace.symbols.truncate(symbols_before);
        return None;
    };

    trampoline.push(Instr::Halt(0));
    out.instructions.extend(trampoline);

    Some(ExportImage {
        name: name.to_owned(),
        entry: u64::from(entry),
        arg_registers,
        arg_types,
        // A function that returns nothing lands on register 0, the null sink.
        ret_register: ret_id.unwrap_or(0),
    })
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
        namespace: &mut out.namespace,
    }
}

fn image_from_output(out: CompileOutput, exports: Vec<ExportImage>) -> ProgramImage {
    // Dynamic-library bindings become referenced-by-name recipes: the logical
    // library name, the symbol, and the marshalling signature, never the
    // shared object's bytes. The VM re-opens the library and rebuilds the libffi
    // CIF from these at load time.
    let dyn_lib_fns = out
        .dyn_lib_fns
        .iter()
        .map(|d| DynLibFnImage {
            library: d.library.to_string(),
            symbol: d.symbol.to_string(),
            types: d.types.to_vec(),
        })
        .collect();
    // `host` functions are captured as name + signature so an embedding runtime
    // can re-bind them; standalone `candela-vm` reports a clear error naming the
    // function it cannot provide.
    let host_fns = out
        .host_fns
        .iter()
        .map(|h| HostFnImage {
            namespace: h.namespace.to_string(),
            name: h.name.to_string(),
            types: h.types.to_vec(),
            variadic: h.variadic,
        })
        .collect();

    ProgramImage {
        instructions: out.instructions,
        registers: out.registers.iter().map(|d| d.0).collect(),
        objs: out
            .pools
            .objs
            .0
            .iter()
            .map(|v| v.iter().map(|d| d.0).collect())
            .collect(),
        maps: out
            .pools
            .maps
            .0
            .iter()
            .map(|m| m.iter().map(|(k, v)| (k.0, v.0)).collect())
            .collect(),
        strings: out.pools.strings.0.clone(),
        instr_src: out
            .instr_src
            .iter()
            .map(|s| InstrSrcImage {
                instr: s.instr,
                span: s.span,
                file_id: s.file_id,
            })
            .collect(),
        fn_registers: out.fn_registers,
        structs: out
            .structs
            .iter()
            .map(|s| StructImage {
                name: s.name.to_string(),
                fields: s
                    .fields
                    .iter()
                    .map(|(n, t, sp)| (n.to_string(), t.clone(), *sp))
                    .collect(),
                id: s.id,
                name_span: s.name_span,
            })
            .collect(),
        enums: out
            .enums
            .iter()
            .map(|e| EnumImage {
                name: e.name.to_string(),
                variants: e
                    .variants
                    .iter()
                    .map(|vt| EnumVariantImage {
                        name: vt.name.to_string(),
                        payload: vt.payload.to_vec(),
                        name_span: vt.name_span,
                    })
                    .collect(),
                id: e.id,
                name_span: e.name_span,
            })
            .collect(),
        sources: out
            .sources
            .iter()
            .map(|s| SourceImage {
                filename: s.filename.to_string(),
                contents: s.contents.clone(),
            })
            .collect(),
        allocated_arg_count: out.allocated_arg_count as u64,
        allocated_call_depth: out.allocated_call_depth as u64,
        dyn_lib_fns,
        host_fns,
        exports,
    }
}
