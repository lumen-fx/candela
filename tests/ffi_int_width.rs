//! An `int` crosses the C boundary as a 64-bit integer, as an argument, as a
//! return, and as a struct field. A boundary that carried 32 bits would answer
//! with the low half of a wide value, or with a struct whose second field sits
//! at the wrong offset.
//!
//! The library under test is built here with `rustc`, which every `cargo test`
//! run has on its path, so the test needs no C toolchain.

use candela::Engine;
use candela::Value;
use candela_vm::rt::TargetOs;
use candela_vm::rt::resolve_library_filename;
use std::path::PathBuf;
use std::process::Command;

/// A unique scratch directory under the system temp dir.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela_ffi_int_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

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

/// Builds the fixture library in `dir` and returns its path.
fn build_fixture(dir: &std::path::Path) -> PathBuf {
    let source_path = dir.join("fixture.rs");
    std::fs::write(&source_path, FIXTURE).expect("write fixture source");
    let library = dir.join(resolve_library_filename("intwidth", TargetOs::CURRENT));
    let output = Command::new("rustc")
        .arg("--edition")
        .arg("2021")
        .arg("--crate-type")
        .arg("cdylib")
        .arg("-o")
        .arg(&library)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(
        output.status.success(),
        "rustc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    library
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
    let root = scratch_dir("wide");
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
