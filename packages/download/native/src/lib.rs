//! The native half of the `download` package.
//!
//! The package's candela code calls three exports through its `dylib` block:
//!
//! - `candela_download_start(source, url, path, checksum, timeout_ms,
//!   max_bytes)` streams `url` to `path` on a thread of its own and reports
//!   under `source`;
//! - `candela_download_cancel(source)` stops the transfer running under
//!   `source`, answering 1 when there was one and 0 when it had already ended;
//! - `candela_download_set_max_concurrent(n)` caps how many transfers run at
//!   once.
//!
//! Each event is one JSON object: `{"kind": "progress", "received": ...,
//! "total": ...}` (`total` is -1 when the server declared no size), then
//! `{"kind": "done", "path": ...}` or `{"kind": "failed", "message": ...}`,
//! which is always the last, after which the source is released. A cancelled
//! transfer fails with the message `cancelled`.

pub mod testkit;
pub mod transfer;
mod wait;

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use transfer::{CANCELLED, Control, Limits};

candela_package_sys::candela_dylib_init!();

/// How many transfers run at once until the program says otherwise.
pub const DEFAULT_MAX_CONCURRENT: usize = 4;

/// The most transfers a program can ask to run at once. Past this the
/// connections compete for the same link rather than finishing sooner.
pub const MAX_MAX_CONCURRENT: usize = 64;

/// The shortest wall time between two progress events for one transfer. A
/// fast link delivers thousands of chunks a second, and a handler run for each
/// of them would be the download's bottleneck.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

static MAX_CONCURRENT: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_CONCURRENT);

/// The transfers in flight, by source, each with the switch that cancels it.
fn live() -> &'static Mutex<HashMap<i64, Control>> {
    static LIVE: OnceLock<Mutex<HashMap<i64, Control>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn progress(source: i64, received: u64, total: Option<u64>) {
    let total = total.and_then(|t| i64::try_from(t).ok()).unwrap_or(-1);
    let received = i64::try_from(received).unwrap_or(i64::MAX);
    let text = serde_json::json!({"kind": "progress", "received": received, "total": total});
    candela_package_sys::post(source, &text.to_string());
}

/// Reports the last event of a transfer and lets the source go.
fn finish(source: i64, last: &serde_json::Value) {
    candela_package_sys::post(source, &last.to_string());
    candela_package_sys::release(source);
}

fn failed(source: i64, message: &str) {
    finish(
        source,
        &serde_json::json!({"kind": "failed", "message": message}),
    );
}

/// Starts a transfer of `url` to `path` reporting under `source`. A timeout or
/// a size of 0 or less means none.
///
/// # Safety
///
/// The string pointers are NUL-terminated strings the candela runtime passes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_download_start(
    source: i64,
    url: *const c_char,
    path: *const c_char,
    checksum: *const c_char,
    timeout_ms: i64,
    max_bytes: i64,
) {
    candela_package_sys::hold(source);
    let url = unsafe { candela_package_sys::arg(url) };
    let path = unsafe { candela_package_sys::arg(path) };
    let checksum = unsafe { candela_package_sys::arg(checksum) };
    if url.trim().is_empty() {
        return failed(source, "to_file was given no url");
    }
    if path.trim().is_empty() {
        return failed(source, "to_file was given no destination path");
    }
    let checksum = match transfer::parse_checksum(&checksum) {
        Ok(c) => c,
        Err(message) => return failed(source, &message),
    };
    let limits = Limits {
        timeout_ms: u64::try_from(timeout_ms).ok().filter(|v| *v > 0),
        max_bytes: u64::try_from(max_bytes).ok().filter(|v| *v > 0),
    };
    let control = Control::default();
    {
        let Ok(mut live) = live().lock() else {
            return failed(source, "the transfer table is unusable");
        };
        let cap = MAX_CONCURRENT.load(Ordering::Relaxed);
        if live.len() >= cap {
            drop(live);
            return failed(
                source,
                &format!(
                    "{cap} downloads are already running, which is the limit; raise it with \
                     download::set_max_concurrent or wait for one to finish"
                ),
            );
        }
        live.insert(source, control.clone());
    }
    let dest = candela_package_sys::resolve(&path);
    let run = {
        let control = control.clone();
        move || {
            // Throttled: a fast link delivers chunks far faster than a
            // program can read them, and every event costs a handler call.
            let mut last = Instant::now();
            let mut report = |received, total: Option<u64>| {
                if last.elapsed() >= PROGRESS_INTERVAL && !control.is_cancelled() {
                    last = Instant::now();
                    progress(source, received, total);
                }
            };
            let outcome = transfer::to_file(&url, &dest, &checksum, &limits, &control, &mut report);
            // A cancel that got in first has reported already; this transfer
            // says nothing more. Otherwise its slot is freed before the outcome
            // goes out, so a handler can start the next transfer.
            if !control.finish() {
                return;
            }
            if let Ok(mut live) = live().lock() {
                live.remove(&source);
            }
            match outcome {
                Ok(done) => {
                    // One last figure, unthrottled, so a progress bar reaches
                    // its end before the done handler runs.
                    progress(source, done.received, done.total);
                    finish(
                        source,
                        &serde_json::json!({
                            "kind": "done",
                            "path": done.path.display().to_string(),
                        }),
                    );
                }
                Err(message) => failed(source, &message),
            }
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name(format!("candela-download-{source}"))
        .spawn(run)
        && control.finish()
    {
        if let Ok(mut live) = live().lock() {
            live.remove(&source);
        }
        failed(source, &format!("no thread to download on: {e}"));
    }
}

/// Cancels the transfer running under `source`. Answers 1 when it was still
/// running, and its failure, with the message `cancelled`, is then the last
/// event; 0 when it had already ended, whose own outcome is on its way.
#[unsafe(no_mangle)]
pub extern "C" fn candela_download_cancel(source: i64) -> i64 {
    let Ok(mut live) = live().lock() else {
        return 0;
    };
    let Some(control) = live.get(&source) else {
        return 0;
    };
    if !control.cancel() {
        return 0;
    }
    live.remove(&source);
    drop(live);
    failed(source, CANCELLED);
    1
}

/// Caps how many transfers run at once, between 1 and
/// [`MAX_MAX_CONCURRENT`]. A start past the cap fails at once.
#[unsafe(no_mangle)]
pub extern "C" fn candela_download_set_max_concurrent(n: i64) {
    let n = usize::try_from(n.max(1))
        .unwrap_or(1)
        .min(MAX_MAX_CONCURRENT);
    MAX_CONCURRENT.store(n, Ordering::Relaxed);
}
