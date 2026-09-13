//! A struct returned by value from a `dylib` function crosses into candela
//! one field at a time, and a string field's allocation can run the string
//! collector partway through. The fields already built have to survive that.
//!
//! The library under test is built here with `rustc`, which every `cargo test`
//! run has on its path, so the test needs no C toolchain.

use candela::Engine;
use candela::Value;
use candela_vm::rt::TargetOs;
use candela_vm::rt::resolve_library_filename;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

/// Enough string fields that building the struct on a fresh program crosses
/// the string collector's starting threshold partway through.
const FIELDS: usize = 300;

/// A unique scratch directory under the system temp dir.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela_ffi_struct_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The value the fixture puts in field `i`. Longer than the inline-string
/// limit, so every field is a pooled string.
fn field_value(i: usize) -> String {
    format!("value-of-field-{i}")
}

/// Builds `libfixture` (or this platform's spelling of it) in `dir`, exporting
/// `wide()`, which returns a struct of `FIELDS` C strings by value.
fn build_fixture(dir: &std::path::Path) -> PathBuf {
    let mut source = String::from("use std::os::raw::c_char;\n#[repr(C)]\npub struct Wide {\n");
    for i in 0..FIELDS {
        writeln!(source, "    pub f{i}: *const c_char,").unwrap();
    }
    source.push_str("}\n#[no_mangle]\npub extern \"C\" fn wide() -> Wide {\n    Wide {\n");
    for i in 0..FIELDS {
        writeln!(source, "        f{i}: c\"{}\".as_ptr(),", field_value(i)).unwrap();
    }
    source.push_str("    }\n}\n");
    let source_path = dir.join("fixture.rs");
    std::fs::write(&source_path, source).expect("write fixture source");

    let library = dir.join(resolve_library_filename("fixture", TargetOs::CURRENT));
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

/// The candela side: the same struct, the import, and a check that reads every
/// field back and counts the ones that do not hold what the library put there.
fn program() -> String {
    let mut src = String::from("struct Wide {\n");
    for i in 0..FIELDS {
        writeln!(src, "    f{i}: string,").unwrap();
    }
    src.push_str("}\n\ndylib \"fixture\" {\n    Wide wide();\n}\n\nfn check() {\n    let w = fixture::wide();\n    let bad = 0;\n");
    for i in 0..FIELDS {
        writeln!(
            src,
            "    if w.f{i} != \"{}\" {{ bad = bad + 1; }}",
            field_value(i)
        )
        .unwrap();
    }
    src.push_str("    return bad;\n}\n\nfn main() {}\n");
    src
}

/// Every field of a wide struct of strings reads back as the library set it,
/// including the ones built before the collector ran.
#[test]
fn every_string_field_survives_a_collection_during_the_return() {
    let root = scratch_dir("wide");
    let _library = build_fixture(&root);
    let script = root.join("app.cdl");

    let engine = Engine::new();
    let mut program = engine
        .compile(&program(), script.to_str().expect("scratch path is utf-8"))
        .expect("the fixture import compiles");
    let bad = program.call("check", &[]).expect("check runs");
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(bad, Value::Int(0));
}
