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

#[cfg(feature = "compiler")]
use crate::compiler::compile;
#[cfg(feature = "compiler")]
use crate::errors::BOLD;
#[cfg(feature = "compiler")]
use crate::errors::ErrorCtx;
#[cfg(feature = "compiler")]
use crate::errors::RED;
#[cfg(feature = "compiler")]
use crate::errors::RESET;
#[cfg(feature = "compiler")]
use crate::repl::repl;
use crate::vm::RegisterFile;
#[cfg(all(feature = "embed", feature = "compiler"))]
use std::ffi::{CStr, CString, c_char};
use std::fs;
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
pub use candela_vm::HostType;
pub use candela_vm::IntoHostFn;
pub use candela_vm::IntoHostValue;
pub use candela_vm::Value;
#[cfg(feature = "compiler")]
pub use engine::Engine;
#[cfg(feature = "compiler")]
pub use engine::Program;

// The VM-only surface: load a pre-compiled `.cdlb`, run it, and call into it.
pub use candela_vm::CallError;
pub use candela_vm::HostBindError;
pub use candela_vm::HostRegistry;
pub use candela_vm::LoadError;
pub use candela_vm::RuntimeProgram;
pub use candela_vm::load_program;
// Compile a `.cdl` source string straight to `.cdlb` bytes (the `candela build`
// path). Needs the compiler.
#[cfg(feature = "compiler")]
pub use build::build_bytecode;

/// Runs a freshly compiled program's `main` to completion on the CLI/REPL path.
/// The embedding API (`Engine`/`Program`) drives the VM directly instead, with
/// the host-function tables the CLI never has.
#[cfg(feature = "compiler")]
fn execute_compiled(out: compiler::CompileOutput) {
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
    execute_compiled(compile(code, "playground.cdl", false));
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
        execute_compiled(compile(code, "embedded.cdl", false));
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

/// Compiles a `.cdl` source file to a `.cdlb` bytecode artifact.
///
/// `candela build <file.cdl> [-o out.cdlb]`. The emitted artifact is run by the
/// VM-only `candela-vm` binary, which links no parser/compiler/REPL.
#[cfg(feature = "compiler")]
fn build_subcommand(args: &mut impl Iterator<Item = String>) {
    let Some(input) = args.next() else {
        eprintln!("{RED}CANDELA ERROR{RESET}\nUsage:\n  candela build <file.cdl> [-o out.cdlb]");
        std::process::exit(1);
    };

    // The output path is only ever named by `-o`/`--output`. A second bare
    // path is rejected instead of taken as the output, so a mistyped
    // `candela build a.cdl b.cdlb` says so rather than quietly writing
    // `b.cdlb`.
    let mut output: Option<String> = None;
    while let Some(a) = args.next() {
        if a == "-o" || a == "--output" {
            let Some(path) = args.next() else {
                eprintln!(
                    "{RED}CANDELA ERROR{RESET}\n{a} needs an output path\nUsage:\n  candela build <file.cdl> [-o out.cdlb]"
                );
                std::process::exit(1);
            };
            output = Some(path);
        } else {
            eprintln!(
                "{RED}CANDELA ERROR{RESET}\nUnexpected argument {RED}{BOLD}{a}{RESET}\nName the output file with -o or --output\nUsage:\n  candela build <file.cdl> [-o out.cdlb]"
            );
            std::process::exit(1);
        }
    }
    // A `-o`/`--output` argument is honored verbatim. Otherwise the default
    // output name replaces the `.cdl` extension with `.cdlb` (so
    // `program.cdl` -> `program.cdlb`); it never appends a second extension
    // (never `program.cdl.cdlb`). A path without a `.cdl` suffix just gets
    // `.cdlb` added.
    let output = output.unwrap_or_else(|| {
        let stem = input.strip_suffix(".cdl").unwrap_or(&input);
        format!("{stem}.cdlb")
    });

    let contents = fs::read_to_string(&input).unwrap_or_else(|_| {
        cold_path();
        eprintln!(
            "--------------\n{RED}CANDELA RUNTIME ERROR:{RESET}\nCannot read {RED}{BOLD}{input}{RESET}\n--------------",
        );
        std::process::exit(1);
    });

    let bytes = match build::build_bytecode(contents, &input) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{RED}CANDELA ERROR{RESET}\nCannot build bytecode: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = fs::write(&output, &bytes) {
        eprintln!("{RED}CANDELA ERROR{RESET}\nCannot write {output}: {e}");
        std::process::exit(1);
    }
    println!("Wrote {} ({} bytes)", output, bytes.len());
}

/// Rejects anything trailing `--help` or `--version`.
///
/// Both flags answer a question that takes no further input, so a trailing
/// argument is a mistake; saying so beats printing the answer to a question
/// nobody asked.
#[cfg(feature = "compiler")]
fn reject_extra_args(args: &mut impl Iterator<Item = String>, flag: &str) {
    if let Some(extra) = args.next() {
        cold_path();
        eprintln!(
            "{RED}CANDELA ERROR{RESET}\n{flag} takes no other arguments, got {RED}{BOLD}{extra}{RESET}\nUsage:\n  candela myfile.cdl\n  candela build <file.cdl> [-o out.cdlb]\n  candela [-h | --help]\n  candela [-v | --version]"
        );
        std::process::exit(1);
    }
}

#[cfg(feature = "compiler")]
pub fn main() {
    #[cfg(not(debug_assertions))]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{RED}CANDELA ERROR{RESET}\n{info}");
    }));

    let mut args = std::env::args().skip(1);

    if args.len() == 0 {
        cold_path();
        repl();
        return;
    }

    let next_arg = unsafe { args.next().unwrap_unchecked() };

    if next_arg == "build" || next_arg == "compile" {
        cold_path();
        build_subcommand(&mut args);
        return;
    }

    if next_arg == "--help" || next_arg == "-h" {
        cold_path();
        reject_extra_args(&mut args, &next_arg);
        let update = update::start();
        println!(
            "{}\nCandela is a fast, statically-typed interpreted language that aims to combine Rust-like syntax with Python's ease-of-use.\n\nUsage:\n  candela myfile.cdl\n  candela build <file.cdl> [-o out.cdlb]   (compile to bytecode; run with candela-vm)\n  candela [-v | --version]",
            util::CANDELA_LOGO
        );
        update::finish(update);
        return;
    }

    if next_arg == "--version" || next_arg == "-v" {
        cold_path();
        reject_extra_args(&mut args, &next_arg);
        println!("Candela {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let filename = &next_arg;

    let contents = fs::read_to_string(filename).unwrap_or_else(|_| {
        cold_path();
        eprintln!(
            "--------------\n{RED}CANDELA RUNTIME ERROR:{RESET}\nCannot read {RED}{BOLD}{filename}{RESET}\n--------------",
        );
        std::process::exit(1);
    });

    #[cfg(debug_assertions)]
    {
        let next = args.next();
        if next == Some(String::from("--debug")) {
            let now = std::time::Instant::now();
            let out = compile(contents, filename, true);
            println!("COMPILATION TIME: {:.2?}", now.elapsed());
            let now = std::time::Instant::now();
            execute_compiled(out);
            println!(
                "EXECUTION TIME: {:.3}ms",
                now.elapsed().as_nanos() / 1_000_000
            );
            return;
        } else if next == Some(String::from("--debug-parser")) {
            let _ = compile(contents, filename, false);
            return;
        }
    }

    execute_compiled(compile(contents, filename, false));
}
