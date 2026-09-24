//! `candela-vm` is the self-contained runtime core for candela.
//!
//! This crate is everything needed to load and run candela bytecode, and
//! nothing else: the register VM ([`vm::execute`]), the bytecode instruction set
//! ([`instr`]), the NaN-boxed value representation ([`data`]) and shared runtime
//! types ([`rt`]), the garbage collector, the host/script value marshalling
//! ([`embed`]), the runtime error/diagnostic machinery ([`errors`]), and the
//! `.cdlb` artifact format ([`artifact`]).
//!
//! It depends on nothing from the compiler (`candela`) crate. Both the
//! `candela-vm` binary and the full `candela` compiler binary link this one VM,
//! so the executor is never duplicated.

pub mod data;
pub mod errors;
pub mod instr;
pub mod rt;

#[path = "vm.rs"]
pub mod vm;

pub mod embed;

// json parse/stringify over the runtime value graph, backing `std/json`.
pub mod json;

// The standard library's native functions, built in where no dynamic library
// can be loaded.
#[cfg(any(target_arch = "wasm32", test))]
pub mod intrinsics;

// The `.cdlb` bytecode artifact format plus the load/run API.
pub mod artifact;

// The garbage collector. Internal to the runtime; its state is reached through
// `rt::Pools`.
mod gc;

// Routes program output and error reports to the process streams, or into a
// buffer that an embedding host reads back.
pub mod captured_output;

// ---- public runtime API ----
pub use errors::Diagnostic;
pub use errors::ErrorCtx;
pub use errors::RuntimeError;
pub use errors::collect_diagnostic;

pub use embed::FromHostValue;
pub use embed::HostBindError;
pub use embed::HostError;
pub use embed::HostRegistry;
pub use embed::HostType;
pub use embed::IntoHostFn;
pub use embed::IntoHostResult;
pub use embed::IntoHostValue;
pub use embed::Value;
pub use embed::marshal_args;
pub use embed::marshal_value;
pub use embed::unmarshal_value;

pub use artifact::CallError;
pub use artifact::LoadError;
pub use artifact::RuntimeProgram;
pub use artifact::load_program;

// What the collector has done, for a host that collects between frames.
pub use gc::GcStats;
pub use gc::PoolStats;

// Where a `dylib` import looks for its library file.
pub use rt::dylib_dirs;
pub use rt::set_dylib_dirs;
// How many leading command-line arguments `argv()` steps over.
pub use rt::argv_skip;
pub use rt::set_argv_skip;
