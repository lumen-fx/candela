//! A library that posts events keeps running code on threads of its own after
//! its last event: returning from the thread, dropping what the thread held.
//! The program that loaded it may be gone by then, so the runtime must not
//! unload such a library when the program drops it.
//!
//! The fixture's thread posts, releases its source, then stays in the
//! library's code for a while before it returns. The test drops the program
//! as soon as the last event has run, then waits for the thread: a library
//! unloaded under it takes the process down with an access violation.
//!
//! This file is a test binary of its own, so a crash names it.

mod ffi_fixture;

use candela::Engine;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FIXTURE: &str = r#"
use std::ffi::{c_char, c_void};

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

#[no_mangle]
pub unsafe extern "C" fn candela_dylib_init(table: *const Table) {
    TABLE = table;
}

// The thread is started through the operating system rather than std, so
// nothing about it registers a thread-local destructor that would keep the
// library loaded on its own.
#[cfg(unix)]
mod os {
    use std::ffi::c_void;
    extern "C" {
        fn pthread_create(
            thread: *mut usize,
            attr: *const c_void,
            start: extern "C" fn(*mut c_void) -> *mut c_void,
            arg: *mut c_void,
        ) -> i32;
        fn pthread_detach(thread: usize) -> i32;
        fn usleep(us: u32) -> i32;
    }
    pub unsafe fn spawn(start: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) {
        let mut thread = 0usize;
        pthread_create(&mut thread, std::ptr::null(), start, arg);
        pthread_detach(thread);
    }
    pub unsafe fn sleep_ms(ms: u32) {
        usleep(ms * 1000);
    }
}

#[cfg(windows)]
mod os {
    use std::ffi::c_void;
    extern "system" {
        fn CreateThread(
            attrs: *const c_void,
            stack: usize,
            start: extern "system" fn(*mut c_void) -> u32,
            arg: *mut c_void,
            flags: u32,
            id: *mut u32,
        ) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn Sleep(ms: u32);
    }
    extern "system" fn trampoline(arg: *mut c_void) -> u32 {
        super::work(arg);
        0
    }
    pub unsafe fn spawn(_start: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) {
        let handle = CreateThread(std::ptr::null(), 0, trampoline, arg, 0, std::ptr::null_mut());
        CloseHandle(handle);
    }
    pub unsafe fn sleep_ms(ms: u32) {
        Sleep(ms);
    }
}

static mut DONE: i64 = 0;

extern "C" fn work(arg: *mut c_void) -> *mut c_void {
    let source = arg as i64;
    unsafe {
        ((*TABLE).post)(source, b"last\0".as_ptr().cast());
        ((*TABLE).release)(source);
        // Still in the library's code after the program has seen the end.
        os::sleep_ms(300);
        std::ptr::write_volatile(&raw mut DONE, 1);
    }
    std::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn start(source: i64) {
    ((*TABLE).hold)(source);
    os::spawn(work, source as *mut c_void);
}
"#;

#[test]
fn a_library_whose_thread_outlives_the_program_stays_loaded() {
    let root = ffi_fixture::scratch_dir("lifetime", "outlives");
    ffi_fixture::build_cdylib(&root, "lingers", FIXTURE);
    let script = root.join("app.cdl");
    let libs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("libs");

    let mut program = Engine::new()
        .with_lib_dir(libs)
        .compile(
            r#"
import "std/event";

dylib "lingers" {
    start(int);
}

fn go() {
    let id = event::source();
    event::on(id, fn(payload) {});
    lingers::start(id);
}

fn main() {}
"#,
            script.to_str().expect("the scratch path is utf-8"),
        )
        .expect("compiles");
    program.call("go", &[]).expect("go runs");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ran = 0;
    while ran == 0 && Instant::now() < deadline {
        ran += program.dispatch_events(1).expect("the handler runs");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(ran, 1);
    // The program, and every handle it held on the library, goes now, while
    // the library's thread is still sleeping in its code.
    drop(program);
    std::thread::sleep(Duration::from_millis(600));
    std::fs::remove_dir_all(&root).ok();
}
