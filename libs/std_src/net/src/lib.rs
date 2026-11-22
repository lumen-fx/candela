//! The native library behind `std/http` and `std/websocket`.
//!
//! One library carries both, so a program that imports either loads a single
//! TLS stack: rustls with the Mozilla root set. Each module binds its own
//! exports, `candela_http_*` and `candela_websocket_*`; the runtime hands the
//! library its event table once per load, and both halves post through it.

pub mod http;
pub mod websocket;

candela_std_sys::candela_dylib_init!();
