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

mod common;

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
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg(dir.join("prog.cdl"))
        .env_remove("CANDELA_LIB_PATH");
    let output = common::output_with_deadline(&mut command, "import run");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

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

/// A call into an aliased module's namespace names the function it cannot find
/// and offers the closest one the module declares. The same report serves a
/// `host` namespace, whose functions are declared outside the namespace tree.
#[test]
fn unknown_function_in_an_aliased_namespace_suggests_the_declared_one() {
    let output = run_program(
        "unknown_in_namespace",
        &[
            ("helper.cdl", "fn compute() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"helper.cdl\" as h;\nfn main() { print(h::computee()); }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot find function")
            && stderr.contains("computee")
            && stderr.contains("in namespace")
            && stderr.contains("h::compute"),
        "stderr: {stderr}"
    );
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

/// A bare library import merges the shipped module into scope; the enum and
/// its impl methods both arrive.
#[test]
fn bare_library_import_merges_enum_and_methods() {
    let dir = std::env::temp_dir().join(format!("candela_imports_lib_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(
        dir.join("prog.cdl"),
        "import \"std/option\";\nfn main() { print(Some(7).unwrap()); print(None.is_some()); print(Some(1).unwrap_or(9)); }\n",
    )
    .expect("write test file");
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg(dir.join("prog.cdl"))
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let output = common::output_with_deadline(&mut command, "std import run");
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

/// A module bound behind an alias resolves the names in its own bodies: a
/// sibling function, a struct literal, and a struct it declared.
#[test]
fn aliased_module_sees_its_own_declarations() {
    let output = run_program(
        "aliased_scope",
        &[
            (
                "geom.cdl",
                "struct Point { x: int, y: int }\n\
                 impl Point { fn shifted(self) { return Point { x: self.x + 1, y: self.y }; } }\n\
                 fn double(n) { return n * 2; }\n\
                 fn origin() { return Point { x: 0, y: 0 }; }\n\
                 fn scaled(n) { return double(n); }\n",
            ),
            (
                "prog.cdl",
                "import \"geom.cdl\" as geom;\n\
                 fn main() {\n\
                 print(geom::scaled(4));\n\
                 print(geom::origin().shifted().x);\n\
                 }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains('8') && stdout.contains('1'), "{stdout}");
}

/// The same for an enum a module declares: a qualified variant path and a bare
/// variant both resolve in the module's own scope, not the importer's.
#[test]
fn aliased_module_sees_its_own_enum() {
    let output = run_program(
        "aliased_enum",
        &[
            (
                "shapes.cdl",
                "enum Shape { Circle(int), Empty }\n\
                 fn unit() { return Shape::Circle(1); }\n\
                 fn nothing() { return Empty; }\n",
            ),
            (
                "prog.cdl",
                "import \"shapes.cdl\" as shapes;\n\
                 fn main() { print(str(shapes::unit())); print(str(shapes::nothing())); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Circle(1)") && stdout.contains("Empty"),
        "{stdout}"
    );
}

/// An aliased module's own imports are its own: what it bare-imported and what
/// it aliased are reachable from its bodies, and a closure it builds still
/// resolves the function it calls.
#[test]
fn aliased_module_keeps_its_own_imports() {
    let output = run_program(
        "aliased_nested",
        &[
            ("base.cdl", "fn twice(n) { return n * 2; }\n"),
            ("side.cdl", "fn triple(n) { return n * 3; }\n"),
            (
                "lib.cdl",
                "import \"base.cdl\";\n\
                 import \"side.cdl\" as side;\n\
                 fn apply(f, n) { return f(n); }\n\
                 fn compute(n) { let step = fn(x) { return twice(x); }; return apply(step, side::triple(n)); }\n",
            ),
            (
                "prog.cdl",
                "import \"lib.cdl\" as lib;\nfn main() { print(lib::compute(2)); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("12"));
}
