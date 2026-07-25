//! Bytecode artifact format (`.cdlb`) and the lean VM-only load/run API.
//!
//! This mirrors an AOT model: the fat `candela` binary compiles a `.cdl` source
//! to a compact, self-contained bytecode artifact (`build_bytecode`), and the
//! lean `candela-vm` binary loads it (`load_program`) and runs it
//! (`RuntimeProgram::run`) WITHOUT linking the parser, compiler, or REPL.
//!
//! The on-disk format is: a 4-byte magic (`CDLB`), a 1-byte format version, then
//! a `postcard`-encoded [`ProgramImage`]. `postcard` was chosen over `bincode`
//! because it is `no_std`/`alloc`-friendly and its varint encoding keeps the
//! artifact small; it also pulls in the least code, which matters for the
//! `candela-vm` binary-size budget.

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
/// [`Span`], and [`DataType`] carry `serde` derives directly.
#[derive(Serialize, Deserialize)]
struct ProgramImage {
    instructions: Vec<Instr>,
    registers: Vec<u64>,
    objs: Vec<Vec<u64>>,
    maps: Vec<Vec<(u64, u64)>>,
    strings: Vec<String>,
    instr_src: Vec<InstrSrcImage>,
    fn_registers: Vec<Vec<u16>>,
    structs: Vec<StructImage>,
    sources: Vec<SourceImage>,
    allocated_arg_count: u64,
    allocated_call_depth: u64,
}

#[derive(Serialize, Deserialize)]
struct InstrSrcImage {
    instr: Instr,
    span: Span,
    file_id: u16,
}

#[derive(Serialize, Deserialize)]
struct StructImage {
    name: String,
    fields: Vec<(String, DataType, Span)>,
    id: u16,
    name_span: Span,
}

#[derive(Serialize, Deserialize)]
struct SourceImage {
    filename: String,
    contents: String,
}

/// A loaded, ready-to-run program image with owned runtime state.
///
/// Produced by [`load_program`] from `.cdlb` bytes. [`Self::run`] executes it
/// exactly as the fat `candela` binary runs a freshly compiled program.
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
    /// `throw_error`), matching the fat `candela <file.cdl>` path exactly.
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

/// Compiles a `.cdl` source string to a `.cdlb` bytecode artifact.
///
/// # Errors
///
/// Returns an error string if the program uses features that cannot be captured
/// in a standalone artifact yet (dynamic C-library `import`s or `host` blocks),
/// or if serialization fails.
#[cfg(feature = "compiler")]
pub fn build_bytecode(source: String, filename: &str) -> Result<Vec<u8>, String> {
    let out = crate::compiler::compile(source, filename, false);
    let image = image_from_output(out)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    bytes.push(FORMAT_VERSION);
    let body = postcard::to_allocvec(&image).map_err(|e| e.to_string())?;
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

#[cfg(feature = "compiler")]
fn image_from_output(out: crate::compiler::CompileOutput) -> Result<ProgramImage, String> {
    if !out.dyn_lib_fns.is_empty() {
        return Err(String::from(
            "dynamic C-library imports (`import \"lib.so\"`) are not yet supported in .cdlb artifacts",
        ));
    }
    if !out.host_fns.is_empty() {
        return Err(String::from(
            "`host` blocks require an embedding Engine and cannot be captured in a .cdlb artifact",
        ));
    }

    Ok(ProgramImage {
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
    })
}
