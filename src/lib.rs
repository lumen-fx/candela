//! `candela` is the full toolchain: lexer, parser, type-checker, compiler,
//! REPL, the `Engine`/`Program` embedding API, and the `candela build`
//! subcommand.
//!
//! The runtime core (the VM executor, bytecode/data types, GC, value
//! marshalling, and the `.cdlb` load/run API) lives in the self-contained
//! `candela-vm` crate. This crate depends on it (strictly `candela ->
//! candela-vm`) and aliases its modules under `crate::` so the compiler keeps
//! its `crate::data` / `crate::instr` / `crate::rt` / `crate::errors` /
//! `crate::vm` paths.

#[cfg(all(feature = "compiler", any(target_arch = "wasm32", feature = "embed")))]
use crate::compiler::compile;
// The colour constants and the cold-path hint are named through `crate::` by
// the parser, the compiler, and the macro scanner, which are descendants of
// this module.
#[cfg(feature = "compiler")]
use crate::errors::BOLD;
#[cfg(feature = "compiler")]
use crate::errors::ErrorCtx;
#[cfg(feature = "compiler")]
use crate::errors::RED;
#[cfg(feature = "compiler")]
use crate::errors::RESET;
use crate::vm::RegisterFile;
#[cfg(all(feature = "embed", feature = "compiler"))]
use std::ffi::{CStr, CString, c_char};
#[cfg(feature = "compiler")]
use std::hint::cold_path;
#[cfg(all(feature = "embed", feature = "compiler"))]
use std::panic::catch_unwind;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// The runtime core is the `candela-vm` crate. Alias its modules under `crate::`
// so the compiler/parser/REPL keep their existing paths, and re-export its
// public runtime API from this crate's surface.
pub(crate) use candela_vm::data;
pub(crate) use candela_vm::errors;
pub(crate) use candela_vm::instr;
pub(crate) use candela_vm::rt;
pub(crate) use candela_vm::vm;

// `pub` so an out-of-tree frontend (candela-lsp) can reuse the lexer, parser,
// and type-checker directly instead of reimplementing them. Gated behind the
// `compiler` feature.
#[cfg(feature = "compiler")]
#[path = "./compiler/compiler.rs"]
pub mod compiler;
// The embedding API (`Engine`/`Program`), built on top of the compiler and the
// `candela-vm` marshalling types.
#[cfg(feature = "compiler")]
mod engine;
// The `candela build` path: compile a `.cdl` source into a `.cdlb` artifact.
#[cfg(feature = "compiler")]
mod build;
// The command line: the verbs, and the project they read.
#[cfg(all(feature = "compiler", not(target_arch = "wasm32")))]
mod cli;
// Talking to `lpm`, the registry client that resolves dependencies.
#[cfg(all(feature = "compiler", not(target_arch = "wasm32")))]
mod lpm;
// `candela.toml`, the project manifest.
#[cfg(all(feature = "compiler", not(target_arch = "wasm32")))]
mod manifest;
// Macro registration and the region scanner. `pub` because both sides of a
// macro are the embedder's: it registers the expanders, and it can scan a file
// for regions without compiling it.
#[cfg(feature = "compiler")]
pub mod macros;
// `pub` for the same reason as `compiler`: exported so tooling can lex/parse
// standalone without going through a full `compiler::compile`.
#[cfg(feature = "compiler")]
#[path = "./parser/parser.rs"]
pub mod parser;
#[cfg(feature = "compiler")]
mod repl;
#[path = "./tests.rs"]
#[cfg(all(test, feature = "compiler"))]
mod tests;
// Call trampolines, shared by the `Engine`/`Program` embedding API and the
// export table `candela build` records in an artifact.
#[cfg(feature = "compiler")]
mod trampoline;
// Tells a person at a terminal when a newer release is out. It is only reached
// from the REPL and `--help`, never while a program is running.
#[cfg(feature = "compiler")]
mod update;
#[path = "./util/util.rs"]
mod util;

pub use candela_vm::Diagnostic;
pub use candela_vm::collect_diagnostic;

pub use candela_vm::FromHostValue;
pub use candela_vm::HostError;
pub use candela_vm::HostType;
pub use candela_vm::IntoHostFn;
pub use candela_vm::IntoHostResult;
pub use candela_vm::IntoHostValue;
pub use candela_vm::Value;
#[cfg(feature = "compiler")]
pub use engine::Engine;
#[cfg(feature = "compiler")]
pub use engine::Program;
// Where an import reads from: the standard library, and the root of every
// package a program depends on.
#[cfg(feature = "compiler")]
pub use compiler::imports::ImportResolver;

// The VM-only surface: load a pre-compiled `.cdlb`, run it, and call into it.
pub use candela_vm::CallError;
pub use candela_vm::HostBindError;
pub use candela_vm::HostRegistry;
pub use candela_vm::LoadError;
pub use candela_vm::RuntimeProgram;
pub use candela_vm::load_program;
// Where a `dylib` import looks for its library file. Read by a compile and by
// an artifact load alike, so a host that keeps its libraries in directories of
// its own sets them once.
pub use candela_vm::dylib_dirs;
pub use candela_vm::set_dylib_dirs;
// Compile a `.cdl` source string straight to `.cdlb` bytes (the `candela build`
// path). Needs the compiler.
#[cfg(feature = "compiler")]
pub use build::build_bytecode;

/// Runs a freshly compiled program's `main` to completion on the CLI/REPL path.
/// The embedding API (`Engine`/`Program`) drives the VM directly instead, with
/// the host-function tables the CLI never has.
#[cfg(feature = "compiler")]
pub(crate) fn execute_compiled(out: compiler::CompileOutput) {
    let compiler::CompileOutput {
        instructions,
        registers,
        mut pools,
        instr_src,
        fn_registers,
        dyn_lib_fns,
        structs,
        enums,
        allocated_arg_count,
        allocated_call_depth,
        sources,
        ..
    } = out;
    vm::execute(
        &instructions,
        &mut RegisterFile(registers),
        &mut pools,
        &ErrorCtx { instr_src, sources },
        &fn_registers,
        &dyn_lib_fns,
        &structs,
        &enums,
        allocated_arg_count,
        allocated_call_depth,
        &[],
        &[],
        0,
    );
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn get_output() -> String {
    candela_vm::captured_output::CAPTURED_OUTPUT.with(|o| o.take())
}

#[cfg(all(target_arch = "wasm32", feature = "compiler"))]
#[wasm_bindgen]
pub fn run(code: String) {
    candela_vm::captured_output::CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    execute_compiled(compile(
        code,
        "playground.cdl",
        false,
        &ImportResolver::new(),
    ));
}

#[cfg(all(feature = "embed", feature = "compiler"))]
#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)] // WIP
pub unsafe extern "C" fn candela_run(code: *const c_char) -> *mut c_char {
    std::panic::set_hook(Box::new(|_| {}));
    let code = unsafe { CStr::from_ptr(code) }
        .to_string_lossy()
        .to_string();
    candela_vm::captured_output::CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    // The caller gets the program's output and any error report back as the
    // returned string, so redirect both for the duration of the run.
    let was_capturing = candela_vm::captured_output::set_capturing(true);
    let _ = catch_unwind(|| {
        execute_compiled(compile(code, "embedded.cdl", false, &ImportResolver::new()));
    });
    candela_vm::captured_output::set_capturing(was_capturing);
    let output = candela_vm::captured_output::CAPTURED_OUTPUT.with(|o| o.take());
    CString::new(output).unwrap_or_default().into_raw()
}

#[cfg(all(feature = "embed", feature = "compiler"))]
#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)] // WIP
pub unsafe extern "C" fn candela_free_output(output: *mut c_char) {
    if !output.is_null() {
        #[allow(unused_must_use)]
        unsafe {
            CString::from_raw(output)
        };
    }
}

/// The `candela` command. The command line itself is read in [`cli`].
#[cfg(all(feature = "compiler", not(target_arch = "wasm32")))]
#[must_use]
pub fn main() -> std::process::ExitCode {
    #[cfg(not(debug_assertions))]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{RED}CANDELA ERROR{RESET}\n{info}");
    }));

    cli::main()
}
