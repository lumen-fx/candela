//! The base directory an embedder sets: `std/fs` resolves a relative path
//! against it instead of the working directory.
//!
//! The setting is the process's, so this file is a test binary of its own.

use candela::Engine;
use candela::Value;
use std::path::PathBuf;

#[test]
fn a_relative_path_resolves_against_the_base_directory() {
    let base = std::env::temp_dir().join(format!("candela_base_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("create the base directory");

    let previous = candela::set_base_dir(Some(base.clone()));
    assert_eq!(previous, None);
    assert_eq!(candela::base_dir(), Some(base.clone()));

    let mut program = Engine::new()
        .with_lib_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("libs"))
        .compile(
            r#"
import "std/fs";

fn save(text: string) -> string {
    fs::write("one.txt", text);
    return fs::read("one.txt");
}

fn main() {}
"#,
            "base.cdl",
        )
        .expect("compiles");
    let read = program
        .call("save", &[Value::String("kept".into())])
        .expect("save runs");
    assert_eq!(read, Value::String("kept".into()));
    assert_eq!(
        std::fs::read_to_string(base.join("one.txt")).expect("the file is under the base"),
        "kept"
    );

    candela::set_base_dir(None);
    assert_eq!(candela::base_dir(), None);
    let _ = std::fs::remove_dir_all(&base);
}
