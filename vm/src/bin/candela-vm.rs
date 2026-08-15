//! `candela-vm` is the lean VM-only runtime.
//!
//! Loads a pre-compiled `.cdlb` bytecode artifact (produced by
//! `candela build <file.cdl>`) and runs it. It links no parser, compiler, or
//! REPL, only the candela runtime core.
//!
//! Arguments after the artifact path are the program's, reachable from
//! `argv()`, the same as arguments after the file name of a source run.

use candela_vm::HostBindError;
use candela_vm::HostRegistry;
use candela_vm::LoadError;
use std::process::ExitCode;

const USAGE: &str = "Usage:\n  candela-vm <file.cdlb> [arguments...]";

fn main() -> ExitCode {
    // The artifact path comes first, and everything after it belongs to the
    // program, which reads it with `argv()`. So an option is only ever an
    // option in the first position; past that the program is being addressed,
    // and a program takes the same arguments whether it runs from source or
    // from an artifact.
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    match path.as_str() {
        "--help" | "-h" => {
            println!(
                "candela-vm runs pre-compiled candela bytecode (.cdlb)\n\n{USAGE}\n\nArguments after the artifact reach the program through argv().\n\nProduce a .cdlb with the candela compiler:\n  candela build <file.cdl> [-o out.cdlb]"
            );
            return ExitCode::SUCCESS;
        }
        "--version" | "-v" => {
            println!("candela-vm {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        // An unknown option is a mistake worth naming. Reading it as a path
        // reports it as a file that is not there, which sends you looking for
        // the wrong thing.
        other if other.starts_with('-') => {
            eprintln!("candela-vm: unknown option {other}\n{USAGE}");
            return ExitCode::from(2);
        }
        _ => {}
    }

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("candela-vm: cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };

    // The standalone runtime is nobody's embedder, so it registers no host
    // functions: an artifact that declares a `host` block is one you run from
    // the program that supplies it.
    let hosts = HostRegistry::new();
    let mut program = match candela_vm::load_program(&bytes, &hosts) {
        Ok(p) => p,
        Err(LoadError::HostBinding(error @ HostBindError::Unregistered(_))) => {
            eprintln!(
                "candela-vm: {error}\nHost functions come from the program that embeds candela; run this artifact from there."
            );
            return ExitCode::from(1);
        }
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
