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

#[path = "./vm/gc/array_gc.rs"]
mod array_gc;
// Bytecode artifact format (`.cdlb`) + the lean VM-only load/run API. Always
// compiled -- this is what `candela-vm` links against.
mod artifact;
#[cfg(any(target_arch = "wasm32", feature = "embed"))]
mod captured_output;
// `pub` so an out-of-tree frontend (candela-lsp) can reuse the lexer, parser,
// and type-checker directly instead of reimplementing them. Gated behind the
// `compiler` feature: with `--no-default-features` the crate is the runtime
// core only (no parser/compiler/repl), which is how `candela-vm` is built.
#[cfg(feature = "compiler")]
#[path = "./compiler/compiler.rs"]
pub mod compiler;
#[path = "./data.rs"]
mod data;
mod embed;
#[path = "./util/errors.rs"]
mod errors;
#[path = "./instr.rs"]
mod instr;
#[path = "./vm/gc/map_gc.rs"]
mod map_gc;
// `pub` for the same reason as `compiler` above: exported so tooling (or a
// future incremental parse-only path) can lex/parse standalone without going
// through a full `compiler::compile`.
#[cfg(feature = "compiler")]
#[path = "./parser/parser.rs"]
pub mod parser;
#[cfg(feature = "compiler")]
mod repl;
// Runtime data types shared by the VM and the compiler. Always compiled.
mod rt;
#[path = "./vm/gc/string_gc.rs"]
mod string_gc;
#[path = "./tests.rs"]
#[cfg(all(test, feature = "compiler"))]
mod tests;
#[path = "./util/util.rs"]
mod util;
#[path = "./vm/vm.rs"]
mod vm;

pub use errors::Diagnostic;
pub use errors::collect_diagnostic;

#[cfg(feature = "compiler")]
pub use embed::Engine;
pub use embed::FromHostValue;
pub use embed::HostType;
pub use embed::IntoHostFn;
pub use embed::IntoHostValue;
#[cfg(feature = "compiler")]
pub use embed::Program;
pub use embed::Value;

// The lean VM-only surface: load a pre-compiled `.cdlb` and run it. Available
// with and without the `compiler` feature -- this is the API `candela-vm` uses.
pub use artifact::LoadError;
pub use artifact::RuntimeProgram;
pub use artifact::load_program;
// Compile a `.cdl` source string straight to `.cdlb` bytes (the `candela build`
// path). Needs the compiler.
#[cfg(feature = "compiler")]
pub use artifact::build_bytecode;

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
    captured_output::CAPTURED_OUTPUT.with(|o| o.take())
}

#[cfg(all(target_arch = "wasm32", feature = "compiler"))]
#[wasm_bindgen]
pub fn run(code: String) {
    captured_output::CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
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
    captured_output::CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    let _ = catch_unwind(|| {
        execute_compiled(compile(code, "embedded.cdl", false));
    });
    let output = captured_output::CAPTURED_OUTPUT.with(|o| o.take());
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
/// lean `candela-vm` binary, which links no parser/compiler/REPL.
#[cfg(feature = "compiler")]
fn build_subcommand(args: &mut impl Iterator<Item = String>) {
    let Some(input) = args.next() else {
        eprintln!(
            "{RED}CANDELA ERROR{RESET}\nUsage:\n  candela build <file.cdl> [-o out.cdlb]"
        );
        std::process::exit(1);
    };

    let mut output: Option<String> = None;
    while let Some(a) = args.next() {
        if a == "-o" || a == "--output" {
            output = args.next();
        } else {
            output = Some(a);
        }
    }
    let output = output.unwrap_or_else(|| {
        let stripped = input.strip_suffix(".cdl").unwrap_or(&input);
        format!("{stripped}.cdlb")
    });

    let contents = fs::read_to_string(&input).unwrap_or_else(|_| {
        cold_path();
        eprintln!(
            "--------------\n{RED}CANDELA RUNTIME ERROR:{RESET}\nCannot read {RED}{BOLD}{input}{RESET}\n--------------",
        );
        std::process::exit(1);
    });

    let bytes = match artifact::build_bytecode(contents, &input) {
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
        println!(
            "{}\nCandela is a fast, statically-typed interpreted language that aims to combine Rust-like syntax with Python's ease-of-use.\n\nUsage:\n  candela myfile.cdl\n  candela build <file.cdl> [-o out.cdlb]   (compile to bytecode; run with candela-vm)\n  candela [-v | --version]",
            util::CANDELA_LOGO
        );
        return;
    }

    if next_arg == "--version" || next_arg == "-v" {
        cold_path();
        if args.len() > 1 {
            eprintln!(
                "{RED}CANDELA ERROR{RESET}\nInvalid arguments\nUsage:\n  candela myfile.cdl\n  candela [-v | --version]"
            );
            return;
        }
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
