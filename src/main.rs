#[cfg(all(not(target_arch = "wasm32"), feature = "compiler"))]
use mimalloc::MiMalloc;

#[cfg(all(not(target_arch = "wasm32"), feature = "compiler"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Live long and prosper
#[cfg(feature = "compiler")]
fn main() {
    candela::main();
}

/// The fat `candela` binary is the compiler front-end; it is meaningless
/// without the `compiler` feature. (The runtime core is shipped as the separate
/// `candela-vm` binary.)
#[cfg(not(feature = "compiler"))]
fn main() {
    eprintln!("this candela binary was built without the `compiler` feature");
    std::process::exit(1);
}
