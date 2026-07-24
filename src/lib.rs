use crate::compiler::compile;
use crate::errors::BOLD;
use crate::errors::ErrorCtx;
use crate::errors::RED;
use crate::errors::RESET;
use crate::repl::repl;
use crate::vm::RegisterFile;
#[cfg(feature = "embed")]
use std::ffi::{CStr, CString, c_char};
use std::fs;
use std::hint::cold_path;
#[cfg(feature = "embed")]
use std::panic::catch_unwind;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[path = "./vm/gc/array_gc.rs"]
mod array_gc;
#[cfg(any(target_arch = "wasm32", feature = "embed"))]
mod captured_output;
#[path = "./compiler/compiler.rs"]
mod compiler;
#[path = "./data.rs"]
mod data;
mod embed;
#[path = "./util/errors.rs"]
mod errors;
#[path = "./instr.rs"]
mod instr;
#[path = "./vm/gc/map_gc.rs"]
mod map_gc;
#[path = "./parser/parser.rs"]
mod parser;
mod repl;
#[path = "./vm/gc/string_gc.rs"]
mod string_gc;
#[path = "./tests.rs"]
#[cfg(test)]
mod tests;
#[path = "./util/util.rs"]
mod util;
#[path = "./vm/vm.rs"]
mod vm;

pub use errors::Diagnostic;
pub use errors::collect_diagnostic;

pub use embed::Engine;
pub use embed::FromHostValue;
pub use embed::HostType;
pub use embed::IntoHostFn;
pub use embed::IntoHostValue;
pub use embed::Program;
pub use embed::Value;

/// Runs a freshly compiled program's `main` to completion on the CLI/REPL path.
/// The embedding API (`Engine`/`Program`) drives the VM directly instead, with
/// the host-function tables the CLI never has.
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

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn run(code: String) {
    captured_output::CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    execute_compiled(compile(code, "playground.kl", false));
}

#[cfg(feature = "embed")]
#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)] // WIP
pub unsafe extern "C" fn keel_run(code: *const c_char) -> *mut c_char {
    std::panic::set_hook(Box::new(|_| {}));
    let code = unsafe { CStr::from_ptr(code) }
        .to_string_lossy()
        .to_string();
    captured_output::CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    let _ = catch_unwind(|| {
        execute_compiled(compile(code, "embedded.kl", false));
    });
    let output = captured_output::CAPTURED_OUTPUT.with(|o| o.take());
    CString::new(output).unwrap_or_default().into_raw()
}

#[cfg(feature = "embed")]
#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)] // WIP
pub unsafe extern "C" fn keel_free_output(output: *mut c_char) {
    if !output.is_null() {
        #[allow(unused_must_use)]
        unsafe {
            CString::from_raw(output)
        };
    }
}

pub fn main() {
    #[cfg(not(debug_assertions))]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{RED}KEEL ERROR{RESET}\n{info}");
    }));

    let mut args = std::env::args().skip(1);

    if args.len() == 0 {
        cold_path();
        repl();
        return;
    }

    let next_arg = unsafe { args.next().unwrap_unchecked() };

    if next_arg == "--help" || next_arg == "-h" {
        cold_path();
        println!(
            "{}\nKeel is a fast, statically-typed interpreted language that aims to combine Rust-like syntax with Python's ease-of-use.\n\nUsage:\n  keel myfile.kl\n  keel [-v | --version]",
            util::KEEL_LOGO
        );
        return;
    }

    if next_arg == "--version" || next_arg == "-v" {
        cold_path();
        if args.len() > 1 {
            eprintln!(
                "{RED}KEEL ERROR{RESET}\nInvalid arguments\nUsage:\n  keel myfile.kl\n  keel [-v | --version]"
            );
            return;
        }
        println!("Keel {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let filename = &next_arg;

    let contents = fs::read_to_string(filename).unwrap_or_else(|_| {
        cold_path();
        eprintln!(
            "--------------\n{RED}KEEL RUNTIME ERROR:{RESET}\nCannot read {RED}{BOLD}{filename}{RESET}\n--------------",
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
            compile(contents, filename, false);
            return;
        }
    }

    execute_compiled(compile(contents, filename, false));
}
