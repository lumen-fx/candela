//! Bytecode artifact format (`.cdlb`) and the VM-only load/run API.
//!
//! This mirrors an AOT model: the full `candela` toolchain (compiler + VM)
//! compiles a `.cdl` source to a compact, self-contained bytecode artifact
//! ([`ProgramImage`] serialized via [`serialize_image`]), and the VM-only
//! `candela-vm` binary loads it ([`load_program`]) and runs it
//! ([`RuntimeProgram::run`]) WITHOUT the parser, compiler, or REPL.
//!
//! An embedding host gets the same two halves the `Engine`/`Program` pair
//! gives it, minus the compiler: it supplies a [`HostRegistry`] at load so the
//! artifact's `host` functions bind to Rust closures, and it invokes script
//! functions by name through [`RuntimeProgram::call`], which runs a trampoline
//! the compiler emitted at build time.
//!
//! The on-disk format is: a 4-byte magic (`CDLB`), a 1-byte format version, then
//! a `postcard`-encoded [`ProgramImage`]. `postcard` was chosen over `bincode`
//! because it is `no_std`/`alloc`-friendly and its varint encoding keeps the
//! artifact small; it also pulls in the least code, which matters for the
//! `candela-vm` binary-size budget.
//!
//! [`ProgramImage`] carries `pub` fields so the `candela` crate's `build`
//! subcommand can populate it from a fresh compile; everything needed to LOAD
//! and RUN it lives here.

use crate::data::Data;
use crate::embed::HostBindError;
use crate::embed::HostDispatch;
use crate::embed::HostRegistry;
use crate::embed::Value;
use crate::embed::describe_value;
use crate::embed::marshal_args;
use crate::embed::unmarshal_value;
use crate::embed::value_matches_type;
use crate::errors::Diagnostic;
use crate::errors::ErrorCtx;
use crate::instr::Instr;
use crate::rt::DataType;
use crate::rt::DynamicLibFn;
use crate::rt::EnumType;
use crate::rt::EnumVariant;
use crate::rt::GcStats;
use crate::rt::HostFnSig;
use crate::rt::InstrSrc;
use crate::rt::LibraryOrigin;
use crate::rt::Pools;
use crate::rt::Source;
use crate::rt::Span;
use crate::rt::Struct;
#[cfg(not(target_arch = "wasm32"))]
use crate::rt::TargetOs;
#[cfg(not(target_arch = "wasm32"))]
use crate::rt::resolve_library_filename;
use crate::vm;
use crate::vm::CandelaMap;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::Pool;
use crate::vm::RegisterFile;
use crate::vm::StringPool;
use serde::Deserialize;
use serde::Serialize;
use smol_strc::SmolStr;
use std::collections::HashMap;

/// The pair of words a value is recorded as. See [`Data::into_words`].
pub type DataWords = (u64, i64);

/// 4-byte artifact magic, so a non-`.cdlb` file fails cleanly.
const MAGIC: [u8; 4] = *b"CDLB";
/// Bumped whenever the serialized shape changes, so a mismatched artifact is
/// rejected instead of mis-decoded. Version 2 added the dynamic-library and
/// host-function recipe tables (`dyn_lib_fns`/`host_fns`). Version 3 added the
/// native-enum type table (`enums`) and the `CloneEnum` instruction. Version 4
/// added the map/json/any library functions (`Keys`/`Values`/`JsonParse`/
/// `JsonStringify` and the `is_*`/`as_*` value ops). Version 5 added the export
/// table (`exports`), the call trampolines that back
/// [`RuntimeProgram::call`]. Version 6 added the `MapRemove` instruction and
/// the origin of each dynamic-library recipe, which says whether its spec is
/// the program's own or a path into the standard library's `libs` directory.
/// Version 7 widened `int` to 64 bits, so every recorded value carries a
/// second word. Version 8 added the cell instructions a captured variable is
/// read and written through. Version 9 added the indirect call, which dispatches
/// on the function a value carries. Version 10 added the instruction that
/// builds a function value, which tells a function from the list it is built
/// as wherever one becomes text. Version 11 added the string comparison and
/// bitwise instructions, which sit among the existing ones and so renumber the
/// instructions after them, and gave the order of each map's recorded pairs a
/// meaning: a map is walked in the order its keys went in, so the pair list is
/// the program's order and an artifact from before the change cannot supply it.
/// Version 12 added the loop step and the add-a-constant instructions, which
/// sit among the existing ones and so renumber the instructions after them.
const FORMAT_VERSION: u8 = 12;

/// Serializable mirror of a compiled program's runtime state.
///
/// A dedicated DTO keeps `serde` off the hot runtime types: `Data` becomes its
/// pair of raw words, `SmolStr` becomes `String`, maps become key/value pairs.
/// Only [`Instr`], [`Span`], and [`DataType`] carry `serde` derives directly.
/// Fields are `pub` so the compiler front-end can build an image from a fresh
/// compile.
#[derive(Serialize, Deserialize)]
pub struct ProgramImage {
    pub instructions: Vec<Instr>,
    pub registers: Vec<DataWords>,
    pub objs: Vec<Vec<DataWords>>,
    pub maps: Vec<Vec<(DataWords, DataWords)>>,
    pub strings: Vec<String>,
    pub instr_src: Vec<InstrSrcImage>,
    /// The registers each recursive call site saves across its call, indexed by
    /// the call-site id its `SaveFrame` carries.
    pub callsite_registers: Vec<Vec<u16>>,
    pub structs: Vec<StructImage>,
    pub enums: Vec<EnumImage>,
    pub sources: Vec<SourceImage>,
    pub allocated_arg_count: u64,
    pub allocated_call_depth: u64,
    /// Recipe for each dynamic-library binding, in `DynamicLibFn` id order. The
    /// artifact stores only the logical library name, symbol, and marshalling
    /// signature, never the shared object's bytes, so the VM re-opens the
    /// library and re-binds the symbol by name at load time (see
    /// [`resolve_library_filename`]).
    pub dyn_lib_fns: Vec<DynLibFnImage>,
    /// Recipe for each `host` function declared by the program, in host-fn id
    /// order. The [`HostRegistry`] handed to [`load_program`] supplies the
    /// closure behind each one; an unregistered name is a load error.
    pub host_fns: Vec<HostFnImage>,
    /// One entry per host-callable function, each with the call trampoline the
    /// compiler emitted for it at build time. This is what lets the VM invoke a
    /// script function by name with no compiler present.
    pub exports: Vec<ExportImage>,
}

/// Serializable recipe for one dynamic-library binding: enough to re-open the
/// library and re-resolve the symbol by name at load, with no embedded bytes.
#[derive(Serialize, Deserialize)]
pub struct DynLibFnImage {
    /// Logical library name (`z`, `sqlite3`) or path. A program's own binding
    /// records it exactly as the source `dylib "..."` block wrote it; a
    /// standard-library one records it relative to the `libs` directory.
    pub library: String,
    /// The C symbol resolved within `library`.
    pub symbol: String,
    /// `[ return_type, arg_types... ]`, driving the rebuilt libffi CIF.
    pub types: Vec<DataType>,
    /// Which search `library` is resolved through at load.
    pub origin: LibraryOrigin,
}

/// Serializable recipe for one `host` function: its fully-qualified name and
/// marshalling signature, so an embedding runtime can re-bind it by name.
#[derive(Serialize, Deserialize)]
pub struct HostFnImage {
    pub namespace: String,
    pub name: String,
    /// `[ return_type, arg_types... ]`.
    pub types: Vec<DataType>,
    pub variadic: bool,
}

impl From<&HostFnImage> for HostFnSig {
    fn from(image: &HostFnImage) -> Self {
        Self {
            types: image.types.clone().into_boxed_slice(),
            namespace: SmolStr::from(image.namespace.as_str()),
            name: SmolStr::from(image.name.as_str()),
            variadic: image.variadic,
        }
    }
}

/// One host-callable function, with the call trampoline `candela build`
/// compiled for it.
///
/// The compiler specialises the function for its declared parameter types and
/// lays down a short instruction run that moves the parameter registers into
/// place, calls the specialisation, and halts. Invoking it is then a matter of
/// writing marshalled arguments into `arg_registers` and executing from
/// `entry`; the result is left in `ret_register`.
#[derive(Serialize, Deserialize)]
pub struct ExportImage {
    /// The name the function is called by, as written in the source.
    pub name: String,
    /// Instruction index the trampoline starts at.
    pub entry: u64,
    /// The register holding each parameter, in declaration order.
    pub arg_registers: Vec<u16>,
    /// The declared type of each parameter, checked against the host's
    /// arguments before the call runs.
    pub arg_types: Vec<DataType>,
    /// The register the trampoline leaves the result in. Register 0 is the null
    /// sink, which is what a function returning nothing lands on.
    pub ret_register: u16,
}

#[derive(Serialize, Deserialize)]
pub struct InstrSrcImage {
    pub instr: Instr,
    pub span: Span,
    pub file_id: u16,
}

#[derive(Serialize, Deserialize)]
pub struct StructImage {
    pub name: String,
    pub fields: Vec<(String, DataType, Span)>,
    pub id: u16,
    pub name_span: Span,
}

#[derive(Serialize, Deserialize)]
pub struct EnumImage {
    pub name: String,
    pub variants: Vec<EnumVariantImage>,
    pub id: u16,
    pub name_span: Span,
}

#[derive(Serialize, Deserialize)]
pub struct EnumVariantImage {
    pub name: String,
    pub payload: Vec<DataType>,
    pub name_span: Span,
}

#[derive(Serialize, Deserialize)]
pub struct SourceImage {
    pub filename: String,
    pub contents: String,
}

/// One host-callable function in a loaded program: where its trampoline starts,
/// which registers its parameters live in, and what it leaves the result in.
struct Export {
    entry: usize,
    arg_registers: Box<[u16]>,
    arg_types: Box<[DataType]>,
    ret_register: u16,
}

/// A loaded, ready-to-run program image with owned runtime state.
///
/// Produced by [`load_program`] from `.cdlb` bytes. [`Self::run`] executes it
/// exactly as the full `candela` binary runs a freshly compiled program, and
/// [`Self::call`] invokes an exported function by name. Registers and heap
/// pools stay resident between calls, so state one call establishes is visible
/// to the next.
pub struct RuntimeProgram {
    instructions: Vec<Instr>,
    registers: Vec<Data>,
    pools: Pools,
    instr_src: Vec<InstrSrc>,
    callsite_registers: Vec<Vec<u16>>,
    dyn_lib_fns: Vec<DynamicLibFn>,
    host_sigs: Vec<HostFnSig>,
    host_dispatch: Vec<HostDispatch>,
    exports: HashMap<String, Export>,
    structs: Vec<Struct>,
    enums: Vec<EnumType>,
    sources: Vec<Source>,
    allocated_arg_count: usize,
    allocated_call_depth: usize,
}

impl RuntimeProgram {
    /// Runs the program's `main` to completion.
    ///
    /// A runtime error prints its report and ends the process, matching the
    /// full `candela <file.cdl>` path exactly. A
    /// host that wants the error as a value instead runs this inside
    /// [`collect_diagnostic`](crate::collect_diagnostic).
    pub fn run(&mut self) {
        let err_ctx = ErrorCtx {
            instr_src: &self.instr_src,
            sources: &self.sources,
        };
        let mut register_file = RegisterFile(std::mem::take(&mut self.registers));
        let result = vm::execute(
            &self.instructions,
            &mut register_file,
            &mut self.pools,
            &err_ctx,
            &self.callsite_registers,
            &self.dyn_lib_fns,
            &self.structs,
            &self.enums,
            self.allocated_arg_count,
            self.allocated_call_depth,
            &self.host_sigs,
            &self.host_dispatch,
            0,
        );
        self.registers = std::mem::take(&mut register_file.0);
        if let Err(error) = result {
            error.report(&err_ctx);
        }
    }

    /// Invokes the exported function `name` with `args`, returning its value
    /// (or [`Value::Null`] for a function that returns nothing).
    ///
    /// The call runs the trampoline the compiler emitted for the function at
    /// build time, against the resident register and heap state, so globals a
    /// previous call or `main` established remain visible. Arguments are
    /// checked against the declared parameter types before anything runs.
    ///
    /// # Errors
    ///
    /// Returns a [`CallError`] if the artifact exports no such function, if the
    /// arguments disagree with the declared signature, or if the call raises a
    /// runtime error.
    pub fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, CallError> {
        let export = self
            .exports
            .get(name)
            .ok_or_else(|| CallError::UnknownFunction(name.to_owned()))?;

        if export.arg_registers.len() != args.len() {
            return Err(CallError::ArgCount {
                function: name.to_owned(),
                expected: export.arg_registers.len(),
                found: args.len(),
            });
        }
        for (idx, (arg, declared)) in args.iter().zip(export.arg_types.iter()).enumerate() {
            if !value_matches_type(arg, declared) {
                return Err(CallError::ArgType {
                    function: name.to_owned(),
                    index: idx + 1,
                    expected: declared.to_string(),
                    found: describe_value(arg),
                });
            }
        }

        let entry = export.entry;
        let ret_register = export.ret_register as usize;
        marshal_args(
            args,
            &export.arg_registers,
            &mut self.registers,
            &mut self.pools,
        );

        self.execute_from(entry)?;

        Ok(unmarshal_value(
            self.registers[ret_register],
            &self.pools.objs,
            &self.pools.maps,
            &self.pools.strings,
            &self.structs,
            &self.enums,
        ))
    }

    /// Does up to about `budget` units of garbage collection while the
    /// program is idle, and answers whether no collection work is left.
    ///
    /// Call it between calls into the program, where the host would otherwise
    /// wait, such as between frames. It carries on a collection already under
    /// way, or begins one when the heap has grown enough since the last; on a
    /// heap that has not, it does nothing and answers `true`. A unit is about
    /// one object traced or one freed slot, never a length of time, so the
    /// same calls do the same work on every machine. `collect(u32::MAX)` runs
    /// a whole collection. Beginning a collection reads every root at once
    /// whatever the budget.
    ///
    /// Once a host has called this, allocations leave more of the work to it
    /// and begin a collection themselves only when the heap has grown further.
    pub fn collect(&mut self, budget: u32) -> bool {
        self.pools.collect(&self.registers, budget)
    }

    /// What the collector has done over this program's heap, and how large
    /// the heap is. See [`GcStats`].
    #[must_use]
    pub fn gc_stats(&self) -> GcStats {
        self.pools.gc_stats()
    }

    /// The names of every function this artifact exports, in no particular
    /// order.
    pub fn exports(&self) -> impl Iterator<Item = &str> {
        self.exports.keys().map(String::as_str)
    }

    /// Runs the VM against the resident state starting at instruction `start`,
    /// capturing any runtime error as a [`Diagnostic`].
    fn execute_from(&mut self, start: usize) -> Result<(), Diagnostic> {
        let err_ctx = ErrorCtx {
            instr_src: &self.instr_src,
            sources: &self.sources,
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

        // An error comes back as a value, so a call sets up no unwind and
        // leaves the process panic hook alone.
        let result = vm::execute(
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

        self.registers = std::mem::take(&mut register_file.0);
        result.map_err(|error| error.diagnostic(&err_ctx))
    }
}

/// Why a [`RuntimeProgram::call`] did not produce a value.
#[derive(Debug)]
pub enum CallError {
    /// The artifact exports no function under this name.
    UnknownFunction(String),
    /// The call passed a different number of arguments than the function
    /// declares.
    ArgCount {
        function: String,
        expected: usize,
        found: usize,
    },
    /// An argument does not match the parameter type it was passed for.
    /// `index` is 1-based.
    ArgType {
        function: String,
        index: usize,
        expected: String,
        found: String,
    },
    /// The call ran and raised a runtime error.
    Runtime(Diagnostic),
}

impl From<Diagnostic> for CallError {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::Runtime(diagnostic)
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownFunction(name) => write!(
                f,
                "this artifact exports no function named '{name}'; only functions defined in the file it was built from, with every parameter annotated, are callable by name"
            ),
            Self::ArgCount {
                function,
                expected,
                found,
            } => write!(
                f,
                "'{function}' takes {expected} argument(s) but the call passed {found}"
            ),
            Self::ArgType {
                function,
                index,
                expected,
                found,
            } => write!(
                f,
                "argument {index} of '{function}' is declared '{expected}' but the call passed a {found}"
            ),
            Self::Runtime(diagnostic) => f.write_str(&diagnostic.message),
        }
    }
}

impl std::error::Error for CallError {}

impl RuntimeProgram {
    /// Reconstructs a runnable program from a decoded [`ProgramImage`].
    ///
    /// This is where a `.cdlb`'s dynamic-library and host recipes are turned
    /// back into live bindings: each `dylib` symbol is re-resolved through the
    /// OS loader by logical name (never from embedded bytes), and each `host`
    /// function is bound to the closure `hosts` registered under the same name,
    /// with its signature checked against the recorded declaration.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] if a referenced library cannot be opened, a symbol
    /// cannot be resolved, or a declared `host` function is unregistered or
    /// bound to a closure of a different shape.
    fn from_image(img: ProgramImage, hosts: &HostRegistry) -> Result<Self, LoadError> {
        let objs: ObjectPool = Pool(
            img.objs
                .into_iter()
                .map(|v| {
                    v.into_iter()
                        .map(|(boxed, int)| Data::from_words(boxed, int))
                        .collect()
                })
                .collect(),
        );
        let maps: MapPool = Pool(
            img.maps
                .into_iter()
                .map(|pairs| {
                    let mut m = CandelaMap::default();
                    for ((k_boxed, k_int), (v_boxed, v_int)) in pairs {
                        m.insert(
                            Data::from_words(k_boxed, k_int),
                            Data::from_words(v_boxed, v_int),
                        );
                    }
                    m
                })
                .collect(),
        );
        let strings = StringPool::from_strings(img.strings);

        let structs: Vec<Struct> = img
            .structs
            .into_iter()
            .map(|s| Struct {
                name: SmolStr::from(s.name),
                fields: s
                    .fields
                    .into_iter()
                    .map(|(n, t, sp)| (SmolStr::from(n), t, sp))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                id: s.id,
                name_span: s.name_span,
            })
            .collect();

        let enums: Vec<EnumType> = img
            .enums
            .into_iter()
            .map(|e| EnumType {
                name: SmolStr::from(e.name),
                variants: e
                    .variants
                    .into_iter()
                    .map(|v| EnumVariant {
                        name: SmolStr::from(v.name),
                        payload: v.payload.into_boxed_slice(),
                        name_span: v.name_span,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                id: e.id,
                name_span: e.name_span,
            })
            .collect();

        // `host` functions need a host-supplied closure to dispatch to. Bind
        // them all before anything runs, so an unbound call is impossible once
        // the program is loaded.
        let host_sigs: Vec<HostFnSig> = img.host_fns.iter().map(HostFnSig::from).collect();
        let host_dispatch = hosts.bind(&host_sigs)?;

        let dyn_lib_fns = resolve_dyn_lib_fns(&img.dyn_lib_fns, &structs)?;

        let exports = img
            .exports
            .into_iter()
            .map(|e| {
                (
                    e.name,
                    Export {
                        entry: e.entry as usize,
                        arg_registers: e.arg_registers.into_boxed_slice(),
                        arg_types: e.arg_types.into_boxed_slice(),
                        ret_register: e.ret_register,
                    },
                )
            })
            .collect();

        Ok(Self {
            instructions: img.instructions,
            registers: img
                .registers
                .into_iter()
                .map(|(boxed, int)| Data::from_words(boxed, int))
                .collect(),
            pools: Pools::new(objs, maps, strings),
            instr_src: img
                .instr_src
                .into_iter()
                .map(|s| InstrSrc {
                    instr: s.instr,
                    span: s.span,
                    file_id: s.file_id,
                })
                .collect(),
            callsite_registers: img.callsite_registers,
            dyn_lib_fns,
            host_sigs,
            host_dispatch,
            exports,
            structs,
            enums,
            sources: img
                .sources
                .into_iter()
                .map(|s| Source {
                    filename: SmolStr::from(s.filename),
                    contents: s.contents,
                })
                .collect(),
            allocated_arg_count: img.allocated_arg_count as usize,
            allocated_call_depth: img.allocated_call_depth as usize,
        })
    }
}

/// Re-opens each referenced dynamic library through the OS loader and rebuilds a
/// live [`DynamicLibFn`] (libffi CIF + code pointer) for every recorded symbol.
///
/// Libraries are opened once and shared across their symbols. The logical name
/// is mapped to this platform's filename convention via
/// [`resolve_library_filename`], so an artifact built on one OS resolves the
/// right file on another. The directories a host named with
/// [`set_dylib_dirs`](crate::rt::set_dylib_dirs) are searched before the
/// loader's own paths, which is how an application ships its libraries in a
/// directory of its own. A recipe the standard library owns is searched for
/// under the toolchain's `libs` directory as well, so an artifact that imports
/// `std/math` runs from any working directory.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_dyn_lib_fns(
    recipes: &[DynLibFnImage],
    structs: &[Struct],
) -> Result<Vec<DynamicLibFn>, LoadError> {
    use crate::rt::open_library;
    use libloading::Library;
    use std::rc::Rc;

    // Keyed by origin as well as filename: the same relative path means one
    // file under the toolchain's `libs` and another beside the program.
    let mut libs: HashMap<(LibraryOrigin, String), Rc<Library>> = HashMap::default();
    let mut out: Vec<DynamicLibFn> = Vec::with_capacity(recipes.len());

    for recipe in recipes {
        let filename = resolve_library_filename(&recipe.library, TargetOs::CURRENT);
        let key = (recipe.origin, filename.clone());
        let lib = if let Some(lib) = libs.get(&key) {
            Rc::clone(lib)
        } else {
            let lib = Rc::new(open_library(&filename, recipe.origin).map_err(|e| {
                LoadError::LibraryOpen {
                    spec: recipe.library.clone(),
                    filename: filename.clone(),
                    message: e.to_string(),
                }
            })?);
            libs.insert(key, Rc::clone(&lib));
            lib
        };

        let ptr = unsafe {
            let sym = lib
                .get::<*const ()>(recipe.symbol.as_bytes())
                .map_err(|_| LoadError::SymbolNotFound {
                    library: recipe.library.clone(),
                    symbol: recipe.symbol.clone(),
                })?;
            libffi::middle::CodePtr(sym.try_as_raw_ptr().ok_or_else(|| {
                LoadError::SymbolNotFound {
                    library: recipe.library.clone(),
                    symbol: recipe.symbol.clone(),
                }
            })?)
        };

        // types[0] is the return type; types[1..] are the argument types.
        let (return_type, arg_types) = recipe.types.split_first().unwrap_or((&DataType::Null, &[]));
        let cif = libffi::middle::Cif::new(
            arg_types.iter().map(|t| t.to_c_type(structs)),
            return_type.to_c_type(structs),
        );

        out.push(DynamicLibFn {
            types: recipe.types.clone().into_boxed_slice(),
            library: SmolStr::from(recipe.library.as_str()),
            origin: recipe.origin,
            symbol: SmolStr::from(recipe.symbol.as_str()),
            _lib: lib,
            ptr,
            cif,
        });
    }

    Ok(out)
}

/// On targets without the FFI backend (wasm) no library loads. A binding the
/// standard library owns resolves to the runtime's own function for it; any
/// other refuses the load rather than silently dropping the binding.
#[cfg(target_arch = "wasm32")]
fn resolve_dyn_lib_fns(
    recipes: &[DynLibFnImage],
    _structs: &[Struct],
) -> Result<Vec<DynamicLibFn>, LoadError> {
    recipes
        .iter()
        .map(|recipe| {
            let intrinsic = match recipe.origin {
                LibraryOrigin::StandardLibrary => {
                    crate::intrinsics::lookup(&recipe.library, &recipe.symbol)
                }
                LibraryOrigin::Program => None,
            };
            let Some(intrinsic) = intrinsic else {
                return Err(LoadError::LibraryOpen {
                    spec: recipe.library.clone(),
                    filename: recipe.library.clone(),
                    message: String::from("dynamic libraries are not supported on this target"),
                });
            };
            Ok(DynamicLibFn {
                types: recipe.types.clone().into_boxed_slice(),
                library: SmolStr::from(recipe.library.as_str()),
                origin: recipe.origin,
                symbol: SmolStr::from(recipe.symbol.as_str()),
                intrinsic,
            })
        })
        .collect()
}

/// Why a `.cdlb` artifact could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The magic header is missing (not a `.cdlb` file).
    BadMagic,
    /// The file is shorter than the fixed header.
    Truncated,
    /// The artifact's format version is not understood by this runtime.
    UnsupportedVersion(u8),
    /// The `postcard` body failed to decode.
    Decode(postcard::Error),
    /// A referenced dynamic library could not be opened by the OS loader.
    LibraryOpen {
        /// Logical name/path as written in the source.
        spec: String,
        /// The platform filename it mapped to at load.
        filename: String,
        /// The loader's error text.
        message: String,
    },
    /// A symbol declared by a `dylib` block was not found in its library.
    SymbolNotFound { library: String, symbol: String },
    /// The artifact's `host` functions could not be bound to the registry the
    /// load was given.
    HostBinding(HostBindError),
}

impl From<HostBindError> for LoadError {
    fn from(error: HostBindError) -> Self {
        Self::HostBinding(error)
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a candela bytecode artifact (bad magic header)"),
            Self::Truncated => write!(f, "truncated candela bytecode artifact"),
            Self::UnsupportedVersion(v) => write!(
                f,
                "unsupported .cdlb format version {v} (this runtime understands version {FORMAT_VERSION})"
            ),
            Self::Decode(e) => write!(f, "failed to decode .cdlb body: {e}"),
            Self::LibraryOpen {
                spec,
                filename,
                message,
            } => write!(
                f,
                "cannot open dynamic library '{spec}' (resolved to '{filename}') referenced by this artifact: {message}"
            ),
            Self::SymbolNotFound { library, symbol } => write!(
                f,
                "dynamic library '{library}' does not export the symbol '{symbol}' this artifact needs"
            ),
            Self::HostBinding(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Loads a `.cdlb` bytecode artifact into a runnable [`RuntimeProgram`].
///
/// `hosts` supplies the Rust closures the artifact's `host` blocks bind to.
/// Pass an empty [`HostRegistry`] for a program that declares none; a program
/// that declares one it does not cover fails to load, naming what is missing.
///
/// # Errors
///
/// Returns a [`LoadError`] if the magic/version header is wrong, the body fails
/// to decode, a dynamic library cannot be re-bound, or a `host` function cannot
/// be bound to the registry.
pub fn load_program(bytes: &[u8], hosts: &HostRegistry) -> Result<RuntimeProgram, LoadError> {
    if bytes.len() < 5 {
        return Err(LoadError::Truncated);
    }
    if bytes[0..4] != MAGIC {
        return Err(LoadError::BadMagic);
    }
    let version = bytes[4];
    if version != FORMAT_VERSION {
        return Err(LoadError::UnsupportedVersion(version));
    }
    let img: ProgramImage = postcard::from_bytes(&bytes[5..]).map_err(LoadError::Decode)?;
    RuntimeProgram::from_image(img, hosts)
}

/// Serializes a [`ProgramImage`] to `.cdlb` bytes (magic + version + body).
///
/// # Errors
///
/// Returns the serialization error as a string if the `postcard` body cannot be
/// encoded.
pub fn serialize_image(image: &ProgramImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    bytes.push(FORMAT_VERSION);
    let body = postcard::to_allocvec(image).map_err(|e| e.to_string())?;
    bytes.extend_from_slice(&body);
    Ok(bytes)
}
