//! Running a first-party package from candela in a test.
//!
//! [`stage`] copies a package into a scratch directory with the native
//! library the test build made beside it, as `dist/<name>.<ext>`, which is
//! where the package's `dylib` block looks. [`run`] compiles a program that
//! imports it, then runs event handlers the way an embedding host does until
//! the program has reported what the test waits for.
//!
//! The program reports through `test::seen(text)`, a host function that
//! records each call, so a test reads what the handlers saw as a list of
//! strings.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use candela::Engine;

/// How long [`run`] waits for the program to report what the test expects.
pub const DEADLINE: Duration = Duration::from_secs(20);

/// The directory cargo built this test binary's crates in.
fn build_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test binary has a path");
    // target/<profile>/deps/<test binary>
    exe.parent()
        .and_then(Path::parent)
        .expect("the test binary sits in target/<profile>/deps")
        .to_path_buf()
}

/// The repository's `libs`, where the standard library is.
fn std_libs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../libs")
}

/// A fresh scratch directory for one test.
#[must_use]
pub fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela-package-{tag}-{}-{}",
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
    for entry in std::fs::read_dir(from).expect("read a package directory") {
        let entry = entry.expect("read a package entry");
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("copy a package file");
        }
    }
}

/// Copies the package at `package` into `into/<name>`, with the native library
/// cargo built for it, `candela_<name>`, as `dist/<name>.<ext>` when the
/// package has a native half. Answers the staged package root.
///
/// # Panics
///
/// When the library is not where cargo leaves it, which means the test did
/// not build the package's native crate.
#[must_use]
pub fn stage(package: &Path, name: &str, into: &Path) -> PathBuf {
    let root = into.join(name);
    std::fs::create_dir_all(&root).expect("create the staged package");
    std::fs::copy(package.join("candela.toml"), root.join("candela.toml"))
        .expect("copy the manifest");
    copy_dir(&package.join("src"), &root.join("src"));
    if package.join("web").is_dir() {
        copy_dir(&package.join("web"), &root.join("web"));
    }
    if package.join("native").is_dir() {
        let (prefix, ext) = if cfg!(windows) {
            ("", "dll")
        } else if cfg!(target_os = "macos") {
            ("lib", "dylib")
        } else {
            ("lib", "so")
        };
        let built = build_dir().join(format!("{prefix}candela_{name}.{ext}"));
        assert!(
            built.is_file(),
            "{} is not built; build the package's native crate first",
            built.display()
        );
        std::fs::create_dir_all(root.join("dist")).expect("create dist");
        std::fs::copy(&built, root.join("dist").join(format!("{name}.{ext}")))
            .expect("copy the native library");
    }
    root
}

/// The import line for the staged package at `root`, by the path of its
/// entry, so no registry is involved.
#[must_use]
pub fn import(root: &Path, name: &str) -> String {
    let entry = root.join("src").join(format!("{name}.cdl"));
    format!("import \"{}\";", entry.to_string_lossy().replace('\\', "/"))
}

/// Compiles `program` as `<dir>/app.cdl`, runs its `main` and then its
/// `start` function, and runs event handlers until `done` says the reports
/// so far are what the test waits for. Answers every report `test::seen`
/// received, in order.
///
/// `program` declares `host "test" { seen(string); }` to report through.
///
/// # Panics
///
/// When the program does not compile, a handler raises, or [`DEADLINE`]
/// passes before `done` is satisfied.
pub fn run(dir: &Path, program: &str, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut engine = Engine::new().with_lib_dir(std_libs());
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
