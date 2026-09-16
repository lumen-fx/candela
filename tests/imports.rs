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
    program_output(test_name, files, &[])
}

/// The same, stopping at `candela check`, for a program with no `main` to run.
fn check_program(test_name: &str, files: &[(&str, &str)]) -> Output {
    program_output(test_name, files, &["check"])
}

fn program_output(test_name: &str, files: &[(&str, &str)], verb: &[&str]) -> Output {
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
        .args(verb)
        .arg(dir.join("prog.cdl"))
        .env_remove("CANDELA_LIB_PATH");
    let output = common::output_with_deadline(&mut command, "import run");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// A module with a function, an enum and a struct with a method, so a test can
/// check that every kind of name a module declares reaches the file that
/// imported it.
const BASE_MODULE: &str = "enum Tag { A(int), B }\n\
                           struct Cell { n: int }\n\
                           impl Cell { fn doubled(self) { return self.n * 2; } }\n\
                           fn tag(n: int) -> Tag { return Tag::A(n); }\n\
                           fn cell(n: int) -> Cell { return Cell { n: n }; }\n\
                           fn unwrap(t: Tag) -> int {\n\
                               let out = -1;\n\
                               match t { A(n) => { out = n; } B => { out = 0; } }\n\
                               return out;\n\
                           }\n";

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

/// A module that declares `main` can be bare-imported: only the entry file's
/// `main` runs, so the module's is not one of the names the merge brings in.
#[test]
fn bare_import_of_a_module_with_main_keeps_the_importers_main() {
    let output = run_program(
        "bare_module_main",
        &[
            (
                "helper.cdl",
                "fn ping() { return 5; }\nfn main() { print(ping()); }\n",
            ),
            (
                "prog.cdl",
                "import \"helper.cdl\";\nfn main() { print(ping() + 1); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The importer's `main` ran, and the module's did not.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "6", "stdout: {stdout}");
}

/// A bare import holds back only the module's `main` function. A struct that
/// happens to carry the same name is a separate symbol and merges like any
/// other; it used to be dropped with the function, leaving the importer with
/// "Unknown struct main".
///
/// Checked rather than run: functions and types share one namespace, so a
/// module's `struct main` still collides with the importing file's own `fn
/// main`. A library entry, which has none, is where the struct is reachable.
#[test]
fn bare_import_keeps_a_struct_named_main() {
    let output = check_program(
        "bare_module_main_struct",
        &[
            (
                "helper.cdl",
                "struct main { a: int }\nfn ping() { return 5; }\nfn main() { print(ping()); }\n",
            ),
            (
                "prog.cdl",
                "import \"helper.cdl\";\nfn build() { let m = main { a: 7 }; return m.a + ping(); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The files a program reaches a module through each bind it in their own
/// form. `base.cdl` here is reached twice, once behind an alias in the entry
/// file and once by `mid.cdl`'s bare import, and both routes work: the entry
/// calls into `base::`, and `mid`'s own bodies call the names it merged.
#[test]
fn a_module_reached_by_two_files_binds_in_both() {
    let output = run_program(
        "two_routes_alias",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "mid.cdl",
                "import \"base.cdl\";\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"base.cdl\" as base;\n\
                 import \"mid.cdl\" as mid;\n\
                 fn main() { print(base::unwrap(mid::from_mid(4)) + base::cell(2).doubled()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "9");
}

/// Which of the two routes the entry file writes first makes no difference.
#[test]
fn the_order_of_the_two_routes_to_a_module_does_not_matter() {
    let output = run_program(
        "two_routes_order",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "mid.cdl",
                "import \"base.cdl\";\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"mid.cdl\" as mid;\n\
                 import \"base.cdl\" as base;\n\
                 fn main() { print(base::unwrap(mid::from_mid(4)) + base::cell(2).doubled()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "9");
}

/// A bare import of a module another import already reached merges it into the
/// importing file's own scope, so its function, its enum and its struct's
/// method are all written unqualified.
#[test]
fn a_module_reached_bare_and_through_another_merges_into_scope() {
    let output = run_program(
        "two_routes_bare",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "mid.cdl",
                "import \"base.cdl\";\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"base.cdl\";\n\
                 import \"mid.cdl\" as mid;\n\
                 fn main() { print(unwrap(mid::from_mid(4)) + cell(2).doubled() + unwrap(tag(1))); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "10");
}

/// A module bare-imported directly and bare-imported again through another
/// module brings its names in twice, and the same underlying symbol arriving
/// twice is not a collision.
#[test]
fn bare_imports_of_a_module_and_of_its_importer_are_not_a_collision() {
    let output = run_program(
        "two_routes_both_bare",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "mid.cdl",
                "import \"base.cdl\";\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"base.cdl\";\n\
                 import \"mid.cdl\";\n\
                 fn main() { print(unwrap(from_mid(4)) + cell(3).doubled()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "11");
}

/// One file reached through two spellings is one module. A library import
/// resolves through the library directory and a source-relative one next to
/// the importing file; when both land on the same file it is parsed once and
/// registers its functions, structs and enums once, so the second route brings
/// in the symbols the first one already did rather than a second copy of them.
#[test]
fn a_module_spelled_two_ways_is_loaded_once() {
    let dir = std::env::temp_dir().join(format!("candela_imports_one_load_{}", std::process::id()));
    let libs = dir.join("libs");
    std::fs::create_dir_all(&libs).expect("create scratch dir");
    std::fs::write(libs.join("shared.cdl"), BASE_MODULE).expect("write module");
    std::fs::write(
        dir.join("prog.cdl"),
        "import \"shared\";\n\
         import \"./libs/shared.cdl\";\n\
         fn main() { print(unwrap(tag(4)) + cell(1).doubled()); }\n",
    )
    .expect("write test file");
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg(dir.join("prog.cdl"))
        // Spelled with a `..` the source-relative route does not take, so the
        // two imports meet only if each is resolved to the file behind it.
        .env("CANDELA_LIB_PATH", libs.join("..").join("libs"));
    let output = common::output_with_deadline(&mut command, "one load run");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "6");
}
