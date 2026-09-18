//! An `int` crosses the C boundary as a 64-bit integer, as an argument, as a
//! return, and as a struct field. A boundary that carried 32 bits would answer
//! with the low half of a wide value, or with a struct whose second field sits
//! at the wrong offset.
//!

mod ffi_fixture;

use candela::Engine;
use candela::Value;
use std::path::PathBuf;

const FIXTURE: &str = r#"
#[repr(C)]
pub struct Pair {
    pub low: i64,
    pub high: i64,
}

#[no_mangle]
pub extern "C" fn triple(x: i64) -> i64 {
    x * 3
}

#[no_mangle]
pub extern "C" fn split(x: i64) -> Pair {
    Pair {
        low: x,
        high: x + 1,
    }
}
"#;

/// The library the `dylib` block under test binds to: `triple` takes and
/// returns a wide `int`, and `split` returns a struct of two.
fn build_fixture(dir: &std::path::Path) -> PathBuf {
    ffi_fixture::build_cdylib(dir, "intwidth", FIXTURE)
}

const PROGRAM: &str = "
struct Pair { low: int, high: int }

dylib \"intwidth\" {
    int triple(int);
    Pair split(int);
}

fn tripled() {
    return intwidth::triple(3000000000);
}

fn split_low() {
    return intwidth::split(5000000000).low;
}

fn split_high() {
    return intwidth::split(5000000000).high;
}

fn main() {}
";

/// An argument and a return past 32 bits cross intact, and both fields of a
/// returned struct of them read back at their own offsets.
#[test]
fn wide_ints_cross_the_c_boundary() {
    let root = ffi_fixture::scratch_dir("int", "wide");
    let _library = build_fixture(&root);
    let script = root.join("app.cdl");

    let engine = Engine::new();
    let mut program = engine
        .compile(PROGRAM, script.to_str().expect("scratch path is utf-8"))
        .expect("the fixture import compiles");
    let tripled = program.call("tripled", &[]).expect("tripled runs");
    let low = program.call("split_low", &[]).expect("split_low runs");
    let high = program.call("split_high", &[]).expect("split_high runs");
    std::fs::remove_dir_all(&root).ok();

    assert_eq!(tripled, Value::Int(9_000_000_000));
    assert_eq!(low, Value::Int(5_000_000_000));
    assert_eq!(high, Value::Int(5_000_000_001));
}
