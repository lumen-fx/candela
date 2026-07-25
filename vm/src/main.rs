//! `candela-vm` -- the lean VM-only runtime.
//!
//! Loads a pre-compiled `.cdlb` bytecode artifact (produced by
//! `candela build <file.cdl>`) and runs it. It links no parser, compiler, or
//! REPL -- only the candela runtime core.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);

    let Some(path) = args.next() else {
        eprintln!("Usage: candela-vm <file.cdlb>");
        return ExitCode::from(2);
    };

    if path == "--help" || path == "-h" {
        println!(
            "candela-vm -- runs pre-compiled candela bytecode (.cdlb)\n\nUsage:\n  candela-vm <file.cdlb>\n\nProduce a .cdlb with the candela compiler:\n  candela build <file.cdl> [-o out.cdlb]"
        );
        return ExitCode::SUCCESS;
    }
    if path == "--version" || path == "-v" {
        println!("candela-vm {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("candela-vm: cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };

    let mut program = match candela::load_program(&bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("candela-vm: {e}");
            return ExitCode::from(1);
        }
    };

    // On a runtime error the VM prints a diagnostic and exits the process
    // itself (matching the full `candela <file.cdl>` path); on success `run`
    // returns and we report success.
    program.run();
    ExitCode::SUCCESS
}
