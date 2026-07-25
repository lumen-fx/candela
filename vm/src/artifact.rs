//! Bytecode artifact format (`.cdlb`) and the VM-only load/run API.
//!
//! This mirrors an AOT model: the full `candela` toolchain (compiler + VM)
//! compiles a `.cdl` source to a compact, self-contained bytecode artifact
//! ([`ProgramImage`] serialized via [`serialize_image`]), and the VM-only
//! `candela-vm` binary loads it ([`load_program`]) and runs it
//! ([`RuntimeProgram::run`]) WITHOUT the parser, compiler, or REPL.
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
use crate::data::DataHash;
use crate::errors::ErrorCtx;
use crate::instr::Instr;
use crate::rt::DataType;
use crate::rt::DynamicLibFn;
use crate::rt::EnumType;
use crate::rt::EnumVariant;
use crate::rt::InstrSrc;
use crate::rt::Pools;
use crate::rt::Source;
use crate::rt::Span;
use crate::rt::Struct;
#[cfg(not(target_arch = "wasm32"))]
use crate::rt::TargetOs;
#[cfg(not(target_arch = "wasm32"))]
use crate::rt::resolve_library_filename;
use crate::vm;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::Pool;
use crate::vm::RegisterFile;
use crate::vm::StringPool;
use serde::Deserialize;
use serde::Serialize;
use smol_strc::SmolStr;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;

/// Runtime map: `Data`-keyed, hashed by the raw NaN-boxed bits.
type CandelaMap = HashMap<Data, Data, BuildHasherDefault<DataHash>>;

/// 4-byte artifact magic, so a non-`.cdlb` file fails cleanly.
const MAGIC: [u8; 4] = *b"CDLB";
/// Bumped whenever the serialized shape changes, so a mismatched artifact is
/// rejected instead of mis-decoded. Version 2 added the dynamic-library and
/// host-function recipe tables (`dyn_lib_fns`/`host_fns`). Version 3 added the
/// native-enum type table (`enums`) and the `CloneEnum` instruction. Version 4
/// added the map/json/any library functions (`Keys`/`Values`/`JsonParse`/
/// `JsonStringify` and the `is_*`/`as_*` value ops).
const FORMAT_VERSION: u8 = 4;

/// Serializable mirror of a compiled program's runtime state.
///
/// A dedicated DTO keeps `serde` off the hot runtime types: `Data` becomes a raw
/// `u64`, `SmolStr` becomes `String`, maps become key/value pairs. Only [`Instr`],
/// [`Span`], and [`DataType`] carry `serde` derives directly. Fields are `pub`
/// so the compiler front-end can build an image from a fresh compile.
#[derive(Serialize, Deserialize)]
pub struct ProgramImage {
    pub instructions: Vec<Instr>,
    pub registers: Vec<u64>,
    pub objs: Vec<Vec<u64>>,
    pub maps: Vec<Vec<(u64, u64)>>,
    pub strings: Vec<String>,
    pub instr_src: Vec<InstrSrcImage>,
    pub fn_registers: Vec<Vec<u16>>,
    pub structs: Vec<StructImage>,
    pub enums: Vec<EnumImage>,
    pub sources: Vec<SourceImage>,
    pub allocated_arg_count: u64,
    pub allocated_call_depth: u64,
    /// Recipe for each dynamic-library binding, in `DynamicLibFn` id order. The
    /// artifact stores only the logical library name, symbol, and marshalling
    /// signature -- never the shared object's bytes -- so the VM re-opens the
    /// library and re-binds the symbol by name at load time (see
    /// [`resolve_library_filename`]).
    pub dyn_lib_fns: Vec<DynLibFnImage>,
    /// Recipe for each `host` function declared by the program, in host-fn id
    /// order. Standalone `candela-vm` has no embedder to bind these to, so a
    /// non-empty list makes the artifact fail to load with a clear error naming
    /// the host function; embedding runtimes carry their own binding path.
    pub host_fns: Vec<HostFnImage>,
}

/// Serializable recipe for one dynamic-library binding: enough to re-open the
/// library and re-resolve the symbol by name at load, with no embedded bytes.
#[derive(Serialize, Deserialize)]
pub struct DynLibFnImage {
    /// Logical library name (`z`, `sqlite3`) or path, exactly as written in the
    /// source `dylib "..."` block.
    pub library: String,
    /// The C symbol resolved within `library`.
    pub symbol: String,
    /// `[ return_type, arg_types... ]`, driving the rebuilt libffi CIF.
    pub types: Vec<DataType>,
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

/// A loaded, ready-to-run program image with owned runtime state.
///
/// Produced by [`load_program`] from `.cdlb` bytes. [`Self::run`] executes it
/// exactly as the full `candela` binary runs a freshly compiled program.
pub struct RuntimeProgram {
    instructions: Vec<Instr>,
    registers: Vec<Data>,
    pools: Pools,
    instr_src: Vec<InstrSrc>,
    fn_registers: Vec<Vec<u16>>,
    dyn_lib_fns: Vec<DynamicLibFn>,
    structs: Vec<Struct>,
    enums: Vec<EnumType>,
    sources: Vec<Source>,
    allocated_arg_count: usize,
    allocated_call_depth: usize,
}

impl RuntimeProgram {
    /// Runs the program's `main` to completion.
    ///
    /// Runtime errors are printed and abort the process (via the VM's
    /// `throw_error`), matching the full `candela <file.cdl>` path exactly.
    pub fn run(&mut self) {
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

        let mut register_file = RegisterFile(std::mem::take(&mut self.registers));
        vm::execute(
            &self.instructions,
            &mut register_file,
            &mut self.pools,
            &err_ctx,
            &self.fn_registers,
            &self.dyn_lib_fns,
            &self.structs,
            &self.enums,
            self.allocated_arg_count,
            self.allocated_call_depth,
            &[],
            &[],
            0,
        );
        self.registers = std::mem::take(&mut register_file.0);
    }
}

impl RuntimeProgram {
    /// Reconstructs a runnable program from a decoded [`ProgramImage`].
    ///
    /// This is where a `.cdlb`'s dynamic-library and host recipes are turned
    /// back into live bindings: each `dylib` symbol is re-resolved through the
    /// OS loader by logical name (never from embedded bytes), and any `host`
    /// function makes loading fail cleanly because the standalone runtime has no
    /// embedder to bind it to.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] if a referenced library cannot be opened, a symbol
    /// cannot be resolved, or the artifact declares `host` functions.
    fn from_image(img: ProgramImage) -> Result<Self, LoadError> {
        let objs: ObjectPool = Pool(
            img.objs
                .into_iter()
                .map(|v| v.into_iter().map(Data).collect())
                .collect(),
        );
        let maps: MapPool = Pool(
            img.maps
                .into_iter()
                .map(|pairs| {
                    let mut m: CandelaMap = HashMap::default();
                    for (k, v) in pairs {
                        m.insert(Data(k), Data(v));
                    }
                    m
                })
                .collect(),
        );
        let strings: StringPool = Pool(img.strings);

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

        // `host` functions need an embedder-supplied closure to dispatch to; the
        // standalone runtime has none, so refuse to load rather than risk an
        // unbound call at runtime. Name the offending function so the failure is
        // actionable.
        if let Some(h) = img.host_fns.first() {
            let name = if h.namespace.is_empty() {
                h.name.clone()
            } else {
                format!("{}::{}", h.namespace, h.name)
            };
            return Err(LoadError::MissingHostFn(name));
        }

        let dyn_lib_fns = resolve_dyn_lib_fns(&img.dyn_lib_fns, &structs)?;

        Ok(Self {
            instructions: img.instructions,
            registers: img.registers.into_iter().map(Data).collect(),
            pools: Pools {
                objs,
                maps,
                strings,
            },
            instr_src: img
                .instr_src
                .into_iter()
                .map(|s| InstrSrc {
                    instr: s.instr,
                    span: s.span,
                    file_id: s.file_id,
                })
                .collect(),
            fn_registers: img.fn_registers,
            dyn_lib_fns,
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
/// right file on another.
#[cfg(not(target_arch = "wasm32"))]
fn resolve_dyn_lib_fns(
    recipes: &[DynLibFnImage],
    structs: &[Struct],
) -> Result<Vec<DynamicLibFn>, LoadError> {
    use libloading::Library;
    use std::rc::Rc;

    let mut libs: HashMap<String, Rc<Library>> = HashMap::default();
    let mut out: Vec<DynamicLibFn> = Vec::with_capacity(recipes.len());

    for recipe in recipes {
        let filename = resolve_library_filename(&recipe.library, TargetOs::CURRENT);
        let lib = if let Some(lib) = libs.get(&filename) {
            Rc::clone(lib)
        } else {
            let lib = Rc::new(unsafe {
                Library::new(&filename).map_err(|e| LoadError::LibraryOpen {
                    spec: recipe.library.clone(),
                    filename: filename.clone(),
                    message: e.to_string(),
                })?
            });
            libs.insert(filename.clone(), Rc::clone(&lib));
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
            symbol: SmolStr::from(recipe.symbol.as_str()),
            _lib: lib,
            ptr,
            cif,
        });
    }

    Ok(out)
}

/// On targets without the FFI backend (wasm) a `.cdlb` that references dynamic
/// libraries cannot be honored; refuse to load it rather than silently drop the
/// bindings.
#[cfg(target_arch = "wasm32")]
fn resolve_dyn_lib_fns(
    recipes: &[DynLibFnImage],
    _structs: &[Struct],
) -> Result<Vec<DynamicLibFn>, LoadError> {
    if let Some(r) = recipes.first() {
        return Err(LoadError::LibraryOpen {
            spec: r.library.clone(),
            filename: r.library.clone(),
            message: String::from("dynamic libraries are not supported on this target"),
        });
    }
    Ok(Vec::new())
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
    /// The artifact declares a `host` function, which requires an embedding
    /// runtime; the standalone VM cannot bind it.
    MissingHostFn(String),
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
            Self::MissingHostFn(name) => write!(
                f,
                "this artifact needs the host function '{name}', which requires an embedding runtime and cannot be provided by the standalone candela-vm"
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Loads a `.cdlb` bytecode artifact into a runnable [`RuntimeProgram`].
///
/// # Errors
///
/// Returns a [`LoadError`] if the magic/version header is wrong or the body
/// fails to decode.
pub fn load_program(bytes: &[u8]) -> Result<RuntimeProgram, LoadError> {
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
    RuntimeProgram::from_image(img)
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
