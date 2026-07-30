//! Integration tests for the import statement's binding rules.
//!
//! A bare `import "path";` merges the module's symbols (functions, structs,
//! enums, impl methods) into the importing file's own scope; `import "path" as
//! name;` binds them behind the `name::` namespace instead. A bare import that
//! would redefine a name is a compile-time error naming both sources. These
//! tests run small multi-file programs through the `candela` binary, since
//! import resolution is relative to real files on disk.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Creates a fresh scratch directory, writes the given files into it, runs
/// `prog.cdl` through the `candela` binary, and cleans up.
fn run_program(test_name: &str, files: &[(&str, &str)]) -> Output {
    let dir = std::env::temp_dir().join(format!(
        "candela_imports_{test_name}_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    for (name, contents) in files {
        std::fs::write(dir.join(name), contents).expect("write test file");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(dir.join("prog.cdl"))
        .env_remove("CANDELA_LIB_PATH")
        .output()
        .expect("candela binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[cfg(not(feature = "embed"))]
#[test]
fn bare_file_import_merges_into_scope() {
    let output = run_program(
        "bare_merge",
        &[
            ("helper.cdl", "fn ping() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"helper.cdl\";\nfn main() { print(ping()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains('5'));
}

#[cfg(not(feature = "embed"))]
#[test]
fn aliased_import_stays_namespaced() {
    // Two modules exporting the same name coexist behind aliases.
    let output = run_program(
        "aliased",
        &[
            ("a.cdl", "fn ping() { return 1; }\n"),
            ("b.cdl", "fn ping() { return 2; }\n"),
            (
                "prog.cdl",
                "import \"a.cdl\" as a;\nimport \"b.cdl\" as b;\nfn main() { print(a::ping() + b::ping()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains('3'));
}

#[test]
fn bare_import_collision_with_local_definition_errors() {
    let output = run_program(
        "collide_local",
        &[
            ("helper.cdl", "fn ping() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"helper.cdl\";\nfn ping() { return 6; }\nfn main() { print(ping()); }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ping") && stderr.contains("defined in this file"),
        "stderr: {stderr}"
    );
}

#[test]
fn bare_import_collision_between_two_imports_errors() {
    let output = run_program(
        "collide_imports",
        &[
            ("a.cdl", "fn ping() { return 1; }\n"),
            ("b.cdl", "fn ping() { return 2; }\n"),
            (
                "prog.cdl",
                "import \"a.cdl\";\nimport \"b.cdl\";\nfn main() { print(ping()); }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ping") && stderr.contains("a.cdl") && stderr.contains("b.cdl"),
        "stderr: {stderr}"
    );
}

#[cfg(not(feature = "embed"))]
#[test]
fn diamond_bare_imports_are_not_a_collision() {
    // Two modules that both bare-import a third re-export the same underlying
    // symbols; importing both is not a conflict.
    let output = run_program(
        "diamond",
        &[
            ("base.cdl", "fn shared() { return 7; }\n"),
            (
                "a.cdl",
                "import \"base.cdl\";\nfn from_a() { return shared(); }\n",
            ),
            (
                "b.cdl",
                "import \"base.cdl\";\nfn from_b() { return shared() + 1; }\n",
            ),
            (
                "prog.cdl",
                "import \"a.cdl\";\nimport \"b.cdl\";\nfn main() { print(from_a() + from_b()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("15"));
}

#[test]
fn legacy_namespaced_import_suggests_replacement() {
    let output = run_program(
        "legacy_form",
        &[("prog.cdl", "import std::list;\nfn main() { print(1); }\n")],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("import \"std/list\";"), "stderr: {stderr}");
}

/// A bare library import merges the shipped module into scope; the enum, its
/// impl methods, and the free helpers all arrive.
#[cfg(not(feature = "embed"))]
#[test]
fn bare_library_import_merges_enum_and_methods() {
    let dir = std::env::temp_dir().join(format!("candela_imports_lib_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(
        dir.join("prog.cdl"),
        "import \"std/option\";\nfn main() { print(unwrap(Some(7))); print(is_some(None)); print(Some(1).unwrap_or(9)); }\n",
    )
    .expect("write test file");
    let output = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(dir.join("prog.cdl"))
        .env("CANDELA_LIB_PATH", repo().join("libs"))
        .output()
        .expect("candela binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains('7') && stdout.contains("false") && stdout.contains('1'),
        "stdout: {stdout}"
    );
}
