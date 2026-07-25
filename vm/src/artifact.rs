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
use crate::rt::InstrSrc;
use crate::rt::Pools;
use crate::rt::Source;
use crate::rt::Span;
use crate::rt::Struct;
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
/// rejected instead of mis-decoded.
const FORMAT_VERSION: u8 = 1;

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
    pub sources: Vec<SourceImage>,
    pub allocated_arg_count: u64,
    pub allocated_call_depth: u64,
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
            self.allocated_arg_count,
            self.allocated_call_depth,
            &[],
            &[],
            0,
        );
        self.registers = std::mem::take(&mut register_file.0);
    }
}

impl From<ProgramImage> for RuntimeProgram {
    fn from(img: ProgramImage) -> Self {
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

        Self {
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
            dyn_lib_fns: Vec::new(),
            structs: img
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
                .collect(),
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
        }
    }
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
    Ok(RuntimeProgram::from(img))
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
