//! Embedding / library API for keel.
//!
//! This is the persistent, in-process entry point a Rust host (such as Lumen)
//! uses to embed keel the way it embeds `rhai`/`mlua`: register typed host
//! functions, compile a script to a reusable [`Program`], and invoke
//! script-defined functions by name with marshalled arguments — all while
//! keeping interpreter state (registers + heap pools) alive between calls.
//!
//! ```no_run
//! let mut engine = keel::Engine::new();
//! engine.register_host_fn("app", "rows", |id: &str| id.len() as i64);
//! let mut program = engine.compile("host \"app\" { int rows(string); }\nfn count(id) { return app.rows(id); }\nfn main() {}", "main.kl")?;
//! let rows = program.call("count", &["board".into()])?;
//! assert_eq!(rows, keel::Value::Int(5));
//! # Ok::<(), keel::Diagnostic>(())
//! ```
//!
//! Unlike the one-shot [`crate::keel_run`] C-ABI entry point (which compiles a
//! script, runs `main`, and returns captured stdout), the [`Engine`]/[`Program`]
//! pair keeps the compiler and VM state resident so the host can drive the
//! script incrementally. Errors are returned as structured [`Diagnostic`]
//! values (reusing the `structured-errors` funnel) instead of printing and
//! aborting the process.

use crate::compiler::CompileOutput;
use crate::compiler::Namespace;
use crate::compiler::compile;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::DynamicLibFn;
use crate::compiler::compiler_data::Dynamiclib;
use crate::compiler::compiler_data::Function;
use crate::compiler::compiler_data::HostFnSig;
use crate::compiler::compiler_data::InstrSrc;
use crate::compiler::compiler_data::Pools;
use crate::compiler::compiler_data::Source;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Struct;
use crate::compiler::expr::Expr;
use crate::compiler::expr::Span;
use crate::compiler::type_system::DataType;
use crate::data::Data;
use crate::data::NULL;
use crate::errors::Diagnostic;
use crate::errors::ErrorCtx;
use crate::errors::collect_diagnostic;
use crate::instr::Instr;
use crate::vm;
use crate::vm::RegisterFile;
use crate::vm::StringPool;
use rustc_hash::FxHashMap;
use smol_strc::SmolStr;
use std::collections::HashMap;
use std::rc::Rc;

/// A dynamically-typed value passed across the host/script boundary.
///
/// keel integers are 32-bit internally (NaN-boxed); [`Value::Int`] widens them
/// to `i64` for host ergonomics and narrows on the way back in.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
}

impl Value {
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        if let Self::Int(i) = self { Some(*i) } else { None }
    }
    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        if let Self::Float(f) = self { Some(*f) } else { None }
    }
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self { Some(*b) } else { None }
    }
    #[must_use]
    pub const fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self { Some(s.as_str()) } else { None }
    }
    #[must_use]
    pub fn into_string(self) -> Option<String> {
        if let Self::String(s) = self { Some(s) } else { None }
    }
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Int(i64::from(v))
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.to_owned())
    }
}
impl From<()> for Value {
    fn from((): ()) -> Self {
        Self::Null
    }
}

/// The primitive type kinds that can cross the host boundary. Used to
/// type-check a registered closure against its `host` block declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostType {
    Int,
    Float,
    Bool,
    String,
    Unit,
}

impl HostType {
    const fn from_datatype(dt: &DataType) -> Option<Self> {
        match dt {
            DataType::Int => Some(Self::Int),
            DataType::Float => Some(Self::Float),
            DataType::Bool => Some(Self::Bool),
            DataType::String => Some(Self::String),
            DataType::Null => Some(Self::Unit),
            _ => None,
        }
    }
    const fn as_str(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::String => "string",
            Self::Unit => "null",
        }
    }
}

/// The type-erased closure the VM dispatches a `host` call to.
pub type HostDispatch = Rc<dyn Fn(&[Value]) -> Value>;

/// A registered host function: its erased dispatcher plus the argument/return
/// type signature derived from the closure, used to validate it against the
/// script's `host` block at compile time.
struct RegisteredFn {
    func: HostDispatch,
    arg_types: Vec<HostType>,
    ret_type: HostType,
}

/// Extracts a Rust argument from a [`Value`] for a registered host closure.
pub trait FromHostValue: Sized {
    fn from_host_value(v: &Value) -> Self;
    fn host_type() -> HostType;
}

impl FromHostValue for i64 {
    fn from_host_value(v: &Value) -> Self {
        v.as_i64().unwrap_or(0)
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl FromHostValue for i32 {
    fn from_host_value(v: &Value) -> Self {
        v.as_i64().unwrap_or(0) as Self
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl FromHostValue for f64 {
    fn from_host_value(v: &Value) -> Self {
        v.as_f64().unwrap_or(0.0)
    }
    fn host_type() -> HostType {
        HostType::Float
    }
}
impl FromHostValue for bool {
    fn from_host_value(v: &Value) -> Self {
        v.as_bool().unwrap_or(false)
    }
    fn host_type() -> HostType {
        HostType::Bool
    }
}
impl FromHostValue for String {
    fn from_host_value(v: &Value) -> Self {
        v.as_str().unwrap_or_default().to_owned()
    }
    fn host_type() -> HostType {
        HostType::String
    }
}

/// Converts a host closure's return value into a [`Value`].
pub trait IntoHostValue {
    fn into_host_value(self) -> Value;
    fn host_type() -> HostType;
}

impl IntoHostValue for i64 {
    fn into_host_value(self) -> Value {
        Value::Int(self)
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl IntoHostValue for i32 {
    fn into_host_value(self) -> Value {
        Value::Int(i64::from(self))
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl IntoHostValue for f64 {
    fn into_host_value(self) -> Value {
        Value::Float(self)
    }
    fn host_type() -> HostType {
        HostType::Float
    }
}
impl IntoHostValue for bool {
    fn into_host_value(self) -> Value {
        Value::Bool(self)
    }
    fn host_type() -> HostType {
        HostType::Bool
    }
}
impl IntoHostValue for String {
    fn into_host_value(self) -> Value {
        Value::String(self)
    }
    fn host_type() -> HostType {
        HostType::String
    }
}
impl IntoHostValue for &str {
    fn into_host_value(self) -> Value {
        Value::String(self.to_owned())
    }
    fn host_type() -> HostType {
        HostType::String
    }
}
impl IntoHostValue for () {
    fn into_host_value(self) -> Value {
        Value::Null
    }
    fn host_type() -> HostType {
        HostType::Unit
    }
}

/// Adapts a Rust closure into a registered host function.
///
/// The `Marker` type parameter disambiguates the blanket impls by arity (and,
/// for `&str`, by borrow) — the same trick `rhai`/`bevy` use to make
/// `register_fn` accept closures of many shapes without annotations.
pub trait IntoHostFn<Marker> {
    /// Internal adapter: yields the erased dispatcher plus the argument and
    /// return type signature derived from the closure. Not meant to be called
    /// directly — [`Engine::register_host_fn`] drives it.
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType);
}

/// Marker for a nullary closure.
pub struct Arity0;
/// Marker for a closure whose sole parameter is a borrowed `&str`.
pub struct ArityStr1;

impl<F, R> IntoHostFn<Arity0> for F
where
    F: Fn() -> R + 'static,
    R: IntoHostValue,
{
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
        (
            Rc::new(move |_args: &[Value]| self().into_host_value()),
            Vec::new(),
            <R as IntoHostValue>::host_type(),
        )
    }
}

impl<F, R> IntoHostFn<ArityStr1> for F
where
    F: Fn(&str) -> R + 'static,
    R: IntoHostValue,
{
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
        (
            Rc::new(move |args: &[Value]| {
                let s = args.first().and_then(Value::as_str).unwrap_or_default();
                self(s).into_host_value()
            }),
            vec![HostType::String],
            <R as IntoHostValue>::host_type(),
        )
    }
}

/// Generates a `IntoHostFn` impl for an owned-argument closure of a given arity.
/// Each argument marker is the tuple of its parameter types, which keeps the
/// impls disjoint from one another and from the `&str`/nullary specializations.
macro_rules! impl_into_host_fn {
    ($($ty:ident $idx:tt),+) => {
        impl<F, R, $($ty,)+> IntoHostFn<($($ty,)+)> for F
        where
            F: Fn($($ty,)+) -> R + 'static,
            R: IntoHostValue,
            $( $ty: FromHostValue + 'static, )+
        {
            fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
                (
                    Rc::new(move |args: &[Value]| {
                        self( $( <$ty as FromHostValue>::from_host_value(&args[$idx]), )+ )
                            .into_host_value()
                    }),
                    vec![ $( <$ty as FromHostValue>::host_type(), )+ ],
                    <R as IntoHostValue>::host_type(),
                )
            }
        }
    };
}

impl_into_host_fn!(A0 0);
impl_into_host_fn!(A0 0, A1 1);
impl_into_host_fn!(A0 0, A1 1, A2 2);
impl_into_host_fn!(A0 0, A1 1, A2 2, A3 3);
impl_into_host_fn!(A0 0, A1 1, A2 2, A3 3, A4 4);

/// The persistent embedding entry point. Holds the table of registered host
/// functions and compiles scripts into reusable [`Program`]s.
///
/// This is the library analogue of the one-shot [`crate::keel_run`]: it does
/// not run a script and hand back stdout, it keeps compiler + VM state resident
/// so the host can call into the script repeatedly.
#[derive(Default)]
pub struct Engine {
    registry: HashMap<(String, String), RegisteredFn>,
}

impl Engine {
    #[must_use]
    pub fn new() -> Self {
        Self { registry: HashMap::new() }
    }

    /// Registers a typed host function under `namespace.name`.
    ///
    /// The closure may take any combination of `i64`/`i32`, `f64`, `bool`,
    /// `String` (or a single `&str`) arguments and return one of those or `()`.
    /// The declared types are checked against the script's `host` block when
    /// [`Engine::compile`] runs; a mismatch is a clean [`Diagnostic`], never a
    /// panic.
    pub fn register_host_fn<Marker, F>(&mut self, namespace: &str, name: &str, f: F)
    where
        F: IntoHostFn<Marker>,
    {
        let (func, arg_types, ret_type) = f.into_host_fn_parts();
        self.registry.insert(
            (namespace.to_owned(), name.to_owned()),
            RegisteredFn { func, arg_types, ret_type },
        );
    }

    /// Compiles `src` into a reusable [`Program`], binding every `host` function
    /// it declares to the matching registered closure.
    ///
    /// `main` is executed once here (module instantiation), so any top-level
    /// setup runs before the host makes its first [`Program::call`].
    ///
    /// # Errors
    ///
    /// Returns a [`Diagnostic`] if the script fails to parse/type-check, if a
    /// declared `host` function has no registered closure, if a registered
    /// closure's arity/types disagree with the `host` block, or if running
    /// `main` raises a runtime error.
    pub fn compile(&self, src: &str, filename: &str) -> Result<Program, Diagnostic> {
        let filename_owned = filename.to_owned();
        let out: CompileOutput =
            collect_diagnostic(|| compile(src.to_owned(), &filename_owned, false))?;

        // Bind each declared host function to a registered closure, validating
        // arity + types against the closure's derived signature.
        let mut host_dispatch: Vec<HostDispatch> = Vec::with_capacity(out.host_fns.len());
        for sig in &out.host_fns {
            let key = (sig.namespace.to_string(), sig.name.to_string());
            let registered = self.registry.get(&key).ok_or_else(|| Diagnostic {
                filename: filename.to_owned(),
                span: 0..0,
                message: format!(
                    "no host function registered for `{}.{}` (declared in a `host` block)",
                    sig.namespace, sig.name
                ),
                code: String::from("unregistered_host_fn"),
            })?;
            validate_host_fn(sig, registered, filename)?;
            host_dispatch.push(Rc::clone(&registered.func));
        }

        // Register 0 is keel's void-return / null sink: a call whose result is
        // discarded writes `null` there. A normal program always has register 0
        // occupied by a constant, but an empty `main` can leave it free, which
        // would let a `Program::call` trampoline allocate a function parameter to
        // it and then have a void host call clobber that parameter. Reserve it.
        let mut registers = out.registers;
        let mut const_registers = out.const_registers;
        if registers.is_empty() {
            registers.push(NULL);
            const_registers.entry(NULL).or_insert(0);
        }

        let mut program = Program {
            instructions: out.instructions,
            registers,
            pools: out.pools,
            instr_src: out.instr_src,
            fn_registers: out.fn_registers,
            dyn_lib_fns: out.dyn_lib_fns,
            host_sigs: out.host_fns,
            host_dispatch,
            allocated_arg_count: out.allocated_arg_count,
            allocated_call_depth: out.allocated_call_depth,
            sources: out.sources,
            structs: out.structs,
            functions: out.functions,
            dyn_libs: out.dyn_libs,
            namespace: out.namespace,
            const_registers,
            free_registers: out.free_registers,
        };

        // Instantiate: run `main` once so top-level state is established before
        // the first host-driven call.
        program.execute_from(0)?;
        Ok(program)
    }
}

/// Checks that a registered closure's derived signature matches the `host`
/// block declaration it is bound to.
fn validate_host_fn(
    sig: &HostFnSig,
    registered: &RegisteredFn,
    filename: &str,
) -> Result<(), Diagnostic> {
    let err = |message: String| Diagnostic {
        filename: filename.to_owned(),
        span: 0..0,
        message,
        code: String::from("host_fn_signature_mismatch"),
    };

    if sig.arg_count() != registered.arg_types.len() {
        return Err(err(format!(
            "host function `{}.{}` is declared with {} argument(s) but the registered closure takes {}",
            sig.namespace,
            sig.name,
            sig.arg_count(),
            registered.arg_types.len(),
        )));
    }

    for (idx, want) in registered.arg_types.iter().enumerate() {
        let declared = HostType::from_datatype(sig.get_arg(idx)).ok_or_else(|| {
            err(format!(
                "host function `{}.{}` argument {} has a type that cannot cross the host boundary",
                sig.namespace,
                sig.name,
                idx + 1,
            ))
        })?;
        if declared != *want {
            return Err(err(format!(
                "host function `{}.{}` argument {} is declared `{}` but the registered closure expects `{}`",
                sig.namespace,
                sig.name,
                idx + 1,
                declared.as_str(),
                want.as_str(),
            )));
        }
    }

    let declared_ret = HostType::from_datatype(sig.get_return_type()).ok_or_else(|| {
        err(format!(
            "host function `{}.{}` has a return type that cannot cross the host boundary",
            sig.namespace, sig.name,
        ))
    })?;
    if declared_ret != registered.ret_type {
        return Err(err(format!(
            "host function `{}.{}` is declared to return `{}` but the registered closure returns `{}`",
            sig.namespace,
            sig.name,
            declared_ret.as_str(),
            registered.ret_type.as_str(),
        )));
    }

    Ok(())
}

/// A compiled keel program with resident interpreter state.
///
/// Registers and heap pools persist between [`Program::call`] invocations, so
/// state established by one call (including anything a host function mutates on
/// the Rust side) is visible to the next.
///
/// `Program` is single-threaded (`!Send`/`!Sync`): it holds `Rc` dispatchers
/// and reflects keel's single-threaded VM.
pub struct Program {
    // ---- VM state (persists across calls) ----
    instructions: Vec<Instr>,
    registers: Vec<Data>,
    pools: Pools,
    instr_src: Vec<InstrSrc>,
    fn_registers: Vec<Vec<u16>>,
    dyn_lib_fns: Vec<DynamicLibFn>,
    host_sigs: Vec<HostFnSig>,
    host_dispatch: Vec<HostDispatch>,
    allocated_arg_count: usize,
    allocated_call_depth: usize,
    sources: Vec<Source>,
    structs: Vec<Struct>,
    // ---- compiler state (drives on-demand call trampolines) ----
    functions: Vec<Function>,
    dyn_libs: Vec<Dynamiclib>,
    namespace: Namespace,
    const_registers: FxHashMap<Data, u16>,
    free_registers: Vec<u16>,
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
    /// # Errors
    ///
    /// Returns a [`Diagnostic`] if `fn_name` is unknown, if the arguments don't
    /// type-check against its signature, or if the call raises a runtime error.
    pub fn call(&mut self, fn_name: &str, args: &[Value]) -> Result<Value, Diagnostic> {
        let arg_exprs: Box<[Expr]> = args.iter().map(value_to_expr).collect();
        let arg_spans: Box<[Span]> = args.iter().map(|_| Span { start: 0, end: 0 }).collect();
        let call_expr = Expr::FunctionCall(
            arg_exprs,
            Box::from([SmolStr::from(fn_name)]),
            Span { start: 0, end: 0 },
            arg_spans,
        );

        // Compile the trampoline (type-checks the call) under a diagnostic sink.
        let (mut output, ret_id) = collect_diagnostic(|| self.build_trampoline(&call_expr))?;

        let ret_id = ret_id.unwrap_or(0);
        output.push(Instr::Halt(0));
        let start = self.instructions.len();
        self.instructions.extend(output);

        self.execute_from(start)?;

        Ok(data_to_value(
            self.registers[ret_id as usize],
            &self.pools.strings,
        ))
    }

    /// Compiles a call trampoline for `call_expr`, appending any freshly
    /// specialized function bodies to a local buffer whose instructions are
    /// absolute (offset by the current instruction count). Returns the buffer
    /// and the register holding the call's result.
    fn build_trampoline(&mut self, call_expr: &Expr) -> (Vec<Instr>, Option<u16>) {
        // A prior call whose trampoline aborted mid-inference (error unwind) may
        // have left stale entries in the return-type inference thread-local;
        // clear it so this compile starts clean, exactly as `compile()` does.
        crate::compiler::type_system::reset_inference_state();

        let offset = self.instructions.len() as u16;
        let ctx = Ctx {
            block_id: 0,
            is_compiling_recursive: false,
            single_run: false,
            file_idx: 0,
            offset,
        };
        let mut variables = Vec::new();
        let mut output = Vec::new();
        let mut state = State {
            registers: &mut self.registers,
            fns: &mut self.functions,
            structs: &mut self.structs,
            pools: &mut self.pools,
            instr_src: &mut self.instr_src,
            fn_registers: &mut self.fn_registers,
            dyn_libs: &mut self.dyn_libs,
            allocated_arg_count: &mut self.allocated_arg_count,
            allocated_call_depth: &mut self.allocated_call_depth,
            const_registers: &mut self.const_registers,
            free_registers: &mut self.free_registers,
            sources: &mut self.sources,
            reserved_registers: rustc_hash::FxHashSet::default(),
            namespace: &mut self.namespace,
        };
        let ret = call_expr.compile(&mut variables, ctx, &mut state, &mut output, None, false, true);
        (output, ret)
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
        let fn_registers = &self.fn_registers;
        let dyn_lib_fns = &self.dyn_lib_fns;
        let structs = &self.structs;
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
                fn_registers,
                dyn_lib_fns,
                structs,
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

/// Synthesizes a literal [`Expr`] carrying a [`Value`] so a host argument can be
/// compiled through the ordinary call path. keel integers are 32-bit, so
/// [`Value::Int`] is narrowed here.
fn value_to_expr(v: &Value) -> Expr {
    match v {
        Value::Null => Expr::Null,
        Value::Int(i) => Expr::Int(*i as i32),
        Value::Float(f) => Expr::Float(*f),
        Value::Bool(b) => Expr::Bool(*b),
        Value::String(s) => Expr::String(SmolStr::from(s.as_str())),
    }
}

/// Marshals a runtime [`Data`] register back into a host [`Value`]. Non-scalar
/// results (arrays, structs, maps) currently surface as [`Value::Null`].
fn data_to_value(d: Data, strings: &StringPool) -> Value {
    if d.is_int() {
        Value::Int(i64::from(d.as_int()))
    } else if d.is_float() {
        Value::Float(d.as_float())
    } else if d.is_bool() {
        Value::Bool(d.as_bool())
    } else if d.is_str() {
        Value::String(d.as_str(strings).to_owned())
    } else {
        Value::Null
    }
}
