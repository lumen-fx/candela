//! Integration tests for the Candela standard library shipped under `libs/std`.
//!
//! Behavior is checked by running each module's `libs/std/tests/*.cdl` file
//! through the `candela` binary: the file uses `import "std/<module>"` and prints
//! an "ok" line, so a zero exit with that line means every check held. These
//! tests point `CANDELA_LIB_PATH` at this checkout's `libs` directory, since the
//! test binary is not laid out like an install.
//!
//! One test instead builds an install-style layout (the binary with `libs/`
//! beside it) and runs from an unrelated directory with nothing set, to confirm
//! the default exe-relative resolution needs no environment.

use candela::{build_bytecode, load_program};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `libs/std/tests/<name>.cdl` through the `candela` binary with the library
/// path pointed at this checkout, and returns its stdout, asserting a clean exit.
fn run_std_test(name: &str) -> String {
    let path = repo().join("libs/std/tests").join(format!("{name}.cdl"));
    let output = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(&path)
        .env("CANDELA_LIB_PATH", repo().join("libs"))
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
fn list_higher_order() {
    assert!(run_std_test("test_list_hof").contains("list hof ok"));
}

#[test]
fn list_methods() {
    assert!(run_std_test("test_list_methods").contains("list methods ok"));
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
fn option_module() {
    assert!(run_std_test("test_option").contains("option ok"));
}

#[test]
fn result_module() {
    assert!(run_std_test("test_result").contains("result ok"));
}

#[test]
fn json_module() {
    assert!(run_std_test("test_json").contains("json ok"));
}

#[test]
fn map_module() {
    assert!(run_std_test("test_map").contains("map ok"));
}

#[test]
fn set_module() {
    assert!(run_std_test("test_set").contains("set ok"));
}

/// The option/result enum modules must inline into a `.cdlb` and run under the
/// VM-only path with no source tree, exactly like the other pure-candela std
/// modules.
#[test]
fn option_result_inline_into_cdlb() {
    unsafe {
        std::env::set_var("CANDELA_LIB_PATH", repo().join("libs"));
    }
    let src = r#"
import "std/option" as option;
import "std/result" as result;
fn main() {
    print(option::unwrap_or(None, 3));
    print(option::unwrap(Some(7)));
    print(result::is_ok(Ok(1)));
    print(result::unwrap_err(Err("x")));
}
"#;
    let filename = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("option_result_probe.cdl")
        .to_string_lossy()
        .into_owned();
    let bytes = build_bytecode(src.to_owned(), &filename).expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB");
    let mut program = load_program(&bytes).expect("artifact with inlined option/result must load");
    program.run();
}

/// A program that imports a library through a bare quoted path must run from any
/// working directory with nothing set, as long as `libs/` sits beside the
/// binary. This mirrors a clean install.
#[test]
fn default_resolution_needs_no_env() {
    let tmp = std::env::temp_dir().join(format!("candela_install_{}", std::process::id()));
    let bin_dir = tmp.join("bin");
    let std_dir = tmp.join("bin/libs/std");
    std::fs::create_dir_all(&std_dir).expect("create install layout");

    // The binary with `libs/std` beside it. `import "std/list"` needs only
    // list.cdl, which imports nothing else.
    let installed_bin = bin_dir.join("candela");
    std::fs::copy(env!("CARGO_BIN_EXE_candela"), &installed_bin).expect("copy binary");
    std::fs::copy(repo().join("libs/std/list.cdl"), std_dir.join("list.cdl")).expect("copy module");

    // A program in an unrelated directory.
    let work = tmp.join("work");
    std::fs::create_dir_all(&work).expect("create work dir");
    std::fs::write(
        work.join("prog.cdl"),
        "import \"std/list\";\nfn main() { print(max([5, 9, 2])); }\n",
    )
    .expect("write program");

    let output = Command::new(&installed_bin)
        .arg("prog.cdl")
        .current_dir(&work)
        .env_remove("CANDELA_LIB_PATH")
        .output()
        .expect("installed candela runs");

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains('9'),
        "zero-config run failed: {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// The json/map/set modules must inline into a `.cdlb` and run under the
/// VM-only path with no source tree, like the other pure-candela std modules.
#[test]
fn json_map_set_inline_into_cdlb() {
    unsafe {
        std::env::set_var("CANDELA_LIB_PATH", repo().join("libs"));
    }
    let src = r#"
import "std/json" as json;
import "std/map" as map;
import "std/set" as set;
fn main() {
    let obj = as_map(json::parse("{\"n\": 7, \"xs\": [1, 2, 3]}"));
    print(as_int(map::get(obj, "n")));
    print(as_list(map::get(obj, "xs")).len());
    print(json::stringify(json::parse("[1,2,3]")));
    let s = set::new();
    set::add(s, 1);
    set::add(s, 1);
    set::add(s, 2);
    print(set::len(s));
}
"#;
    let filename = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("json_map_set_probe.cdl")
        .to_string_lossy()
        .into_owned();
    let bytes = build_bytecode(src.to_owned(), &filename).expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB");
    let mut program = load_program(&bytes).expect("artifact with inlined json/map/set must load");
    program.run();
}

#[test]
fn std_inlines_into_cdlb() {
    // A program that imports pure-Candela std modules must build to a `.cdlb`
    // whose image carries the module bytecode inline, so the artifact loads with
    // no source tree present. `CANDELA_LIB_PATH` lets the compiler find the
    // modules from this checkout.
    unsafe {
        std::env::set_var("CANDELA_LIB_PATH", repo().join("libs"));
    }
    let src = r#"
import "std/list" as list;
import "std/string" as string;
fn inc(x) { return x + 1; }
fn main() {
    print(list::sum([1, 2, 3]));
    print(list::map([1, 2, 3], inc));
    print(list::filter([1, 2, 3, 4], fn(x) { return x % 2 == 0; }));
    print(string::capitalize("hi"));
}
"#;
    let filename = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("std_cdlb_probe.cdl")
        .to_string_lossy()
        .into_owned();
    let bytes = build_bytecode(src.to_owned(), &filename).expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");
    assert!(
        load_program(&bytes).is_ok(),
        "artifact with inlined std must load"
    );
}
