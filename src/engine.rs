//! Embedding / library API for candela.
//!
//! This is the persistent, in-process entry point a Rust host (such as Lumen)
//! uses to embed candela the way it embeds `rhai`/`mlua`: register typed host
//! functions, compile a script to a reusable [`Program`], and invoke
//! script-defined functions by name with marshalled arguments, all while
//! keeping interpreter state (registers + heap pools) alive between calls.
//!
//! ```
//! let mut engine = candela::Engine::new();
//! engine.register_host_fn("app", "rows", |id: &str| id.len() as i64);
//! let mut program = engine.compile("host \"app\" { int rows(string); }\nfn count(id: string) { return app.rows(id); }\nfn main() {}", "main.cdl")?;
//! let rows = program.call("count", &["board".into()])?;
//! assert_eq!(rows, candela::Value::Int(5));
//! # Ok::<(), candela::Diagnostic>(())
//! ```
//!
//! Unlike the one-shot [`crate::candela_run`] C-ABI entry point (which compiles a
//! script, runs `main`, and returns captured stdout), the [`Engine`]/[`Program`]
//! pair keeps the compiler and VM state resident so the host can drive the
//! script incrementally. Errors are returned as structured [`Diagnostic`]
//! values (reusing the `structured-errors` funnel) instead of printing and
//! aborting the process. The value-marshalling types it uses ([`Value`],
//! [`HostType`], ...) live in the VM-only `candela-vm` crate.

use crate::compiler::CompileOutput;
use crate::compiler::FileNamespaces;
use crate::compiler::compiler_data::Dynamiclib;
use crate::compiler::compiler_data::Function;
use crate::compiler::compiler_data::IndirectRegisters;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::expr::Expr;
use crate::compiler::imports::DEFAULT_PACKAGE_ENTRY;
use crate::compiler::imports::ImportResolver;
use crate::compiler::type_system::Generics;
use crate::compiler::type_system::GenericsCheckpoint;
use crate::macros::MacroEnv;
use crate::macros::MacroError;
#[cfg(not(target_arch = "wasm32"))]
use crate::manifest::MANIFEST_NAME;
#[cfg(not(target_arch = "wasm32"))]
use crate::manifest::Manifest;
use crate::trampoline::compile_checked;
use crate::trampoline::compile_trampoline;
use candela_vm::data::Data;
use candela_vm::embed::HostDispatch;
use candela_vm::embed::HostError;
use candela_vm::embed::HostRegistry;
use candela_vm::embed::HostType;
use candela_vm::embed::IntoHostFn;
use candela_vm::embed::Value;
use candela_vm::embed::describe_value;
use candela_vm::embed::holds_enum;
use candela_vm::embed::marshal_value;
use candela_vm::embed::unmarshal_value;
use candela_vm::errors::Diagnostic;
use candela_vm::errors::ErrorCtx;
use candela_vm::errors::collect_diagnostic;
use candela_vm::instr::Instr;
use candela_vm::rt::DataType;
use candela_vm::rt::DynamicLibFn;
use candela_vm::rt::EnumType;
use candela_vm::rt::HostFnSig;
use candela_vm::rt::InstrSrc;
use candela_vm::rt::Pools;
use candela_vm::rt::Source;
use candela_vm::rt::Span;
use candela_vm::rt::Struct;
use candela_vm::vm;
use candela_vm::vm::RegisterFile;
use rustc_hash::FxHashMap;
use smol_strc::SmolStr;
use std::path::Path;
use std::path::PathBuf;

// `Engine::compile` and `Program::call` recover from a compile error by
// catching the unwind the error funnel raises, so this API exists only under an
// unwinding panic strategy. Built with `panic = "abort"` the first error in a
// script would abort the process instead of coming back as a `Diagnostic`,
// which is what the LSP and every embedding host read.
//
// wasm32 aborts on panic whatever the profile says, so the check is for the
// profiles that build the toolchain.
#[cfg(not(target_arch = "wasm32"))]
const _: () = assert!(
    cfg!(panic = "unwind"),
    "the candela embedding API needs an unwinding panic strategy; see [profile.release]"
);

/// The persistent embedding entry point. Holds the table of registered host
/// functions and compiles scripts into reusable [`Program`]s.
///
/// This is the library analogue of the one-shot [`crate::candela_run`]: it does
/// not run a script and hand back stdout, it keeps compiler + VM state resident
/// so the host can call into the script repeatedly.
#[derive(Default)]
pub struct Engine {
    registry: HostRegistry,
    macros: MacroEnv,
    resolver: ImportResolver,
}

impl Engine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Points library imports at `dir` instead of the directory the toolchain
    /// installed.
    ///
    /// `dir` is the one that holds `std/`, so `import "std/string";` reads
    /// `dir/std/string.cdl`. A host that ships the standard library inside its
    /// own application directory names it here, and the scripts it compiles
    /// find it wherever the application was installed.
    ///
    /// ```no_run
    /// let engine = candela::Engine::new().with_lib_dir("/opt/myapp/libs");
    /// ```
    #[must_use]
    pub fn with_lib_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.resolver.set_lib_dir(dir.into());
        self
    }

    /// Makes `dir` the root of the package a script imports as `name`.
    ///
    /// The package is entered through the file its `candela.toml` names as
    /// `entry`, or `src/main.cdl` when the manifest names none or there is no
    /// manifest. `import "shapes";` then reads that file, and
    /// `import "shapes/circle";` reads `circle.cdl` from the entry's
    /// directory. `dir` is searched for native libraries too, so a package
    /// that ships a shared library under its root binds a `dylib` import with
    /// no further setup.
    ///
    /// ```no_run
    /// let engine = candela::Engine::new().with_import_root("shapes", "/cache/shapes/1.2.3");
    /// ```
    #[must_use]
    pub fn with_import_root(mut self, name: &str, dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let entry = package_entry(&dir);
        self.resolver.add_root(name, dir, entry);
        self
    }

    /// Registers a typed host function under `namespace::name`.
    ///
    /// The closure may take any combination of `i64`/`i32`, `f64`, `bool`,
    /// `String` (or a single `&str`) arguments and return one of those or `()`.
    /// The declared types are checked against the script's `host` block when
    /// [`Engine::compile`] runs; a mismatch is a clean [`Diagnostic`], never a
    /// panic.
    ///
    /// A closure that can fail returns `Result<T, HostError>` instead of `T`,
    /// and the error is raised at the call site in the script:
    ///
    /// ```no_run
    /// use candela::HostError;
    ///
    /// let mut engine = candela::Engine::new();
    /// engine.register_host_fn("fs", "read", |path: &str| {
    ///     std::fs::read_to_string(path).map_err(HostError::new)
    /// });
    /// ```
    ///
    /// The type checked against the declaration is the `T` inside the `Result`,
    /// so both spellings bind to the same `host` signature.
    pub fn register_host_fn<Marker, F>(&mut self, namespace: &str, name: &str, f: F)
    where
        F: IntoHostFn<Marker>,
    {
        self.registry.register_host_fn(namespace, name, f);
    }

    /// Registers a host function whose signature is given as data rather than
    /// derived from a Rust closure's types.
    ///
    /// The closure takes the arguments as a `&[Value]` slice, and `arg_types` /
    /// `ret_type` say what the script may pass and expect. The binding is
    /// checked against the `host` block exactly as [`Engine::register_host_fn`]
    /// is, so a declaration that disagrees is a [`Diagnostic`] at
    /// [`Engine::compile`] time, and the declaration must not use `...`.
    ///
    /// ```no_run
    /// use candela::{HostType, Value};
    ///
    /// let mut engine = candela::Engine::new();
    /// engine.register_host_fn_typed(
    ///     "gpio",
    ///     "read",
    ///     vec![HostType::Int],
    ///     HostType::Int,
    ///     |args: &[Value]| Ok(Value::Int(args[0].as_i64().unwrap_or(0))),
    /// );
    /// ```
    ///
    /// Use it when the signature is only known at run time: a plugin table, a
    /// generated binding, anything with no Rust signature to read the types
    /// from.
    pub fn register_host_fn_typed<F>(
        &mut self,
        namespace: &str,
        name: &str,
        arg_types: Vec<HostType>,
        ret_type: HostType,
        f: F,
    ) where
        F: Fn(&[Value]) -> Result<Value, HostError> + 'static,
    {
        self.registry
            .register_host_fn_typed(namespace, name, arg_types, ret_type, f);
    }

    /// Registers a variadic host function under `namespace::name`.
    ///
    /// Unlike [`Engine::register_host_fn`], the closure receives every argument
    /// as a `&[Value]` slice of any length and returns a single [`Value`] (or a
    /// [`HostError`] to raise in the script), so arguments of mixed /
    /// dynamically-typed shape can cross the boundary without a fixed Rust
    /// signature. The `host` block must declare the function with a `...`
    /// argument list:
    ///
    /// ```candela
    /// host "app" {
    ///     log(...);
    /// }
    /// ```
    ///
    /// No arity or per-argument type checking is performed at the call site;
    /// the closure interprets the slice it is handed. A non-variadic
    /// declaration bound to a variadic closure (or vice versa) is a clean
    /// [`Diagnostic`] at [`Engine::compile`] time.
    pub fn register_host_fn_variadic<F>(&mut self, namespace: &str, name: &str, f: F)
    where
        F: Fn(&[Value]) -> Result<Value, HostError> + 'static,
    {
        self.registry.register_host_fn_variadic(namespace, name, f);
    }

    /// Registers the expander for `name!( ... )` invocations in the scripts this
    /// engine compiles.
    ///
    /// The expander receives the raw text between the parentheses, which
    /// candela does not interpret, and returns candela source for one
    /// expression that is parsed in its place. A [`MacroError`] it returns
    /// instead becomes a compile error at the macro, at the byte offset into
    /// the region the error names.
    ///
    /// ```no_run
    /// use candela::macros::MacroError;
    ///
    /// let mut engine = candela::Engine::new();
    /// engine.register_macro("rows", |body: &str| {
    ///     Ok::<String, MacroError>(body.lines().count().to_string())
    /// });
    /// ```
    pub fn register_macro<F>(&mut self, name: &str, expander: F)
    where
        F: Fn(&str) -> Result<String, MacroError> + 'static,
    {
        self.macros.register(name, expander);
    }

    /// Sets what a macro with no registered expander does: fail the compile
    /// naming it (the default), or, when `allow` is true, compile as `null`.
    ///
    /// Tooling that reads scripts written for a host it is not part of turns
    /// this on, so the host's macros do not read as errors.
    pub const fn allow_unknown_macros(&mut self, allow: bool) {
        self.macros.allow_unknown(allow);
    }

    /// Compiles `src` into a reusable [`Program`], binding every `host` function
    /// it declares to the matching registered closure.
    ///
    /// Every function in `src` whose parameters are all annotated is compiled at
    /// those declared types, whether or not anything calls it, so an error in
    /// the body of a function `main` never calls comes back from here rather
    /// than from the first [`Program::call`]. A function with a bare parameter
    /// has no declared type to compile against and stays lazy, and so does a
    /// function an import brought in.
    ///
    /// `main` is executed once here (module instantiation), so any top-level
    /// setup runs before the host makes its first [`Program::call`]. The
    /// entry-point check runs first, so a broken body is reported before any
    /// top-level side effect.
    ///
    /// # Errors
    ///
    /// Returns a [`Diagnostic`] if the script fails to parse/type-check, if the
    /// body of an annotated function does not compile at its declared parameter
    /// types, if the script declares no `main`, if a declared `host` function
    /// has no registered closure, if a registered closure's arity/types
    /// disagree with the `host` block, or if running `main` raises a runtime
    /// error.
    pub fn compile(&self, src: &str, filename: &str) -> Result<Program, Diagnostic> {
        let filename_owned = filename.to_owned();
        let resolver = &self.resolver;
        let out: CompileOutput = self.macros.scope(|| {
            collect_diagnostic(|| {
                // `compile_checked` compiles an entry point for every
                // annotated function, which is what makes this a check step: a
                // body error in a function `main` never calls is reported now,
                // not on the first `Program::call` that reaches it. It also
                // leaves those specialisations warm for that call.
                let (out, _) = compile_checked(src.to_owned(), &filename_owned, resolver);
                // A `Program` runs `main` as soon as it is built, so an
                // embedded program needs one the way a run from the CLI does.
                out.require_main();
                out
            })
        })?;

        // Bind each declared host function to a registered closure, validating
        // arity + types against the closure's derived signature.
        let host_dispatch: Vec<HostDispatch> =
            self.registry.bind(&out.host_fns).map_err(|e| Diagnostic {
                filename: filename.to_owned(),
                span: 0..0,
                message: e.to_string(),
                code: e.code().to_owned(),
            })?;

        let mut program = Program {
            instructions: out.instructions,
            registers: out.registers,
            pools: out.pools,
            instr_src: out.instr_src,
            callsite_registers: out.callsite_registers,
            dyn_lib_fns: out.dyn_lib_fns,
            host_sigs: out.host_fns,
            host_dispatch,
            allocated_arg_count: out.allocated_arg_count,
            allocated_call_depth: out.allocated_call_depth,
            sources: out.sources,
            structs: out.structs,
            enums: out.enums,
            functions: out.functions,
            dyn_libs: out.dyn_libs,
            namespaces: out.namespaces,
            const_registers: out.const_registers,
            free_registers: out.free_registers,
            indirect_registers: out.indirect_registers,
            generics: out.generics,
        };

        // Instantiate: run `main` once so top-level state is established before
        // the first host-driven call.
        program.execute_from(0)?;
        Ok(program)
    }
}

/// A compiled candela program with resident interpreter state.
///
/// Registers and heap pools persist between [`Program::call`] invocations, so
/// state established by one call (including anything a host function mutates on
/// the Rust side) is visible to the next.
///
/// `Program` is single-threaded (`!Send`/`!Sync`): it holds `Rc` dispatchers
/// and reflects candela's single-threaded VM.
pub struct Program {
    // ---- VM state (persists across calls) ----
    instructions: Vec<Instr>,
    registers: Vec<Data>,
    pools: Pools,
    instr_src: Vec<InstrSrc>,
    callsite_registers: Vec<Vec<u16>>,
    dyn_lib_fns: Vec<DynamicLibFn>,
    host_sigs: Vec<HostFnSig>,
    host_dispatch: Vec<HostDispatch>,
    allocated_arg_count: usize,
    allocated_call_depth: usize,
    sources: Vec<Source>,
    structs: Vec<Struct>,
    enums: Vec<EnumType>,
    // ---- compiler state (drives on-demand call trampolines) ----
    functions: Vec<Function>,
    dyn_libs: Vec<Dynamiclib>,
    namespaces: FileNamespaces,
    const_registers: FxHashMap<Data, u16>,
    free_registers: Vec<u16>,
    generics: Generics,
    indirect_registers: IndirectRegisters,
}

/// A checkpoint of the resident tables [`Program::call`] can grow while
/// compiling a trampoline, taken by [`Program::checkpoint`] and undone by
/// [`Program::rollback_to`] if the compile ends in a diagnostic.
struct CompileCheckpoint {
    registers: usize,
    /// `Program::functions` itself grows during a compile, not just once at
    /// [`Engine::compile`]: an anonymous function literal hoists to a fresh
    /// entry the first time it is reached, and instantiating a generic type
    /// lowers every applicable `impl` method the same way. A function the
    /// aborted attempt hoisted is left holding a bytecode address the resident
    /// stream never reached, so it goes back with everything else.
    functions: usize,
    /// Each pre-existing function's specialization-cache length, indexed the
    /// same as the first `functions` entries of `Program::functions`.
    fn_impls: Box<[usize]>,
    /// `Program::callsite_registers` takes one entry per recursive call site
    /// the attempt compiled, and each entry is written once, at the end of the
    /// body that holds the call site. An entry from an earlier compile is
    /// therefore never touched, so the length alone undoes the growth.
    callsite_registers: usize,
    /// Every file's scope length, not only the entry file's: a body compiled
    /// from an imported module declares into that module's scope.
    namespaces: crate::compiler::FileNamespacesCheckpoint,
    /// A generic type is instantiated (and, symmetrically, an enum's variants
    /// added) the first time a call site needs it, the same on-demand way a
    /// function is specialized; `structs`/`enums` cover that growth exactly as
    /// `functions` covers a closure or a lowered `impl` method.
    structs: usize,
    enums: usize,
    /// `add_to_src` grows this in step with the trampoline's local `output`
    /// buffer, keyed by instruction value rather than position, so a stale
    /// entry cannot point past the end of `Program::instructions`; it can only
    /// mislabel a later instruction that happens to be identical to one the
    /// aborted attempt compiled. Rolled back for the same reason as
    /// everything else here: it is not this call's to leave behind.
    instr_src: usize,
    generics: GenericsCheckpoint,
}

impl Program {
    /// Invokes the script-defined function `fn_name` with `args`, returning its
    /// value (or [`Value::Null`] for a void function).
    ///
    /// Each call compiles a small trampoline (which specializes `fn_name` for
    /// the argument types if it hasn't been already) onto the resident
    /// instruction stream and runs it against the persistent register/heap
    /// state, so globals mutated by a previous call remain visible.
    ///
    /// A diagnostic raised while compiling the trampoline (an undeclared name
    /// at the call site, a type mismatch, a nested call that needed its own
    /// specialization first) leaves the resident tables exactly as they stood
    /// before this call began; see [`Program::rollback_to`]. The `Program`
    /// stays callable afterward.
    ///
    /// # Errors
    ///
    /// Returns a [`Diagnostic`] if `fn_name` is unknown, if the arguments don't
    /// type-check against its signature, or if the call raises a runtime error.
    pub fn call(&mut self, fn_name: &str, args: &[Value]) -> Result<Value, Diagnostic> {
        // An enum travels outward only, and no parameter accepts one, not even
        // an `any` one that accepts everything else. Refusing it here, before
        // anything is marshalled, is what keeps the script from reading a null
        // the host never passed.
        if let Some((index, value)) = args.iter().enumerate().find(|(_, v)| holds_enum(v)) {
            return Err(Diagnostic {
                filename: self
                    .sources
                    .first()
                    .map(|source| source.filename.to_string())
                    .unwrap_or_default(),
                span: 0..0,
                message: format!(
                    "argument {} of '{fn_name}' cannot be a {}: an enum value crosses the boundary outward only",
                    index + 1,
                    describe_value(value)
                ),
                code: String::from("argument_type_mismatch"),
            });
        }

        // Every table a trampoline compile can grow is snapshotted up front, so
        // a diagnostic raised while compiling it can be undone rather than
        // leaving a half-compiled specialization for a later call to trip on.
        let checkpoint = self.checkpoint();

        // Scalars compile as literal exprs; arrays/maps can't, so they are
        // allocated into the heap pools now and passed as a pre-seeded variable
        // that holds the handle in a register the trampoline moves into place.
        let dummy = Span { start: 0, end: 0 };
        let mut arg_exprs: Vec<Expr> = Vec::with_capacity(args.len());
        let mut seed_vars: Vec<Variable> = Vec::new();
        for (i, v) in args.iter().enumerate() {
            if let Some(expr) = value_to_expr(v) {
                arg_exprs.push(expr);
            } else {
                let handle = marshal_value(
                    v,
                    &mut self.pools.objs,
                    &mut self.pools.maps,
                    &mut self.pools.strings,
                );
                let register_id = self.registers.len() as u16;
                self.registers.push(handle);
                let name = SmolStr::from(format!("__host_arg{i}"));
                seed_vars.push(Variable {
                    name: name.clone(),
                    register_id,
                    cell: false,
                    var_type: value_datatype(v),
                });
                arg_exprs.push(Expr::Var(name, dummy));
            }
        }

        let arg_spans: Box<[Span]> = args.iter().map(|_| dummy).collect();
        let call_expr = Expr::FunctionCall(
            arg_exprs.into_boxed_slice(),
            Box::from([SmolStr::from(fn_name)]),
            dummy,
            arg_spans,
            Box::from([]),
        );

        // Compile the trampoline (type-checks the call) under a diagnostic sink.
        let offset = self.instructions.len() as u16;
        let (mut output, ret_id) = match collect_diagnostic(|| {
            let mut state = self.compiler_state();
            compile_trampoline(&mut state, offset, &call_expr, seed_vars)
        }) {
            Ok(compiled) => compiled,
            Err(diagnostic) => {
                self.rollback_to(&checkpoint);
                return Err(diagnostic);
            }
        };

        let ret_id = ret_id.unwrap_or(0);
        output.push(Instr::Halt(0));
        let start = self.instructions.len();
        self.instructions.extend(output);

        self.execute_from(start)?;

        Ok(unmarshal_value(
            self.registers[ret_id as usize],
            &self.pools.objs,
            &self.pools.maps,
            &self.pools.strings,
            &self.structs,
            &self.enums,
        ))
    }

    /// Borrows the resident compiler state a trampoline compile writes into.
    fn compiler_state(&mut self) -> State<'_> {
        State {
            registers: &mut self.registers,
            fns: &mut self.functions,
            structs: &mut self.structs,
            enums: &mut self.enums,
            pools: &mut self.pools,
            instr_src: &mut self.instr_src,
            callsite_registers: &mut self.callsite_registers,
            dyn_libs: &mut self.dyn_libs,
            allocated_arg_count: &mut self.allocated_arg_count,
            allocated_call_depth: &mut self.allocated_call_depth,
            const_registers: &mut self.const_registers,
            free_registers: &mut self.free_registers,
            sources: &mut self.sources,
            reserved_registers: rustc_hash::FxHashSet::default(),
            value_callsites: rustc_hash::FxHashSet::default(),
            namespaces: &mut self.namespaces,
            generics: &mut self.generics,
            indirect_registers: &mut self.indirect_registers,
        }
    }

    /// Snapshots the resident tables a trampoline compile writes into as it
    /// goes, so a diagnostic raised partway through can be undone with
    /// [`Program::rollback_to`].
    ///
    /// `self.instructions` needs no entry here: the bytecode a trampoline
    /// compiles lives in a local buffer and only reaches the resident stream
    /// once the whole attempt succeeds (see [`Program::call`]). Everything
    /// captured below, by contrast, is written in place as compilation
    /// proceeds, because a nested call can only reuse a specialization another
    /// call site already produced. Left unrestored on a later error, a
    /// specialization compiled and cached here during an attempt that then
    /// aborts records a bytecode address in a region `self.instructions` never
    /// reached, and the next call that reuses it jumps into whatever unrelated
    /// code, or none, later lands there.
    ///
    /// This is a handful of lengths, one per table plus one per file in the
    /// program, not a copy of the tables themselves, so a call that does not
    /// error pays for little more than reading them.
    fn checkpoint(&self) -> CompileCheckpoint {
        CompileCheckpoint {
            registers: self.registers.len(),
            functions: self.functions.len(),
            fn_impls: self.functions.iter().map(|f| f.impls.len()).collect(),
            callsite_registers: self.callsite_registers.len(),
            namespaces: self.namespaces.checkpoint(),
            structs: self.structs.len(),
            enums: self.enums.len(),
            instr_src: self.instr_src.len(),
            generics: self.generics.checkpoint(),
        }
    }

    /// Undoes everything a failed trampoline compile wrote to the resident
    /// tables, back to `checkpoint`. `Program::call` runs this only when the
    /// compile step itself returned a diagnostic; a successful compile leaves
    /// the tables as they stand and commits the compiled bytecode alongside
    /// them.
    fn rollback_to(&mut self, checkpoint: &CompileCheckpoint) {
        // A `const_registers`/`free_registers` entry naming a register at or
        // past `registers` was necessarily added during the aborted attempt
        // (registers only ever grow, and a constant is registered the moment
        // its register is pushed), so it goes with the registers themselves.
        // An entry `free_registers` lost, because the attempt popped and
        // reused an already-free register it never got to write through
        // committed bytecode, is not restored: that register is simply
        // orphaned rather than double-allocated, which the next compile
        // cannot observe.
        self.const_registers
            .retain(|_, &mut reg| (reg as usize) < checkpoint.registers);
        self.free_registers
            .retain(|&reg| (reg as usize) < checkpoint.registers);
        self.registers.truncate(checkpoint.registers);

        // Every function this attempt added (a hoisted closure, a lowered
        // `impl` method) goes, along with every specialization cached on a
        // function that was already there.
        self.functions.truncate(checkpoint.functions);
        for (func, &len) in self.functions.iter_mut().zip(checkpoint.fn_impls.iter()) {
            func.impls.truncate(len);
        }
        self.callsite_registers
            .truncate(checkpoint.callsite_registers);

        self.namespaces.rollback_to(&checkpoint.namespaces);

        // A generic type instantiated during the attempt is cached in
        // `self.generics` by rendered name, pointing at the struct or enum
        // entry the attempt pushed here; both go together for the same reason
        // `functions` does.
        self.structs.truncate(checkpoint.structs);
        self.enums.truncate(checkpoint.enums);
        self.generics.rollback_to(&checkpoint.generics);

        self.instr_src.truncate(checkpoint.instr_src);
    }

    /// Runs the VM against the resident state starting at instruction `start`,
    /// capturing any error as a [`Diagnostic`].
    fn execute_from(&mut self, start: usize) -> Result<(), Diagnostic> {
        let err_ctx = ErrorCtx {
            instr_src: self.instr_src.clone(),
            sources: self
                .sources
                .iter()
                .map(|s| Source {
                    filename: s.filename.clone(),
                    contents: s.contents.clone(),
                })
                .collect(),
        };

        // Move the register file out so the VM can borrow it mutably, then
        // reclaim it (register state must persist across calls).
        let mut register_file = RegisterFile(std::mem::take(&mut self.registers));

        let instructions = &self.instructions;
        let pools = &mut self.pools;
        let callsite_registers = &self.callsite_registers;
        let dyn_lib_fns = &self.dyn_lib_fns;
        let structs = &self.structs;
        let enums = &self.enums;
        let host_sigs = &self.host_sigs;
        let host_dispatch = &self.host_dispatch;
        let allocated_arg_count = self.allocated_arg_count;
        let allocated_call_depth = self.allocated_call_depth;

        let result = collect_diagnostic(|| {
            vm::execute(
                instructions,
                &mut register_file,
                pools,
                &err_ctx,
                callsite_registers,
                dyn_lib_fns,
                structs,
                enums,
                allocated_arg_count,
                allocated_call_depth,
                host_sigs,
                host_dispatch,
                start,
            );
        });

        self.registers = std::mem::take(&mut register_file.0);
        result
    }
}

/// Synthesizes a literal [`Expr`] carrying a scalar [`Value`] so a host argument
/// can be compiled through the ordinary call path. candela integers are 32-bit, so
/// [`Value::Int`] is narrowed here. Returns `None` for non-scalars (arrays/maps),
/// which cannot be expressed as literal exprs and are instead allocated into the
/// heap pools and passed as a register handle (see [`Program::call`]).
fn value_to_expr(v: &Value) -> Option<Expr> {
    Some(match v {
        Value::Null => Expr::Null,
        Value::Int(i) => Expr::Int(*i),
        Value::Float(f) => Expr::Float(*f),
        Value::Bool(b) => Expr::Bool(*b),
        Value::String(s) => Expr::String(SmolStr::from(s.as_str())),
        Value::Array(_) | Value::Map(_) | Value::Enum { .. } => return None,
    })
}

/// Infers the candela [`DataType`] of a [`Value`] so a host-provided array/map
/// argument can be given a type the call site type-checks against. Homogeneous
/// element/value types are assumed (matching candela's static collection typing);
/// the first element is sampled, empty collections yield an unknown element type.
/// An enum only travels outward, and [`Program::call`] refuses an argument
/// holding one before it gets here, so that arm is what keeps the match
/// exhaustive rather than a type anything is given.
fn value_datatype(v: &Value) -> DataType {
    match v {
        Value::Null | Value::Enum { .. } => DataType::Null,
        Value::Int(_) => DataType::Int,
        Value::Float(_) => DataType::Float,
        Value::Bool(_) => DataType::Bool,
        Value::String(_) => DataType::String,
        Value::Array(items) => DataType::Array(items.first().map(|e| Box::new(value_datatype(e)))),
        Value::Map(entries) => {
            let value = entries.values().next().map(value_datatype);
            DataType::Map(Box::new((Some(DataType::String), value)))
        }
    }
}

/// The entry a package at `dir` is imported through, relative to `dir`: what
/// its manifest names, or the default when there is no manifest to ask or it
/// cannot be read. A manifest the `candela` command would refuse is not this
/// API's to judge; the import simply lands on the default.
#[cfg(not(target_arch = "wasm32"))]
fn package_entry(dir: &Path) -> String {
    Manifest::load(&dir.join(MANIFEST_NAME)).map_or_else(
        |_| String::from(DEFAULT_PACKAGE_ENTRY),
        |m| m.entry().to_owned(),
    )
}

/// The wasm build carries no manifest reader, so every package enters at the
/// default.
#[cfg(target_arch = "wasm32")]
fn package_entry(_dir: &Path) -> String {
    String::from(DEFAULT_PACKAGE_ENTRY)
}
