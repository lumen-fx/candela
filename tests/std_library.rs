//! Integration tests for the Candela standard library shipped under `libs/std`.
//!
//! Behavior is checked by running each module's `libs/std/tests/*.cdl` file
//! through the `candela` binary: the file imports a std module, exercises it with
//! assertions, and prints an "ok" line, so a zero exit with that line means every
//! check held. This drives the same path a user runs (`candela run file.cdl`).
//!
//! A separate check confirms a std-importing program builds to a `.cdlb` artifact
//! with the module bytecode inlined, so the artifact loads with no source tree
//! present.

use candela::{build_bytecode, load_program};
use std::path::PathBuf;
use std::process::Command;

/// Runs `libs/std/tests/<name>.cdl` through the `candela` binary and returns its
/// stdout, asserting a clean exit.
fn run_std_test(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("libs/std/tests")
        .join(format!("{name}.cdl"));
    let output = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(&path)
        .output()
        .expect("candela binary runs");
    assert!(
        output.status.success(),
        "{name} exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn string_module() {
    assert!(run_std_test("test_string").contains("string ok"));
}

#[test]
fn list_reductions() {
    assert!(run_std_test("test_list").contains("list ok"));
}

#[test]
fn list_slices() {
    assert!(run_std_test("test_list_slices").contains("list slices ok"));
}

#[test]
fn convert_module() {
    assert!(run_std_test("test_convert").contains("convert ok"));
}

#[test]
fn assert_module() {
    assert!(run_std_test("test_assert").contains("assert ok"));
}

#[test]
fn std_inlines_into_cdlb() {
    // A program that imports pure-Candela std modules must build to a `.cdlb`
    // whose image carries the module bytecode inline, so the artifact loads with
    // no source tree present.
    let src = r#"
import "libs/std/list.cdl";
import "libs/std/string.cdl";
fn main() {
    print(list::sum([1, 2, 3]));
    print(string::capitalize("hi"));
}
"#;
    let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/std_cdlb_probe.cdl");
    let bytes = build_bytecode(src.to_owned(), filename).expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");
    assert!(
        load_program(&bytes).is_ok(),
        "artifact with inlined std must load"
    );
}
