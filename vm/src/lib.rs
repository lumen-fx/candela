//! `candela-vm` -- the self-contained runtime core for candela.
//!
//! This crate is everything needed to LOAD and RUN candela bytecode, and
//! nothing else: the register VM ([`vm::execute`]), the bytecode instruction set
//! ([`instr`]), the NaN-boxed value representation ([`data`]) and shared runtime
//! types ([`rt`]), the garbage collector, the host/script value marshalling
//! ([`embed`]), the runtime error/diagnostic machinery ([`errors`]), and the
//! `.cdlb` artifact format ([`artifact`]).
//!
//! It depends on NOTHING from the compiler (`candela`) crate -- the dependency
//! direction is strictly `candela -> candela-vm`. Both the `candela-vm` binary
//! and the full `candela` compiler binary link this one VM, so the executor is
//! never duplicated.

pub mod data;
pub mod errors;
pub mod instr;
pub mod rt;

#[path = "vm.rs"]
pub mod vm;

pub mod embed;

// json parse/stringify over the runtime value graph, backing `std::json`.
pub mod json;

// The `.cdlb` bytecode artifact format plus the load/run API.
pub mod artifact;

// GC helpers, referenced as `crate::{array_gc,map_gc,string_gc}` by the VM and
// value modules. Internal to the runtime.
mod array_gc;
mod map_gc;
mod string_gc;

// Routes runtime output through a captured sink for wasm / embedding hosts.
#[cfg(any(target_arch = "wasm32", feature = "embed"))]
pub mod captured_output;

// ---- public runtime API ----
pub use errors::Diagnostic;
pub use errors::ErrorCtx;
pub use errors::collect_diagnostic;

pub use embed::FromHostValue;
pub use embed::HostType;
pub use embed::IntoHostFn;
pub use embed::IntoHostValue;
pub use embed::Value;
pub use embed::marshal_value;
pub use embed::unmarshal_value;

pub use artifact::LoadError;
pub use artifact::RuntimeProgram;
pub use artifact::load_program;
