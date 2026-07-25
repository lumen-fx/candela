//! Integration tests for the `.cdlb` bytecode artifact round-trip.
//!
//! The fat `candela` binary compiles source to a `.cdlb` artifact
//! (`build_bytecode`); the lean `candela-vm` binary loads it (`load_program`)
//! and runs it. These tests exercise the serialize -> deserialize half of that
//! path (the run half, and output equality with the fat binary, are covered by
//! the CLI round-trip). They guard the artifact format's magic/version header
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
