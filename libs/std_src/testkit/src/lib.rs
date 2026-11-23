//! Running the standard library's native libraries from candela in a test.
//!
//! [`stage`] builds native libraries and lays out a library directory with
//! them where the std modules' `dylib` blocks look. [`run`] compiles a program
//! against that directory, then runs event handlers the way an embedding host
//! does until the program has reported what the test waits for.
//!
//! The program reports through `test::seen(text)`, a host function that
//! records each call, so a test reads what the handlers saw as a list of
//! strings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use candela::Engine;

/// How long [`run`] waits for the program to report what the test expects.
pub const DEADLINE: Duration = Duration::from_secs(20);

/// The directory a nested build writes to: under the coverage target's
/// `debug` profile during `cargo llvm-cov`, whose location the inherited
/// `LLVM_PROFILE_FILE` gives away, so the report maps what the library ran;
/// under the target directory otherwise. Its own directory either way, so
/// the nested cargo never waits on the one running the tests.
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

/// Builds the native library `name`, the crate `candela-std-<name>`, and
/// answers where it landed.
///
/// A workspace test run does not promise to have built another member's
/// cdylib before the tests start, so the first test that loads a library
/// builds it, except during `cargo llvm-cov` when the run has built it
/// already (see `run_library`). Later tests in the process reuse that build: a second cargo
/// run replaces the library file, and a test reading it meanwhile finds
/// nothing there.
///
/// # Panics
///
/// When the build fails.
#[must_use]
pub fn build_library(name: &str) -> PathBuf {
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let mut built = BUILT
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(path) = built.get(name) {
        return path.clone();
    }
    let path = build_once(name);
    built.insert(name.to_owned(), path.clone());
    path
}

/// During `cargo llvm-cov`, the library the test run itself built for `name`,
/// when there is one.
///
/// Building a package's tests builds its library as a cdylib in the same
/// compiler run as the rlib the tests link, so the two carry the same
/// coverage records. A nested build compiles the crate again with the
/// dependency features of one package rather than the whole workspace, and
/// its records describe the same functions under a different hash; the
/// report keeps one record per exported function, the first it reads, so the
/// calls the library answered would go unseen. Loading the run's own build
/// gives every copy one hash, and the counts add up.
///
/// The build directory keeps a unit per set of features it was ever built
/// with, so of several the newest is the one this run made.
fn run_library(name: &str) -> Option<PathBuf> {
    std::env::var_os("CARGO_LLVM_COV")?;
    let file = format!(
        "{}candela_std_{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    // The test binary sits in <build>/<profile>/build/<package>/<hash>/out.
    let exe = std::env::current_exe().ok()?;
    let units = exe
        .parent()?
        .parent()?
        .parent()?
        .parent()?
        .join(format!("candela-std-{name}"));
    std::fs::read_dir(units)
        .ok()?
        .filter_map(Result::ok)
        .map(|unit| unit.path().join("out").join(&file))
        .filter_map(|path| Some((std::fs::metadata(&path).ok()?.modified().ok()?, path)))
        .max()
        .map(|(_, path)| path)
}

fn build_once(name: &str) -> PathBuf {
    if let Some(path) = run_library(name) {
        return path;
    }
    let target = nested_target_dir();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let out = Command::new(cargo)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
        .args(["build", "-p", &format!("candela-std-{name}")])
        .arg("--target-dir")
        .arg(&target)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "building candela-std-{name} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = target.join("debug").join(format!(
        "{}candela_std_{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(path.is_file(), "{} was not built", path.display());
    path
}

/// The repository's `libs`, where the standard library is.
fn std_libs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh scratch directory for one test.
#[must_use]
pub fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela-std-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("create the scratch directory");
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create a staged directory");
    for entry in std::fs::read_dir(from).expect("read a library directory") {
        let entry = entry.expect("read a library entry");
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("copy a library file");
        }
    }
}

/// Lays out a library directory under `into/libs` the way an install does:
/// the standard library's modules in `std/`, and each native library in
/// `names`, built by [`build_library`], as `std_src/<name>/<name>.<ext>`,
/// which is where the modules' `dylib` blocks look. Answers the directory.
#[must_use]
pub fn stage(names: &[&str], into: &Path) -> PathBuf {
    let libs = into.join("libs");
    copy_dir(&std_libs().join("std"), &libs.join("std"));
    let ext = std::env::consts::DLL_EXTENSION;
    for name in names {
        let built = build_library(name);
        let dir = libs.join("std_src").join(name);
        std::fs::create_dir_all(&dir).expect("create the native library directory");
        std::fs::copy(&built, dir.join(format!("{name}.{ext}"))).expect("copy the native library");
    }
    libs
}

/// Compiles `program` as `<dir>/app.cdl` against the library directory
/// `libs`, from [`stage`], runs its `main` and then its
/// `start` function, and runs event handlers until `done` says the reports
/// so far are what the test waits for. Answers every report `test::seen`
/// received, in order.
///
/// `program` declares `host "test" { fn seen(text: string); }` to report through.
///
/// # Panics
///
/// When the program does not compile, a handler raises, or [`DEADLINE`]
/// passes before `done` is satisfied.
pub fn run(
    libs: &Path,
    dir: &Path,
    program: &str,
    done: impl Fn(&[String]) -> bool,
) -> Vec<String> {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut engine = Engine::new().with_lib_dir(libs.to_path_buf());
    let sink = Arc::clone(&seen);
    engine.register_host_fn("test", "seen", move |text: &str| {
        sink.lock().expect("the report list").push(text.to_owned());
    });
    let script = dir.join("app.cdl");
    let mut compiled = engine
        .compile(program, script.to_str().expect("the scratch path is utf-8"))
        .unwrap_or_else(|e| panic!("the program does not compile: {}", e.message));
    compiled
        .call("start", &[])
        .unwrap_or_else(|e| panic!("start failed: {}", e.message));
    let deadline = Instant::now() + DEADLINE;
    loop {
        compiled
            .dispatch_events(64)
            .unwrap_or_else(|e| panic!("a handler raised: {}", e.message));
        let now = seen.lock().expect("the report list").clone();
        if done(&now) {
            return now;
        }
        assert!(
            Instant::now() < deadline,
            "the program did not finish in {DEADLINE:?}; it reported {now:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
