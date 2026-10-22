//! Builds what a suite runs that its own package does not build: the
//! `candela-vm` binary and the first-party packages' native libraries.
//!
//! `cargo test` builds the `candela` binary for this package's tests, but not
//! a binary or a library of another workspace member, and a workspace run
//! does not promise to have built one before the tests start. So a test that
//! needs one builds it here, with a nested cargo, into a directory of its own
//! under the target directory, so the nested build never waits on the outer
//! one. A warm tree makes it a no-op check.
//!
//! Under `cargo llvm-cov` the nested build is instrumented like everything
//! else, through the inherited environment, and lands inside the profile
//! directory the report reads objects from, so what it runs is measured.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The directory a nested build writes to: under the coverage target's
/// `debug` profile during `cargo llvm-cov`, whose location the inherited
/// `LLVM_PROFILE_FILE` gives away, and under the target directory otherwise.
fn nested_target_dir() -> PathBuf {
    if std::env::var_os("CARGO_LLVM_COV").is_some()
        && let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE")
        && let Some(root) = Path::new(&profile).parent()
    {
        return root.join("debug").join("nested");
    }
    // The test binary sits in <target>/<profile>/deps.
    let exe = std::env::current_exe().expect("the test binary has a path");
    exe.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("the test binary sits in <target>/<profile>/deps")
        .join("nested")
}

/// Builds `file` with `cargo build` and `args`, into the nested target
/// directory, and answers where it landed: `debug/<file>` there.
pub fn build(args: &[&str], file: &str) -> PathBuf {
    let target = nested_target_dir();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("build")
        .args(args)
        .arg("--target-dir")
        .arg(&target)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "cargo build {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = target.join("debug").join(file);
    assert!(path.is_file(), "{} was not built", path.display());
    path
}
