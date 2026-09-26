//! Machine code for a program, generated when `candela build` writes its
//! `.cdlb`.
//!
//! The finished bytecode and the compiler's function table are lowered once
//! to a shared IR ([`ir`], built by [`lower`]), and a backend turns the IR
//! into code for one machine ([`clif`], Cranelift). The result is a
//! [`NativeImage`] the artifact carries beside the bytecode; `candela-vm`
//! maps it at load and runs it in place of the functions it covers.
//!
//! Nothing here runs in the VM: the code generator lives in the compiler
//! only, and the VM carries a loader.

mod clif;
mod inline;
pub mod ir;
mod lower;

use candela_vm::artifact::FunctionTable;
use candela_vm::artifact::NativeArch;
use candela_vm::artifact::NativeFunctionImage;
use candela_vm::artifact::NativeImage;
use candela_vm::artifact::NativeOs;
use candela_vm::artifact::NativeSiteImage;
use candela_vm::artifact::ProgramImage;
use candela_vm::native::abi;
use cranelift_codegen::isa;
use cranelift_codegen::settings;
use cranelift_codegen::settings::Configurable;
use std::str::FromStr;
use target_lexicon::Architecture;
use target_lexicon::OperatingSystem;
use target_lexicon::Triple;

/// A machine code can be generated for.
#[derive(Clone, Debug)]
pub struct Target {
    triple: Triple,
    arch: NativeArch,
    os: NativeOs,
}

impl Target {
    /// The machine the compiler runs on, when code can be generated for it.
    #[must_use]
    pub fn host() -> Option<Self> {
        Self::from_triple(Triple::host()).ok()
    }

    /// The target a triple such as `aarch64-apple-darwin` names.
    ///
    /// # Errors
    ///
    /// Names the triple when it is not one code can be generated for:
    /// x86-64 or aarch64, on Linux, macOS or Windows.
    pub fn parse(triple: &str) -> Result<Self, String> {
        let parsed = Triple::from_str(triple).map_err(|e| format!("{triple}: {e}"))?;
        Self::from_triple(parsed).map_err(|()| {
            format!(
                "{triple} is not a target candela generates machine code for; it does for x86_64 and aarch64 on Linux, macOS and Windows"
            )
        })
    }

    fn from_triple(triple: Triple) -> Result<Self, ()> {
        let arch = match triple.architecture {
            Architecture::X86_64 => NativeArch::X86_64,
            Architecture::Aarch64(_) => NativeArch::Aarch64,
            _ => return Err(()),
        };
        let os = match triple.operating_system {
            OperatingSystem::Linux => NativeOs::Linux,
            OperatingSystem::Darwin(_) | OperatingSystem::MacOSX(_) => NativeOs::MacOs,
            OperatingSystem::Windows if arch == NativeArch::X86_64 => NativeOs::Windows,
            _ => return Err(()),
        };
        Ok(Self { triple, arch, os })
    }
}

/// Machine code for the functions of `image` that qualify, for `target`.
/// `None` when no function qualifies.
///
/// # Errors
///
/// Returns what the code generator refused. The program itself is fine, and
/// runs on bytecode.
pub fn generate(
    image: &ProgramImage,
    table: &FunctionTable,
    target: &Target,
) -> Result<Option<NativeImage>, String> {
    let module = lower::lower(image, table);
    if module.funcs.is_empty() {
        return Ok(None);
    }
    let mut flags = settings::builder();
    let set = |flags: &mut settings::Builder, name: &str, value: &str| {
        flags
            .set(name, value)
            .map_err(|e| format!("cranelift setting {name}: {e}"))
    };
    set(&mut flags, "opt_level", "speed")?;
    set(&mut flags, "is_pic", "true")?;
    set(
        &mut flags,
        "enable_verifier",
        if cfg!(debug_assertions) {
            "true"
        } else {
            "false"
        },
    )?;
    set(&mut flags, "unwind_info", "false")?;
    // A frame larger than a page touches every page on the way down, so a
    // guard page is never skipped.
    set(&mut flags, "enable_probestack", "true")?;
    set(&mut flags, "probestack_strategy", "inline")?;
    let isa = isa::lookup(target.triple.clone())
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let layout = abi::object_layout();
    let code = clif::compile(&module, &*isa, layout)?;

    let functions = module
        .funcs
        .iter()
        .zip(&code.entries)
        .map(|(f, offset)| NativeFunctionImage {
            loc: f.loc,
            offset: *offset,
            returns: f.ret.is_some(),
        })
        .collect();
    let sites = module
        .sites
        .iter()
        .map(|&(at, function)| NativeSiteImage { at, function })
        .collect();
    Ok(Some(NativeImage {
        abi: abi::VERSION,
        arch: target.arch,
        os: target.os,
        // The baseline of each architecture: x86-64 as first defined, and
        // armv8.0.
        cpu_features: Vec::new(),
        object_layout: layout,
        frame_bytes: code.frame_bytes,
        functions,
        main: module.main,
        sites,
        code: code.bytes,
    }))
}
