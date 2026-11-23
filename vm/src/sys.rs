//! What `std/os` and `std/time` ask of the process: the environment, the
//! working directory, platform facts and clocks.
//!
//! The VM's instructions call these; each answers plain Rust values and the
//! instruction turns them into candela ones.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The fact `os_query(kind)` answers: the platform, the architecture, the
/// working directory, the home directory (empty when there is none) and the
/// temporary directory.
///
/// # Errors
///
/// The working directory can be gone or unreadable; that is the I/O error.
pub fn query(kind: i64) -> std::io::Result<String> {
    Ok(match kind {
        0 => match std::env::consts::OS {
            "linux" | "macos" | "windows" => std::env::consts::OS,
            _ if cfg!(target_arch = "wasm32") => "web",
            other => other,
        }
        .to_owned(),
        1 => std::env::consts::ARCH.to_owned(),
        2 => std::env::current_dir()?.display().to_string(),
        3 => std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default(),
        _ => std::env::temp_dir().display().to_string(),
    })
}

/// Every variable of this process's environment, as name, value, name,
/// value. A name or value that is not UTF-8 is left out.
#[must_use]
pub fn env_vars() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(k, v)| Some([k.into_string().ok()?, v.into_string().ok()?]))
        .flatten()
        .collect()
}

/// Sets `name` to `value` in this process's environment.
pub fn set_env(name: &str, value: &str) {
    if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
        return;
    }
    // SAFETY: the program runs on one thread, and the native libraries that
    // run threads of their own read no environment while a program could be
    // changing it: a child takes its environment when it starts, on the
    // program's thread.
    unsafe { std::env::set_var(name, value) };
}

/// Milliseconds since the epoch (`kind` 0), or nanoseconds on a clock that
/// only moves forward, counted from the first time it is read (`kind` 1).
#[must_use]
pub fn clock(kind: i64) -> i64 {
    if kind == 0 {
        return SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
    }
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as i64
}
