//! The `candela build` path: compile a `.cdl` source to a `.cdlb` bytecode
//! artifact.
//!
//! The artifact format and its load/run half live in the VM-only `candela-vm`
//! crate ([`candela_vm::artifact`]); this module is the compiler-side half that
//! turns a fresh [`compile_checked_profile`] result into a [`ProgramImage`] and
//! serializes it.
//!
//! The VM carries no compiler, so it cannot build a call trampoline when a host
//! asks for a function by name. The trampolines are compiled ahead of time by
//! [`compile_checked_profile`] and recorded in the artifact's export table.

use crate::compiler::CompileOutput;
use crate::compiler::function_table::function_table;
use crate::compiler::imports::ImportResolver;
use crate::trampoline::compile_checked_profile;
use candela_vm::artifact::DynLibFnImage;
use candela_vm::artifact::EnumImage;
use candela_vm::artifact::EnumVariantImage;
use candela_vm::artifact::ExportImage;
use candela_vm::artifact::FunctionTable;
use candela_vm::artifact::HostFnImage;
use candela_vm::artifact::InstrSrcImage;
use candela_vm::artifact::NativeImage;
use candela_vm::artifact::ProgramImage;
use candela_vm::artifact::SourceImage;
use candela_vm::artifact::StructImage;
use candela_vm::artifact::serialize_image;

/// Compiles a `.cdl` source string to a `.cdlb` bytecode artifact.
///
/// The artifact captures the whole program: every imported workspace `.cdl`
/// module is linked into the single serialized image, so the resulting `.cdlb`
/// runs under `candela-vm` with no source tree present. Dynamic-library `import`s
/// and `host` blocks are captured as recipes (logical name + symbol +
/// signature) that the VM re-binds by name at load time, never as embedded
/// binary bytes.
///
/// Every fully annotated function is compiled at its declared parameter types
/// before the image is built, so a body error fails the build even in a
/// function `main` never calls, instead of dropping that function silently.
///
/// `resolver` says where a library import reads from, the same as it does for a
/// run from source, so a program that depends on packages builds into an
/// artifact that carries them.
///
/// # Errors
///
/// Returns an error string if serialization fails. A compile diagnostic
/// travels by the error funnel, not through this `Result`.
pub fn build_bytecode(
    source: String,
    filename: &str,
    resolver: &ImportResolver,
) -> Result<Vec<u8>, String> {
    build_artifact(source, filename, resolver, &BuildOptions::default())
}

/// How a program is built into an artifact.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    /// The release profile: the passes that make the program faster to run,
    /// and machine code. `false` builds the program exactly as a run from
    /// source compiles it.
    pub release: bool,
    /// The machine code the artifact carries.
    pub native: NativeCode,
}

impl Default for BuildOptions {
    /// The release profile, with machine code for the machine building.
    fn default() -> Self {
        Self {
            release: true,
            native: NativeCode::Host,
        }
    }
}

/// Which machine code a release build puts in the artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeCode {
    /// None: the program runs on bytecode everywhere.
    None,
    /// Code for the machine the build runs on, when candela generates code
    /// for it.
    Host,
    /// Code for the machine a target triple such as `aarch64-apple-darwin`
    /// names.
    Target(String),
}

/// Compiles a `.cdl` source string to a `.cdlb` artifact the way `options`
/// asks. [`build_bytecode`] is this with the defaults `candela build` uses.
///
/// Machine code covers the functions the code generator handles, and the VM
/// runs the bytecode for everything else, and for the whole program on a
/// machine the code was not generated for. A candela built without its
/// `native` feature generates none.
///
/// # Errors
///
/// Returns an error string if serialization fails, or if `options` names a
/// target candela does not generate code for. A compile diagnostic travels by
/// the error funnel, not through this `Result`.
pub fn build_artifact(
    source: String,
    filename: &str,
    resolver: &ImportResolver,
    options: &BuildOptions,
) -> Result<Vec<u8>, String> {
    let (out, exports) = compile_checked_profile(source, filename, resolver, options.release);
    // An artifact runs from `main`, so packaging a file without one would write
    // a program that cannot start.
    out.require_main();
    let table = function_table(&out);
    let mut image = image_from_output(out, exports);
    if options.release {
        image.native = native_sections(&image, &table, &options.native)?;
    }
    serialize_image(&image, Some(&table))
}

/// The machine code sections for `image`.
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
fn native_sections(
    image: &ProgramImage,
    table: &FunctionTable,
    native: &NativeCode,
) -> Result<Vec<NativeImage>, String> {
    use crate::native::Target;
    let target = match native {
        NativeCode::None => return Ok(Vec::new()),
        NativeCode::Host => match Target::host() {
            Some(target) => target,
            None => return Ok(Vec::new()),
        },
        NativeCode::Target(triple) => Target::parse(triple)?,
    };
    match crate::native::generate(image, table, &target) {
        Ok(section) => Ok(section.into_iter().collect()),
        // The program is sound; the code generator is what refused. A test
        // build says so, and a release build carries on with bytecode, which
        // runs the program all the same.
        Err(e) if cfg!(debug_assertions) => Err(format!("machine code generation failed: {e}")),
        Err(_) => Ok(Vec::new()),
    }
}

/// Without the code generator there is no machine code to add.
#[cfg(not(all(feature = "native", not(target_arch = "wasm32"))))]
fn native_sections(
    _image: &ProgramImage,
    _table: &FunctionTable,
    native: &NativeCode,
) -> Result<Vec<NativeImage>, String> {
    match native {
        NativeCode::Target(triple) => Err(format!(
            "this candela generates no machine code, so it cannot build for {triple}; it was built without the native feature"
        )),
        NativeCode::None | NativeCode::Host => Ok(Vec::new()),
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
            origin: d.origin,
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
        registers: out.registers.iter().map(|d| d.into_words()).collect(),
        objs: out
            .pools
            .objs
            .0
            .iter()
            .map(|v| v.iter().map(|d| d.into_words()).collect())
            .collect(),
        maps: out
            .pools
            .maps
            .0
            .iter()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.into_words(), v.into_words()))
                    .collect()
            })
            .collect(),
        strings: out.pools.strings.iter().cloned().collect(),
        instr_src: out
            .instr_src
            .iter()
            .map(|s| InstrSrcImage {
                instr: s.instr,
                span: s.span,
                file_id: s.file_id,
            })
            .collect(),
        callsite_registers: out.callsite_registers,
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
        native: Vec::new(),
    }
}
