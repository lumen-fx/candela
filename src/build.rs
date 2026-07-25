//! The `candela build` path: compile a `.cdl` source to a `.cdlb` bytecode
//! artifact.
//!
//! The artifact FORMAT and its load/run half live in the VM-only `candela-vm`
//! crate ([`candela_vm::artifact`]); this module is the compiler-side half that
//! turns a fresh [`compile`] result into a [`ProgramImage`] and serializes it.

use crate::compiler::CompileOutput;
use crate::compiler::compile;
use candela_vm::artifact::InstrSrcImage;
use candela_vm::artifact::ProgramImage;
use candela_vm::artifact::SourceImage;
use candela_vm::artifact::StructImage;
use candela_vm::artifact::serialize_image;

/// Compiles a `.cdl` source string to a `.cdlb` bytecode artifact.
///
/// # Errors
///
/// Returns an error string if the program uses features that cannot be captured
/// in a standalone artifact yet (dynamic C-library `import`s or `host` blocks),
/// or if serialization fails.
pub fn build_bytecode(source: String, filename: &str) -> Result<Vec<u8>, String> {
    let out = compile(source, filename, false);
    let image = image_from_output(out)?;
    serialize_image(&image)
}

fn image_from_output(out: CompileOutput) -> Result<ProgramImage, String> {
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
