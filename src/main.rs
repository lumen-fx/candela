// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
#[cfg(all(not(target_arch = "wasm32"), feature = "compiler"))]
use mimalloc::MiMalloc;

#[cfg(all(not(target_arch = "wasm32"), feature = "compiler"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Live long and prosper
#[cfg(all(feature = "compiler", not(target_arch = "wasm32")))]
fn main() -> std::process::ExitCode {
    candela::main()
}

/// The full `candela` binary (compiler + VM) is the compiler front-end; it is
/// meaningless without the `compiler` feature. (The VM-only runtime is shipped
/// as the separate `candela-vm` binary.)
#[cfg(not(feature = "compiler"))]
fn main() {
    eprintln!("this candela binary was built without the `compiler` feature");
    std::process::exit(1);
}

/// The wasm build is the library the playground calls into. There is no command
/// line to read there: no files, no project, no processes to run `lpm` in.
#[cfg(all(feature = "compiler", target_arch = "wasm32"))]
fn main() {
    eprintln!("the wasm build of candela has no command line; call run() from the page");
}
