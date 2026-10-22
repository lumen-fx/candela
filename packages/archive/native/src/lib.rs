//! The native half of the `archive` package.
//!
//! The package's candela code calls two exports through its `dylib` block:
//!
//! - `candela_archive_extract(source, src, dest, include, count)` unpacks the
//!   archive at `src` into `dest` on a thread of its own, keeping only the
//!   files matching one of the `count` glob patterns at `include` (all of them
//!   when there are none), and reports under `source`;
//! - `candela_archive_set_max_concurrent(n)` caps how many extractions run at
//!   once.
//!
//! The one event is a JSON object, `{"kind": "done", "dest": ..., "files":
//! ...}` or `{"kind": "failed", "message": ...}`, after which the source is
//! released.

pub mod testkit;
pub mod unpack;

use std::collections::HashSet;
use std::ffi::c_char;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

candela_package_sys::candela_dylib_init!();

/// How many extractions run at once until the program says otherwise.
pub const DEFAULT_MAX_CONCURRENT: usize = 4;

/// The most a program can ask to run at once. Each extraction holds a thread
/// for as long as it reads.
pub const MAX_MAX_CONCURRENT: usize = 64;

static MAX_CONCURRENT: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_CONCURRENT);

/// The extractions in flight, by source.
fn live() -> &'static Mutex<HashSet<i64>> {
    static LIVE: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Reports the outcome of an extraction and lets the source go.
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

/// Unpacks `src` into `dest`, reporting under `source`.
///
/// # Safety
///
/// The string pointers are NUL-terminated strings the candela runtime passes,
/// and `include` points at `count` of them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_archive_extract(
    source: i64,
    src: *const c_char,
    dest: *const c_char,
    include: *const *const c_char,
    count: i64,
) {
    candela_package_sys::hold(source);
    let src = unsafe { candela_package_sys::arg(src) };
    let dest = unsafe { candela_package_sys::arg(dest) };
    let patterns: Vec<String> = (0..usize::try_from(count).unwrap_or(0))
        .map(|i| unsafe { candela_package_sys::arg(*include.add(i)) })
        .collect();
    let filter = (!patterns.is_empty()).then(|| unpack::Filter::new(patterns));
    {
        let Ok(mut live) = live().lock() else {
            return failed(source, "the extraction table is unusable");
        };
        let cap = MAX_CONCURRENT.load(Ordering::Relaxed);
        if live.len() >= cap {
            drop(live);
            return failed(
                source,
                &format!(
                    "{cap} extractions are already running, which is the limit; raise it with \
                     archive::set_max_concurrent or wait for one to finish"
                ),
            );
        }
        live.insert(source);
    }
    let src = candela_package_sys::resolve(&src);
    let dest = candela_package_sys::resolve(&dest);
    let work = move || {
        let outcome = unpack::extract(&src, &dest, filter.as_ref());
        if let Ok(mut live) = live().lock() {
            live.remove(&source);
        }
        match outcome {
            Ok(unpacked) => finish(
                source,
                &serde_json::json!({
                    "kind": "done",
                    "dest": dest.to_string_lossy(),
                    "files": unpacked.files,
                }),
            ),
            Err(message) => failed(source, &message),
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name(format!("candela-archive-{source}"))
        .spawn(work)
    {
        if let Ok(mut live) = live().lock() {
            live.remove(&source);
        }
        failed(source, &format!("no thread to extract on: {e}"));
    }
}

/// Caps how many extractions run at once, between 1 and
/// [`MAX_MAX_CONCURRENT`]. An extraction started past the cap fails at once.
#[unsafe(no_mangle)]
pub extern "C" fn candela_archive_set_max_concurrent(n: i64) {
    let n = usize::try_from(n.max(1))
        .unwrap_or(1)
        .min(MAX_MAX_CONCURRENT);
    MAX_CONCURRENT.store(n, Ordering::Relaxed);
}
