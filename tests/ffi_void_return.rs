//! A C function that returns nothing is still called: its effect is the point
//! of calling it.

mod ffi_fixture;

use candela::Engine;
use candela::Value;

const FIXTURE: &str = r#"
use std::sync::atomic::{AtomicI64, Ordering};

static STORED: AtomicI64 = AtomicI64::new(0);

#[no_mangle]
pub extern "C" fn store(x: i64) {
    STORED.store(x, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn stored() -> i64 {
    STORED.load(Ordering::SeqCst)
}
"#;

const PROGRAM: &str = "
dylib \"voidret\" {
    store(int);
    int stored();
}

fn round_trip() {
    voidret::store(42);
    return voidret::stored();
}

fn main() {}
";

#[test]
fn a_void_c_function_runs() {
    let root = ffi_fixture::scratch_dir("void", "store");
    let _library = ffi_fixture::build_cdylib(&root, "voidret", FIXTURE);
    let script = root.join("app.cdl");

    let mut program = Engine::new()
        .compile(PROGRAM, script.to_str().expect("scratch path is utf-8"))
        .expect("the fixture import compiles");
    let value = program.call("round_trip", &[]).expect("round_trip runs");
    std::fs::remove_dir_all(&root).ok();

    assert_eq!(value, Value::Int(42));
}
