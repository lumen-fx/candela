//! The native half of the `process` package.
//!
//! The package's candela code calls two exports through its `dylib` block:
//!
//! - `candela_process_start(source, cmd, args, argc, cwd, env_json,
//!   end_at_exit)` starts `cmd` and reports everything after that as events
//!   under `source`;
//! - `candela_process_stop(source)` ends the child started under `source`,
//!   answering 1 when there was one still running and 0 when there was not.
//!
//! Each event is one JSON object: `{"kind": "stdout", "line": ...}`,
//! `{"kind": "stderr", "line": ...}`, `{"kind": "exit", "code": ...}`, or
//! `{"kind": "failed", "message": ...}` when the program did not start. The
//! exit, or the failure, is always the last event, and the source is released
//! after it.
//!
//! A child started with `end_at_exit` is ended when the process exits through
//! the C runtime's exit path, which is how `exit(code)` and the end of the
//! program leave.

pub mod child;
#[cfg(windows)]
mod job;

use child::{Event, Options, Running};

use std::collections::HashMap;
use std::ffi::c_char;
use std::sync::{Arc, Mutex, Once, OnceLock};

candela_package_sys::candela_dylib_init!();

/// A child that has not reported its exit yet, by the source it reports
/// under.
struct Entry {
    running: Running,
    end_at_exit: bool,
}

/// The children still running.
fn children() -> &'static Mutex<HashMap<i64, Entry>> {
    static CHILDREN: OnceLock<Mutex<HashMap<i64, Entry>>> = OnceLock::new();
    CHILDREN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The JSON text of one event.
fn payload(event: &Event) -> String {
    let value = match event {
        Event::Stdout(line) => serde_json::json!({"kind": "stdout", "line": line}),
        Event::Stderr(line) => serde_json::json!({"kind": "stderr", "line": line}),
        Event::Exit(code) => serde_json::json!({"kind": "exit", "code": code}),
    };
    value.to_string()
}

/// Posts a start that failed, and lets the source go.
fn fail(source: i64, message: &str) {
    let text = serde_json::json!({"kind": "failed", "message": message}).to_string();
    candela_package_sys::post(source, &text);
    candela_package_sys::release(source);
}

/// The variables an `env` object sets, in name order. A value that is not a
/// string is refused: an environment holds nothing else.
fn environment(json: &str) -> Result<Vec<(String, String)>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("env is not a JSON object: {e}"))?;
    let serde_json::Value::Object(map) = value else {
        return Err(String::from("env is not a JSON object"));
    };
    let mut env = Vec::with_capacity(map.len());
    for (name, value) in map {
        match value {
            serde_json::Value::String(text) => env.push((name, text)),
            _ => return Err(format!("the value of env `{name}` is not a string")),
        }
    }
    env.sort();
    Ok(env)
}

/// Ends the children started with `end_at_exit` as the process exits.
extern "C" fn end_children_at_exit() {
    let ending: Vec<Running> = children().lock().map_or_else(
        |_| Vec::new(),
        |live| {
            live.values()
                .filter(|entry| entry.end_at_exit)
                .map(|entry| entry.running.clone())
                .collect()
        },
    );
    child::stop_all(&ending);
}

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> i32;
}

/// Starts `cmd` with the `argc` strings at `args`, reporting under `source`.
///
/// # Safety
///
/// Every pointer is one the candela runtime passes: NUL-terminated strings,
/// and `args` pointing at `argc` of them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn candela_process_start(
    source: i64,
    cmd: *const c_char,
    args: *const *const c_char,
    argc: i64,
    cwd: *const c_char,
    env_json: *const c_char,
    end_at_exit: i64,
) {
    candela_package_sys::hold(source);
    let cmd = unsafe { candela_package_sys::arg(cmd) };
    let argv: Vec<String> = (0..usize::try_from(argc).unwrap_or(0))
        .map(|i| unsafe { candela_package_sys::arg(*args.add(i)) })
        .collect();
    let cwd = unsafe { candela_package_sys::arg(cwd) };
    let env = match environment(&unsafe { candela_package_sys::arg(env_json) }) {
        Ok(env) => env,
        Err(why) => return fail(source, &format!("start({cmd}): {why}")),
    };
    let options = Options {
        cwd: (!cwd.is_empty()).then_some(cwd),
        env,
    };
    let end_at_exit = end_at_exit != 0;
    if end_at_exit {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| unsafe {
            atexit(end_children_at_exit);
        });
    }

    // The record is made under the same lock the exit takes, so a child that
    // ends before this returns is still forgotten.
    let Ok(mut live) = children().lock() else {
        return fail(
            source,
            &format!("start({cmd}): the process table is unusable"),
        );
    };
    let emit: child::Emit = Arc::new(move |event| {
        let text = payload(&event);
        if matches!(event, Event::Exit(_))
            && let Ok(mut live) = children().lock()
        {
            live.remove(&source);
        }
        candela_package_sys::post(source, &text);
        if matches!(event, Event::Exit(_)) {
            candela_package_sys::release(source);
        }
    });
    match child::start(&cmd, &argv, &source.to_string(), &options, emit) {
        Ok(running) => {
            live.insert(
                source,
                Entry {
                    running,
                    end_at_exit,
                },
            );
        }
        Err(message) => {
            drop(live);
            fail(source, &message);
        }
    }
}

/// Ends the child running under `source`: `SIGTERM` to its process group on
/// Unix, then `SIGKILL` after a grace period; the end of its job on Windows.
/// Answers 1 when there was one, 0 when it had already ended or never
/// started. Its exit still arrives as the last event.
#[unsafe(no_mangle)]
pub extern "C" fn candela_process_stop(source: i64) -> i64 {
    let running = children()
        .lock()
        .ok()
        .and_then(|live| live.get(&source).map(|entry| entry.running.clone()));
    i64::from(running.is_some_and(|running| running.stop()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_env_object_is_read_in_name_order() {
        assert_eq!(
            environment(r#"{"B": "2", "A": "1"}"#).unwrap(),
            vec![("A".into(), "1".into()), ("B".into(), "2".into())]
        );
        assert_eq!(environment("{}").unwrap(), vec![]);
        assert_eq!(environment("").unwrap(), vec![]);
        assert!(environment(r#"{"A": 1}"#).is_err());
        assert!(environment("[]").is_err());
    }

    #[test]
    fn each_event_is_one_json_object() {
        let read = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert_eq!(
            read(payload(&Event::Stdout("a \"b\"".into()))),
            serde_json::json!({"kind": "stdout", "line": "a \"b\""})
        );
        assert_eq!(
            read(payload(&Event::Exit(3))),
            serde_json::json!({"kind": "exit", "code": 3})
        );
    }
}
