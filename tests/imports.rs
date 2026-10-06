//! Integration tests for the import statement's binding rules.
//!
//! `import "path";` binds the module under the last segment of its path, so
//! its names are reached as `name::symbol`; `import "path" as other;` picks the
//! name. `import "path" { a, b };` brings the named items (functions, structs,
//! enums, and the impl methods of a type) into the importing file's own scope,
//! and an item that would redefine a name is a compile-time error naming both
//! sources. These tests run small multi-file programs through the `candela`
//! binary, since import resolution is relative to real files on disk.

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
        let path = dir.join(name);
        // A file name may carry a directory, for a program whose modules sit
        // in different packages.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create module dir");
        }
        std::fs::write(path, contents).expect("write test file");
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
fn a_file_import_binds_the_module_name() {
    let output = run_program(
        "binds_name",
        &[
            ("helper.cdl", "fn ping() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"helper.cdl\";\nfn main() { print(helper::ping()); }\n",
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
                "import \"a.cdl\";\nimport \"b.cdl\";\nfn main() { print(a::ping() + b::ping()); }\n",
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
fn selective_import_collision_with_local_definition_errors() {
    let output = run_program(
        "collide_local",
        &[
            ("helper.cdl", "fn ping() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"helper.cdl\" { ping };\nfn ping() { return 6; }\nfn main() { print(ping()); }\n",
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
fn selective_import_collision_between_two_imports_errors() {
    let output = run_program(
        "collide_imports",
        &[
            ("a.cdl", "fn ping() { return 1; }\n"),
            ("b.cdl", "fn ping() { return 2; }\n"),
            (
                "prog.cdl",
                "import \"a.cdl\" { ping };\nimport \"b.cdl\" { ping };\nfn main() { print(ping()); }\n",
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
fn diamond_imports_are_not_a_collision() {
    // Two modules that both import a third, and the entry file naming an item
    // of the third, reach one module; nothing collides.
    let output = run_program(
        "diamond",
        &[
            ("base.cdl", "fn shared() { return 7; }\n"),
            (
                "a.cdl",
                "import \"base.cdl\" { shared };\nfn from_a() { return shared(); }\n",
            ),
            (
                "b.cdl",
                "import \"base.cdl\";\nfn from_b() { return base::shared() + 1; }\n",
            ),
            (
                "prog.cdl",
                "import \"a.cdl\";\nimport \"b.cdl\";\nimport \"base.cdl\" { shared };\nfn main() { print(a::from_a() + b::from_b() - shared() + 7); }\n",
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

/// A prelude module imported by name keeps its enum, variants and methods
/// reachable with no prefix, as they are with no import at all.
#[test]
fn a_prelude_module_imported_by_name_keeps_its_bare_names() {
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
                "import \"geom.cdl\";\n\
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
                "import \"shapes.cdl\";\n\
                 fn main() { print(string(shapes::unit())); print(string(shapes::nothing())); }\n",
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

/// A variant of an aliased module's enum is reached through the alias and the
/// enum: `shapes::Shape::Circle(1)`, with or without a payload, and the same
/// path in a `match` arm.
#[test]
fn aliased_enum_variant_is_reached_through_its_enum() {
    let output = run_program(
        "aliased_variant",
        &[
            ("shapes.cdl", "enum Shape { Circle(int), Empty }\n"),
            (
                "prog.cdl",
                "import \"shapes.cdl\";\n\
                 fn name(s: shapes::Shape) -> string {\n\
                     let out = \"?\";\n\
                     match s {\n\
                         shapes::Shape::Circle(r) => { out = string(r); }\n\
                         shapes::Shape::Empty => { out = \"empty\"; }\n\
                     }\n\
                     return out;\n\
                 }\n\
                 fn main() {\n\
                     print(string(shapes::Shape::Circle(1)));\n\
                     print(name(shapes::Shape::Circle(7)));\n\
                     print(name(shapes::Shape::Empty));\n\
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
    assert!(
        stdout.contains("Circle(1)") && stdout.contains('7') && stdout.contains("empty"),
        "{stdout}"
    );
}

/// A generic enum an aliased module declares names its instantiation in the
/// middle of the path: `g::Slot<int>::Filled(4)`, with or without a payload,
/// and the same path in a `match` arm.
#[test]
fn aliased_generic_enum_variant_names_its_instantiation() {
    let output = run_program(
        "aliased_generic_variant",
        &[
            ("shapes.cdl", "enum Slot<T> { Filled(T), Empty }\n"),
            (
                "prog.cdl",
                "import \"shapes.cdl\" as g;\n\
                 fn read(s) {\n\
                     match s {\n\
                         g::Slot<int>::Filled(x) => { return x; }\n\
                         g::Slot<int>::Empty => { return 0; }\n\
                     }\n\
                 }\n\
                 fn main() {\n\
                     print(read(g::Slot<int>::Filled(4)));\n\
                     print(read(g::Slot<int>::Empty));\n\
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
    assert_eq!(stdout.trim(), "4\n0", "{stdout}");
}

/// A generic function an aliased module declares takes its type arguments on
/// the name at the end of the path, `m::first<int>(nums)`, and still infers
/// them from the arguments when none are written.
#[test]
fn aliased_generic_function_takes_explicit_type_arguments() {
    let output = run_program(
        "aliased_generic_call",
        &[
            ("lib.cdl", "fn first<T>(xs: T[]) -> T { return xs[0]; }\n"),
            (
                "prog.cdl",
                "import \"lib.cdl\" as m;\n\
                 fn main() {\n\
                     let nums = [1, 2, 3];\n\
                     print(m::first<int>(nums));\n\
                     print(m::first(nums));\n\
                     print(m::first<float>([1.5, 2.5]));\n\
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
    assert_eq!(stdout.trim(), "1\n1\n1.5", "{stdout}");
}

/// The type argument is what picks the specialisation when no argument
/// mentions the type parameter, so this spelling is the only way to call such a
/// function through an alias.
#[test]
fn aliased_generic_function_type_argument_picks_the_specialisation() {
    let output = run_program(
        "aliased_generic_call_unpinned",
        &[
            (
                "lib.cdl",
                "struct Signal<T> { name: string }\n\
                 fn signal<T>(name: string) { return Signal<T>{ name: name }; }\n\
                 impl Signal<int> { fn label(self) -> string { return \"int\"; } }\n\
                 impl Signal<float> { fn label(self) -> string { return \"float\"; } }\n",
            ),
            (
                "prog.cdl",
                "import \"lib.cdl\" as m;\n\
                 fn main() {\n\
                     print(m::signal<int>(\"a\").label());\n\
                     print(m::signal<float>(\"b\").label());\n\
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
    assert_eq!(stdout.trim(), "int\nfloat", "{stdout}");
}

/// The `<` after a namespaced path is still the comparison operator: a type
/// argument list only opens when the whole list closes into a call.
#[test]
fn aliased_path_before_a_comparison_stays_a_comparison() {
    let output = run_program(
        "aliased_comparison",
        &[
            (
                "lib.cdl",
                "fn one() -> int { return 1; }\n\
                 fn first<T>(xs: T[]) -> T { return xs[0]; }\n",
            ),
            (
                "prog.cdl",
                "import \"lib.cdl\" as m;\n\
                 fn main() {\n\
                     let hi = 9;\n\
                     print(m::one() < hi);\n\
                     print(m::first<int>([2, 3]) < hi);\n\
                     print(hi < m::one() && hi > 0);\n\
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
    assert_eq!(stdout.trim(), "true\ntrue\nfalse", "{stdout}");
}

/// A generic type an aliased module declares is named through the alias
/// wherever a type is written: a parameter, a struct field, an array of them, a
/// return annotation, and a type argument of another generic type.
#[test]
fn aliased_generic_type_is_named_in_every_type_position() {
    let output = run_program(
        "aliased_generic_type_position",
        &[
            (
                "shapes.cdl",
                "enum Slot<T> { Filled(T), Empty }\n\
                 struct Cell<T> { value: T }\n\
                 impl Cell<T> { fn get(self) -> T { return self.value; } }\n",
            ),
            (
                "prog.cdl",
                "import \"shapes.cdl\" as g;\n\
                 struct Holder { one: g::Slot<int>, many: g::Slot<int>[] }\n\
                 fn wrap(n: int) -> g::Slot<int> { return g::Slot<int>::Filled(n); }\n\
                 fn peek(s: g::Slot<int>) -> int {\n\
                     match s {\n\
                         g::Slot<int>::Filled(v) => { return v; }\n\
                         g::Slot<int>::Empty => { return -1; }\n\
                     }\n\
                     return -1;\n\
                 }\n\
                 fn total(all: g::Slot<int>[]) -> int {\n\
                     let sum = 0;\n\
                     for s in all { sum = sum + peek(s); }\n\
                     return sum;\n\
                 }\n\
                 fn inner(c: g::Cell<g::Slot<int>>) -> int { return peek(c.get()); }\n\
                 fn main() {\n\
                     let h = Holder { one: wrap(9), many: [wrap(1), wrap(2)] };\n\
                     print(peek(h.one));\n\
                     print(total(h.many));\n\
                     print(inner(g::Cell<g::Slot<int>>{ value: wrap(7) }));\n\
                     print(peek(g::Slot<int>::Empty));\n\
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
    assert_eq!(stdout.trim(), "9\n3\n7\n-1", "{stdout}");
}

/// Two modules can each declare a generic type by the same name. The alias in
/// front of the name says which one is meant, in a type position, in a literal
/// and in a method call, and a value of one is not a value of the other.
#[test]
fn two_modules_can_each_declare_the_same_generic_name() {
    let output = run_program(
        "same_generic_name_two_modules",
        &[
            (
                "left.cdl",
                "struct Slot<T> { tag: string, held: T }\n                 impl Slot<T> { fn label(self) -> string { return self.tag; } }\n",
            ),
            (
                "right.cdl",
                "struct Slot<T> { held: T }\n                 impl Slot<T> { fn label(self) -> string { return \"right\"; } }\n",
            ),
            (
                "prog.cdl",
                "import \"left.cdl\" as a;\n                 import \"right.cdl\" as b;\n                 fn takeA(s: a::Slot<int>) -> string { return s.tag; }\n                 fn takeB(s: b::Slot<int>) -> int { return s.held; }\n                 fn main() {\n                     let left = a::Slot<int>{ tag: \"left\", held: 1 };\n                     let right = b::Slot<int>{ held: 2 };\n                     print(takeA(left));\n                     print(takeB(right));\n                     print(left.label());\n                     print(right.label());\n                 }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "left\n2\nleft\nright", "{stdout}");
}

/// One module's generic type is not the other's, so a function declared to
/// return `a::Slot<int>` cannot return `b::Slot<int>`. The two are told apart
/// by the module they come from.
#[test]
fn one_modules_generic_type_is_not_the_others() {
    let output = check_program(
        "same_generic_name_not_interchangeable",
        &[
            ("left.cdl", "struct Slot<T> { tag: string, held: T }\n"),
            ("right.cdl", "struct Slot<T> { held: T }\n"),
            (
                "prog.cdl",
                "import \"left.cdl\" as a;\n                 import \"right.cdl\" as b;\n                 fn mkB() -> a::Slot<int> { return b::Slot<int>{ held: 7 }; }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("left::Slot<int>") && stderr.contains("right::Slot<int>"),
        "stderr: {stderr}"
    );
}

/// The module path in front of a generic type has to name a module the file
/// imported; one that names nothing is reported instead of resolving to
/// whatever declared the name.
#[test]
fn aliased_generic_type_needs_a_module_that_exists() {
    let output = check_program(
        "aliased_generic_type_unknown_module",
        &[
            ("shapes.cdl", "enum Slot<T> { Filled(T), Empty }\n"),
            (
                "prog.cdl",
                "import \"shapes.cdl\" as g;\n\
                 fn peek(s: h::Slot<int>) -> int { return 0; }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unknown namespace") && stderr.contains('h'),
        "stderr: {stderr}"
    );
}

/// A variant belongs to its enum, so the alias alone does not name one: the
/// two-segment path is a function call, and there is no such function.
#[test]
fn aliased_variant_without_its_enum_is_unknown() {
    let output = check_program(
        "aliased_variant_bare",
        &[
            ("shapes.cdl", "enum Shape { Circle(int), Empty }\n"),
            (
                "prog.cdl",
                "import \"shapes.cdl\";\n\
                 fn build() { return shapes::Circle(1); }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot find function")
            && stderr.contains("Circle")
            && stderr.contains("in namespace"),
        "stderr: {stderr}"
    );
}

/// A namespaced module's own imports are its own: the items it named and the
/// modules it bound are reachable from its bodies, and a closure it builds
/// still resolves the function it calls.
#[test]
fn a_namespaced_module_keeps_its_own_imports() {
    let output = run_program(
        "aliased_nested",
        &[
            ("base.cdl", "fn twice(n) { return n * 2; }\n"),
            ("side.cdl", "fn triple(n) { return n * 3; }\n"),
            (
                "lib.cdl",
                "import \"base.cdl\" { twice };\n\
                 import \"side.cdl\";\n\
                 fn apply(f, n) { return f(n); }\n\
                 fn compute(n) { let step = fn(x) { return twice(x); }; return apply(step, side::triple(n)); }\n",
            ),
            (
                "prog.cdl",
                "import \"lib.cdl\";\nfn main() { print(lib::compute(2)); }\n",
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

/// A module that declares `main` can have its items imported: only the entry
/// file's `main` runs, so the module's is not one of the names it exports.
#[test]
fn importing_items_of_a_module_with_main_keeps_the_importers_main() {
    let output = run_program(
        "bare_module_main",
        &[
            (
                "helper.cdl",
                "fn ping() { return 5; }\nfn main() { print(ping()); }\n",
            ),
            (
                "prog.cdl",
                "import \"helper.cdl\" { ping };\nfn main() { print(ping() + 1); }\n",
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

/// An import holds back only the module's `main` function. A struct that
/// happens to carry the same name is a separate symbol and is imported like any
/// other; it used to be dropped with the function, leaving the importer with
/// "Unknown struct main".
///
/// Checked rather than run: functions and types share one namespace, so a
/// module's `struct main` still collides with the importing file's own `fn
/// main`. A library entry, which has none, is where the struct is reachable.
#[test]
fn an_import_keeps_a_struct_named_main() {
    let output = check_program(
        "bare_module_main_struct",
        &[
            (
                "helper.cdl",
                "struct main { a: int }\nfn ping() { return 5; }\nfn main() { print(ping()); }\n",
            ),
            (
                "prog.cdl",
                "import \"helper.cdl\" { main, ping };\nfn build() { let m = main { a: 7 }; return m.a + ping(); }\n",
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
/// form. `base.cdl` here is reached twice, once under its name in the entry
/// file and once by `mid.cdl`'s import of two of its items, and both routes
/// work: the entry calls into `base::`, and `mid`'s own bodies call the names
/// it named.
#[test]
fn a_module_reached_by_two_files_binds_in_both() {
    let output = run_program(
        "two_routes_alias",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "mid.cdl",
                "import \"base.cdl\" { Tag, tag };\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"base.cdl\";\n\
                 import \"mid.cdl\";\n\
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
                "import \"base.cdl\" { Tag, tag };\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"mid.cdl\";\n\
                 import \"base.cdl\";\n\
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

/// Naming the items of a module another import already reached brings them
/// into the importing file's own scope, so its functions and its struct's
/// method are all written unqualified.
#[test]
fn a_module_reached_by_name_and_through_another_brings_its_items() {
    let output = run_program(
        "two_routes_bare",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "mid.cdl",
                "import \"base.cdl\" { Tag, tag };\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
            ),
            (
                "prog.cdl",
                "import \"base.cdl\" { unwrap, cell, tag };\n\
                 import \"mid.cdl\";\n\
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

/// What a module imports stays its own: the items `mid.cdl` named from
/// `base.cdl` are not names of `mid`, bare or behind its namespace.
#[test]
fn a_modules_imports_do_not_reach_its_importer() {
    for (name, body) in [
        ("bare", "fn main() { print(tag(1)); }\n"),
        ("namespaced", "fn main() { print(mid::tag(1)); }\n"),
    ] {
        let output = run_program(
            &format!("own_imports_{name}"),
            &[
                ("base.cdl", BASE_MODULE),
                (
                    "mid.cdl",
                    "import \"base.cdl\" { Tag, tag };\nfn from_mid(n: int) -> Tag { return tag(n + 1); }\n",
                ),
                ("prog.cdl", &format!("import \"mid.cdl\";\n{body}")),
            ],
        );
        assert!(!output.status.success(), "{name} compiled");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Cannot find function") && stderr.contains("tag"),
            "{name}: {stderr}"
        );
    }
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
        "import \"shared\" { unwrap, tag, cell };\n\
         import \"./libs/shared.cdl\" { unwrap, tag, cell };\n\
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

/// Two modules can each declare a plain struct by the same name. The alias in
/// front of the name says which one is meant, and a method call on a value of
/// one reaches that module's method rather than the other module's.
#[test]
fn two_modules_can_each_declare_the_same_struct_name() {
    let output = run_program(
        "same_struct_name_two_modules",
        &[
            (
                "left.cdl",
                "struct Plain { tag: string }\n\
                 impl Plain { fn get(self) -> string { return self.tag; } }\n",
            ),
            (
                "right.cdl",
                "struct Plain { held: int }\n\
                 impl Plain { fn get(self) -> int { return self.held; } }\n",
            ),
            (
                "prog.cdl",
                "import \"left.cdl\" as a;\n\
                 import \"right.cdl\" as b;\n\
                 fn takeA(p: a::Plain) -> string { return p.tag; }\n\
                 fn takeB(p: b::Plain) -> int { return p.held; }\n\
                 fn main() {\n\
                     let left = a::Plain { tag: \"left\" };\n\
                     let right = b::Plain { held: 2 };\n\
                     print(takeA(left));\n\
                     print(takeB(right));\n\
                     print(left.get());\n\
                     print(right.get());\n\
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
    assert_eq!(stdout.trim(), "left\n2\nleft\n2", "{stdout}");
}

/// The same for plain enums: two modules each declaring `Tag` keep their own
/// variants and their own methods.
#[test]
fn two_modules_can_each_declare_the_same_enum_name() {
    let output = run_program(
        "same_enum_name_two_modules",
        &[
            (
                "left.cdl",
                "enum Tag { One, Two }\n\
                 impl Tag { fn name(self) -> string { return \"left\"; } }\n",
            ),
            (
                "right.cdl",
                "enum Tag { Three }\n\
                 impl Tag { fn name(self) -> string { return \"right\"; } }\n",
            ),
            (
                "prog.cdl",
                "import \"left.cdl\" as a;\n\
                 import \"right.cdl\" as b;\n\
                 fn takeA(t: a::Tag) -> string { return t.name(); }\n\
                 fn takeB(t: b::Tag) -> string { return t.name(); }\n\
                 fn main() {\n\
                     print(takeA(a::Tag::One));\n\
                     print(takeB(b::Tag::Three));\n\
                     print(a::Tag::Two.name());\n\
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
    assert_eq!(stdout.trim(), "left\nright\nleft", "{stdout}");
}

/// One module's `Plain` is not the other's, so a function declared to return
/// `a::Plain` cannot return `b::Plain`. The two are told apart by the module
/// they come from rather than both reading `Plain`.
#[test]
fn a_mismatch_between_same_named_structs_names_both_modules() {
    let output = check_program(
        "same_struct_name_mismatch",
        &[
            ("left.cdl", "struct Plain { tag: string }\n"),
            ("right.cdl", "struct Plain { held: int }\n"),
            (
                "prog.cdl",
                "import \"left.cdl\" as a;\n\
                 import \"right.cdl\" as b;\n\
                 fn mkB() -> a::Plain { return b::Plain { held: 7 }; }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("left::Plain") && stderr.contains("right::Plain"),
        "stderr: {stderr}"
    );
}

/// Two packages can each ship a module of the same file name declaring the
/// same type. The qualifier in a diagnostic takes as much of the path as it
/// takes to tell the two modules apart.
#[test]
fn same_named_modules_in_two_packages_declare_the_same_type() {
    let output = check_program(
        "same_struct_name_two_packages",
        &[
            ("left/types.cdl", "struct Plain { tag: string }\n"),
            ("right/types.cdl", "struct Plain { held: int }\n"),
            (
                "prog.cdl",
                "import \"left/types.cdl\" as a;\n\
                 import \"right/types.cdl\" as b;\n\
                 fn mkB() -> a::Plain { return b::Plain { held: 7 }; }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("left::types::Plain") && stderr.contains("right::types::Plain"),
        "stderr: {stderr}"
    );
}

/// Naming an item in an import brings it, and the methods of a type it names,
/// into the file's scope; the module itself is bound under no name.
#[test]
fn selective_import_brings_the_named_items() {
    let output = run_program(
        "selective",
        &[
            ("base.cdl", BASE_MODULE),
            (
                "prog.cdl",
                "import \"base.cdl\" { Cell, cell, tag, unwrap };\n\
                 fn main() {\n\
                     let c = Cell { n: 4 };\n\
                     print(c.doubled() + cell(1).doubled() + unwrap(tag(3)));\n\
                 }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "13");
}

/// An item a module does not declare is an error that lists what it does.
#[test]
fn selective_import_of_an_unknown_item_lists_the_exports() {
    let output = run_program(
        "selective_unknown",
        &[
            ("base.cdl", BASE_MODULE),
            ("prog.cdl", "import \"base.cdl\" { cel };\nfn main() { }\n"),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("import_item_not_exported")
            || (stderr.contains("declares no") && stderr.contains("cel")),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("Tag, Cell, tag, cell, unwrap"),
        "stderr: {stderr}"
    );
}

/// A call to a name the file reaches only through a module's namespace is an
/// error whose help names both ways to write it.
#[test]
fn a_name_behind_a_namespace_gets_help_naming_both_forms() {
    let output = run_program(
        "behind_namespace",
        &[
            ("helper.cdl", "fn ping() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"helper.cdl\";\nfn main() { print(ping()); }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("helper::ping") && stderr.contains("import \"helper.cdl\" { ping };"),
        "stderr: {stderr}"
    );
}

/// The same for a type written bare in front of `::` or in a struct literal.
#[test]
fn a_type_behind_a_namespace_gets_help_naming_both_forms() {
    for (name, body) in [
        ("literal", "fn main() { print(Cell { n: 1 }.n); }\n"),
        ("path", "fn main() { print(Cell::nothing()); }\n"),
    ] {
        let output = run_program(
            &format!("type_behind_namespace_{name}"),
            &[
                ("base.cdl", BASE_MODULE),
                ("prog.cdl", &format!("import \"base.cdl\";\n{body}")),
            ],
        );
        assert!(!output.status.success(), "{name} compiled");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("base::Cell") && stderr.contains("import \"base.cdl\" { Cell };"),
            "{name}: {stderr}"
        );
    }
}

/// A path whose last segment is not a name has to be given one with `as`.
#[test]
fn a_path_that_ends_in_no_name_needs_as() {
    let output = run_program(
        "not_a_name",
        &[
            ("my-lib.cdl", "fn ping() { return 5; }\n"),
            ("prog.cdl", "import \"my-lib.cdl\";\nfn main() { }\n"),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("my-lib") && stderr.contains("as"),
        "stderr: {stderr}"
    );

    let output = run_program(
        "not_a_name_as",
        &[
            ("my-lib.cdl", "fn ping() { return 5; }\n"),
            (
                "prog.cdl",
                "import \"my-lib.cdl\" as my_lib;\nfn main() { print(my_lib::ping()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "5");
}

/// Two imports that would bind one name to two modules are an error; the same
/// module bound twice under one name is not.
#[test]
fn two_modules_under_one_name_collide() {
    let output = run_program(
        "name_collision",
        &[
            ("geo.cdl", "fn ping() { return 1; }\n"),
            ("sub/geo.cdl", "fn ping() { return 2; }\n"),
            (
                "prog.cdl",
                "import \"geo.cdl\";\nimport \"./sub/geo.cdl\";\nfn main() { }\n",
            ),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("geo") && stderr.contains("sub/geo.cdl") && stderr.contains("as"),
        "stderr: {stderr}"
    );

    let output = run_program(
        "name_twice",
        &[
            ("geo.cdl", "fn ping() { return 1; }\n"),
            (
                "prog.cdl",
                "import \"geo.cdl\";\nimport \"./geo.cdl\";\nfn main() { print(geo::ping()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An `impl` block on a builtin type in a module bound under a namespace adds
/// methods that resolve on the receiver with no prefix.
#[test]
fn builtin_type_methods_of_a_namespaced_module_need_no_prefix() {
    let output = run_program(
        "builtin_impl",
        &[
            (
                "shout.cdl",
                "impl string { fn shout(self) -> string => self + \"!\"; }\n",
            ),
            (
                "prog.cdl",
                "import \"shout.cdl\";\nfn main() { print(\"hi\".shout()); }\n",
            ),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hi!");
}

/// A name and a list of items on one import is a parse error, and so is an
/// empty list.
#[test]
fn an_import_takes_a_name_or_items_not_both() {
    for (name, import) in [
        ("both", "import \"geo.cdl\" as g { ping };"),
        ("empty", "import \"geo.cdl\" {};"),
    ] {
        let output = run_program(
            &format!("import_shape_{name}"),
            &[
                ("geo.cdl", "fn ping() { return 1; }\n"),
                ("prog.cdl", &format!("{import}\nfn main() {{ }}\n")),
            ],
        );
        assert!(!output.status.success(), "{name} compiled");
    }
}
