//! The event seam: a native library posts from its own thread, and the
//! program's closure runs on the program's thread.
//!
//! The fixture library receives the runtime's table through
//! `candela_dylib_init` and starts a thread per call that posts a few events
//! under the source it was given, then releases it.

mod built;
mod common;
mod ffi_fixture;

use candela::Engine;
use candela::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The `candela-vm` binary, built for this run; see `built`.
fn candela_vm() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            built::build(
                &["-p", "candela-vm", "--bin", "candela-vm"],
                &format!("candela-vm{}", std::env::consts::EXE_SUFFIX),
            )
        })
        .clone()
}

const FIXTURE: &str = r#"
use std::ffi::{c_char, CString};

#[repr(C)]
pub struct Table {
    abi: u32,
    new_source: extern "C" fn() -> i64,
    post: extern "C" fn(i64, *const c_char),
    hold: extern "C" fn(i64),
    release: extern "C" fn(i64),
    base_dir: extern "C" fn() -> *const c_char,
}

static mut TABLE: *const Table = std::ptr::null();

fn table() -> &'static Table {
    unsafe { &*TABLE }
}

#[no_mangle]
pub unsafe extern "C" fn candela_dylib_init(table: *const Table) {
    TABLE = table;
}

/// Holds `source`, then posts `count` events from a thread of its own, a
/// pause apart, and releases the source.
#[no_mangle]
pub extern "C" fn tick(source: i64, count: i64, pause_ms: i64) -> i64 {
    (table().hold)(source);
    std::thread::spawn(move || {
        for i in 0..count {
            std::thread::sleep(std::time::Duration::from_millis(pause_ms as u64));
            let text = CString::new(format!("tick {i}")).unwrap();
            (table().post)(source, text.as_ptr());
        }
        (table().release)(source);
    });
    1
}

/// A source the library made itself, posted to once and released.
#[no_mangle]
pub extern "C" fn own_source() -> i64 {
    let source = (table().new_source)();
    (table().hold)(source);
    let text = CString::new("from the library").unwrap();
    (table().post)(source, text.as_ptr());
    (table().release)(source);
    source
}

#[no_mangle]
pub extern "C" fn abi() -> i64 {
    table().abi as i64
}
"#;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch directory with the fixture library built into it.
fn fixture(tag: &str) -> PathBuf {
    let root = ffi_fixture::scratch_dir("events", tag);
    ffi_fixture::build_cdylib(&root, "ticker", FIXTURE);
    root
}

/// Runs `program` with the `candela` binary from `dir`, answering its stdout.
fn run(dir: &Path, program: &str) -> String {
    let app = dir.join("app.cdl");
    std::fs::write(&app, program).expect("write the program");
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg(&app)
        .current_dir(dir)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let output = common::output_with_deadline(&mut command, "candela run");
    assert!(
        output.status.success(),
        "the run failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

const AFTER_MAIN: &str = r#"
import "std/event";

dylib "ticker" {
    int tick(int, int, int);
}

fn main() {
    let id = event::source();
    let seen = 0;
    event::on(id, fn(payload) {
        seen += 1;
        print(payload + " (" + string(seen) + ")");
    });
    ticker::tick(id, 3, 30);
    print("main is done");
}
"#;

/// Work `main` started keeps the program running, and its events reach the
/// closure in the order they were posted, after `main` returns.
#[test]
fn handlers_run_after_main_while_work_is_held() {
    let dir = fixture("after_main");
    let out = run(&dir, AFTER_MAIN);
    assert_eq!(
        out, "main is done\ntick 0 (1)\ntick 1 (2)\ntick 2 (3)\n",
        "the closure runs once per event, after main, in order"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The same program built to an artifact behaves the same under `candela-vm`.
#[test]
fn an_artifact_keeps_dispatching_under_candela_vm() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = candela_vm();
    let dir = fixture("artifact");
    // An artifact re-opens a library by the name it recorded, and a bare name
    // goes to the system loader, which does not look in the working
    // directory. A path does, so this program names the library by one.
    // On Windows the two names are the same file, `ticker.dll`.
    let ext = candela_vm::rt::TargetOs::CURRENT.dynamic_lib_extension();
    let built = dir.join(candela_vm::rt::resolve_library_filename(
        "ticker",
        candela_vm::rt::TargetOs::CURRENT,
    ));
    let by_path = dir.join(format!("ticker.{ext}"));
    if built != by_path {
        std::fs::copy(&built, &by_path).unwrap();
    }
    let app = dir.join("app.cdl");
    std::fs::write(
        &app,
        AFTER_MAIN.replace("dylib \"ticker\"", "dylib \"./ticker\""),
    )
    .unwrap();
    let artifact = dir.join("app.cdlb");
    let mut build = Command::new(candela);
    build
        .arg("build")
        .arg(&app)
        .arg("-o")
        .arg(&artifact)
        .current_dir(&dir)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let built = common::output_with_deadline(&mut build, "candela build");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut vm = Command::new(&candela_vm);
    vm.arg(&artifact).current_dir(&dir);
    let out = common::output_with_deadline(&mut vm, "candela-vm run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        "main is done\ntick 0 (1)\ntick 1 (2)\ntick 2 (3)\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `dispatch` called from `main` runs what arrived, and with `wait` blocks for
/// more while a source is held.
#[test]
fn main_can_dispatch_itself() {
    let dir = fixture("explicit");
    let out = run(
        &dir,
        r#"
import "std/event";

dylib "ticker" {
    int tick(int, int, int);
}

fn main() {
    let id = event::source();
    event::on(id, fn(payload) { print(payload); });
    ticker::tick(id, 2, 10);
    let ran = 0;
    while ran < 2 {
        ran += event::dispatch(1, true);
    }
    print("ran " + string(ran) + " in main");
    print(event::dispatch(-1, false));
}
"#,
    );
    assert_eq!(out, "tick 0\ntick 1\nran 2 in main\n0\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// A program that imports nothing that holds a source exits at the end of
/// `main`, and one with no handler for a source drops its events.
#[test]
fn nothing_held_means_main_is_the_end() {
    let dir = fixture("nothing_held");
    let out = run(
        &dir,
        r#"
import "std/event";

dylib "ticker" {
    int own_source();
    int abi();
}

fn main() {
    print(ticker::abi());
    let id = ticker::own_source();
    print(id > 0);
}
"#,
    );
    assert_eq!(out, "1\ntrue\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// An embedder runs handlers from its own loop: the waker says an event
/// arrived, and `dispatch_events` runs it on the program's thread.
#[test]
fn an_embedder_dispatches_from_its_own_loop() {
    let dir = fixture("embedder");
    let woken = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&woken);
    candela::set_event_waker(Some(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })));

    let script = dir.join("app.cdl");
    let mut program = Engine::new()
        .with_lib_dir(repo().join("libs"))
        .compile(
            r#"
import "std/event";

dylib "ticker" {
    int tick(int, int, int);
}

fn start() -> int {
    let id = event::source();
    let seen = [""];
    event::on(id, fn(payload) { seen.push(payload); });
    ticker::tick(id, 2, 5);
    return id;
}

fn main() {}
"#,
            script.to_str().unwrap(),
        )
        .expect("the program compiles");
    assert!(matches!(
        program.call("start", &[]).expect("start runs"),
        Value::Int(_)
    ));

    let mut ran = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while ran < 2 && std::time::Instant::now() < deadline {
        ran += program.dispatch_events(16).expect("handlers run");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    candela::set_event_waker(None);
    assert_eq!(ran, 2, "both events ran through dispatch_events");
    assert!(
        woken.load(Ordering::SeqCst) >= 2,
        "the waker fired for each post"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A program that does not import `std/event` has nothing to dispatch.
#[test]
fn without_std_event_there_is_nothing_to_dispatch() {
    let mut program = Engine::new()
        .compile("fn main() {}", "plain.cdl")
        .expect("compiles");
    assert_eq!(program.dispatch_events(10).expect("nothing to run"), 0);
}

/// A handler held only by the runtime survives a full collection between its
/// registration and its event.
#[test]
fn a_registered_handler_survives_a_collection() {
    let dir = fixture("collect");
    let script = dir.join("app.cdl");
    let mut program = Engine::new()
        .with_lib_dir(repo().join("libs"))
        .compile(
            r#"
import "std/event";

dylib "ticker" {
    int tick(int, int, int);
}

fn start() {
    let id = event::source();
    let label = "seen " + string(id);
    event::on(id, fn(payload) { print(label + ": " + payload); });
    let garbage = [];
    for i in 0..2000 {
        garbage.push([i, i + 1]);
    }
    ticker::tick(id, 1, 50);
}

fn main() {}
"#,
            script.to_str().unwrap(),
        )
        .expect("compiles");
    program.call("start", &[]).expect("start runs");
    while !program.collect(u32::MAX) {}
    let mut ran = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while ran < 1 && std::time::Instant::now() < deadline {
        ran += program.dispatch_events(1).expect("the handler runs");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(ran, 1);
    std::fs::remove_dir_all(&dir).ok();
}
