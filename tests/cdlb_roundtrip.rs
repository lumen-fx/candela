//! Integration tests for the `.cdlb` bytecode artifact round-trip.
//!
//! The full `candela` toolchain (compiler + VM) compiles source to a `.cdlb`
//! artifact (`build_bytecode`); the VM-only `candela-vm` binary loads it
//! (`load_program`) and runs it. These tests exercise the serialize ->
//! deserialize half of that path (the run half, and output equality with the
//! full binary, are covered by the CLI round-trip). They guard the artifact
//! format's magic/version header
//! and its ability to carry instructions, the constant pools, structs, and
//! sources.

use candela::load_program;

const STRUCT_PROGRAM: &str = "
struct Point { x: int, y: int }

fn dist_sq(p) {
    return p.x * p.x + p.y * p.y;
}

fn main() {
    let pts = [Point { x: 3, y: 4 }, Point { x: 1, y: 2 }];
    for p in pts {
        print(dist_sq(p));
    }
    let s = \"Hello, Candela\";
    print(s.uppercase());
}
";

#[test]
fn bytecode_round_trips_through_load() {
    let bytes = candela::build_bytecode(STRUCT_PROGRAM.to_owned(), "struct_program.cdl")
        .expect("program with structs/arrays/strings should compile to bytecode");

    // Header: 4-byte magic + 1 version byte.
    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");
    assert!(bytes.len() > 5, "artifact must carry a serialized body");

    // The lean loader accepts the artifact and reconstructs a runnable program.
    assert!(
        load_program(&bytes).is_ok(),
        "freshly built artifact must load"
    );
}

/// A program that defines and calls `impl` methods. Methods lower to ordinary
/// mangled free-function calls in the bytecode, so this must compile, load, and
/// run through the VM-only `candela-vm` path with NO runtime changes.
const METHOD_PROGRAM: &str = "
struct Point { x: int, y: int }
struct Counter { n: int }

impl Point {
    fn len(self) { return self.x + self.y; }
    fn scaled(self, factor) { return Point { x: self.x * factor, y: self.y * factor }; }
}

impl Counter {
    fn inc(self) { return Counter { n: self.n + 1 }; }
    fn get(self) { return self.n; }
}

fn main() {
    let p = Point { x: 2, y: 3 };
    print(p.len());
    print(p.scaled(3).len());
    let c = Counter { n: 0 };
    print(c.inc().inc().get());
}
";

#[test]
fn method_program_round_trips_and_runs() {
    // Build the artifact with the full `candela` toolchain (compiler + VM).
    let bytes = candela::build_bytecode(METHOD_PROGRAM.to_owned(), "methods.cdl")
        .expect("a program using impl methods should compile to bytecode");

    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");

    // Load it with the lean loader and run it exactly as `candela-vm` does
    // (`candela-vm`'s whole job is `load_program(..).run()`). Methods are just
    // ordinary calls in the bytecode, so this executes to completion unchanged.
    let mut program = load_program(&bytes).expect("method artifact must load on the VM-only path");
    program.run();
}

#[test]
fn empty_main_round_trips() {
    let bytes = candela::build_bytecode("fn main() {}".to_owned(), "empty.cdl").expect("compiles");
    assert!(load_program(&bytes).is_ok(), "empty program must load");
}

#[test]
fn bad_magic_is_rejected() {
    assert!(matches!(
        load_program(b"NOPE\x01garbage"),
        Err(candela::LoadError::BadMagic)
    ));
}

#[test]
fn truncated_is_rejected() {
    assert!(matches!(
        load_program(b"CD"),
        Err(candela::LoadError::Truncated)
    ));
}

#[test]
fn unknown_version_is_rejected() {
    // Correct magic, but a version byte this runtime does not understand.
    assert!(matches!(
        load_program(b"CDLB\xff"),
        Err(candela::LoadError::UnsupportedVersion(0xff))
    ));
}
