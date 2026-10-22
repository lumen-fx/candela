//! The native half of the `websocket` package.
//!
//! The package's candela code calls four exports through its `dylib` block,
//! and the same four names through its `host` block in a browser, where the
//! JavaScript module in `web/` answers them:
//!
//! - `candela_websocket_open(source, url)` connects on a thread of its own
//!   and reports under `source`;
//! - `candela_websocket_send(source, text)` queues a text frame, answering 1
//!   when the connection is open and 0 when it is not;
//! - `candela_websocket_close(source, code, reason)` starts the closing
//!   handshake, answering 1 when there was a connection to close, 0 when
//!   there was none, and -1 for a code or reason the protocol does not allow;
//! - `candela_websocket_state(source)` answers `connecting`, `open`,
//!   `closing` or `closed`.
//!
//! Each event is one JSON object: `{"kind": "open"}`, `{"kind": "message",
//! "text": ...}`, `{"kind": "error", "message": ...}`, and `{"kind": "close",
//! "code": ..., "reason": ..., "clean": ...}`, which is always the last, after
//! which the source is released.

pub mod connection;

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::{Arc, Mutex, OnceLock};

use connection::{Connection, Outgoing, State};

candela_package_sys::candela_dylib_init!();

/// Every connection still running, by source.
fn live() -> &'static Mutex<HashMap<i64, Connection>> {
    static LIVE: OnceLock<Mutex<HashMap<i64, Connection>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with<T>(source: i64, f: impl FnOnce(Option<&Connection>) -> T) -> T {
    let live = live()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(live.get(&source))
}

/// Reports a connection that never started: an error, then the close a
/// browser reports for one, and the source goes.
fn never_opened(source: i64, message: &str) {
    candela_package_sys::post(
        source,
        &serde_json::json!({"kind": "error", "message": message}).to_string(),
    );
    candela_package_sys::post(
        source,
        &serde_json::json!({"kind": "close", "code": 1006, "reason": "", "clean": false})
            .to_string(),
    );
    candela_package_sys::release(source);
}

/// Connects to `url`, reporting under `source`.
///
/// # Safety
///
/// `url` is a NUL-terminated string the candela runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_websocket_open(source: i64, url: *const c_char) {
    candela_package_sys::hold(source);
    let url = match connection::socket_url(&unsafe { candela_package_sys::arg(url) }) {
        Ok(url) => url,
        Err(message) => return never_opened(source, &message),
    };
    let mut connections = live()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let report: connection::Report = Arc::new(move |payload| match payload {
        Some(payload) => candela_package_sys::post(source, &payload.to_string()),
        None => {
            if let Ok(mut live) = live().lock() {
                live.remove(&source);
            }
            candela_package_sys::release(source);
        }
    });
    match connection::start(url, report) {
        Ok(connection) => {
            connections.insert(source, connection);
        }
        Err(message) => {
            drop(connections);
            never_opened(source, &message);
        }
    }
}

/// Queues a text frame on the connection under `source`.
///
/// # Safety
///
/// `text` is a NUL-terminated string the candela runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_websocket_send(source: i64, text: *const c_char) -> i64 {
    let text = unsafe { candela_package_sys::arg(text) };
    i64::from(with(source, |c| {
        c.is_some_and(|c| c.state() == State::Open && c.push(Outgoing::Text(text)))
    }))
}

/// Starts closing the connection under `source` with `code`, 1000 or 3000 to
/// 4999, and a reason of at most 123 bytes.
///
/// # Safety
///
/// `reason` is a NUL-terminated string the candela runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_websocket_close(
    source: i64,
    code: i64,
    reason: *const c_char,
) -> i64 {
    let reason = unsafe { candela_package_sys::arg(reason) };
    let Some(code) = u16::try_from(code)
        .ok()
        .filter(|code| *code == 1000 || (3000..=4999).contains(code))
    else {
        return -1;
    };
    if reason.len() > 123 {
        return -1;
    }
    i64::from(with(source, |c| {
        c.is_some_and(|c| {
            matches!(c.state(), State::Connecting | State::Open) && {
                c.set_state(State::Closing);
                c.push(Outgoing::Close(code, reason))
            }
        })
    }))
}

/// What the connection under `source` is doing; `closed` when there is none.
#[unsafe(no_mangle)]
pub extern "C" fn candela_websocket_state(source: i64) -> *const c_char {
    candela_package_sys::ret(with(source, |c| {
        c.map_or(State::Closed, Connection::state).name()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_with_no_connection_is_closed_and_takes_nothing() {
        let state = unsafe { candela_package_sys::arg(candela_websocket_state(-5)) };
        assert_eq!(state, "closed");
        assert_eq!(unsafe { candela_websocket_send(-5, c"hi".as_ptr()) }, 0);
        assert_eq!(
            unsafe { candela_websocket_close(-5, 1000, c"".as_ptr()) },
            0
        );
    }

    #[test]
    fn a_close_code_the_protocol_reserves_is_refused() {
        for code in [0, 1001, 1006, 2999, 5000] {
            assert_eq!(
                unsafe { candela_websocket_close(-5, code, c"".as_ptr()) },
                -1,
                "{code}"
            );
        }
        let long = std::ffi::CString::new("x".repeat(124)).unwrap();
        assert_eq!(
            unsafe { candela_websocket_close(-5, 1000, long.as_ptr()) },
            -1
        );
    }
}
