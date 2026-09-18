//! Scratch directories and fixture libraries for the FFI tests.
//!
//! The fixtures build with `rustc`, which every `cargo test` run has on its
//! path, so an FFI test needs no C toolchain.

use candela_vm::rt::TargetOs;
use candela_vm::rt::resolve_library_filename;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// A unique scratch directory under the system temp dir, named for the test
/// family in `prefix` and the case in `tag`.
pub fn scratch_dir(prefix: &str, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela_ffi_{prefix}_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Compiles `source` into a cdylib named `name` inside `dir` and answers where
/// it landed.
pub fn build_cdylib(dir: &Path, name: &str, source: &str) -> PathBuf {
    let source_path = dir.join("fixture.rs");
    std::fs::write(&source_path, source).expect("write fixture source");
    let library = dir.join(resolve_library_filename(name, TargetOs::CURRENT));
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
