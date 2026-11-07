//! Integration tests for where a `dylib` import looks for its library file.
//!
//! A library is looked for beside the importing file, which is wrong for an
//! application that keeps its sources in one directory and its native libraries
//! in another. Such a host names the library directories with
//! `set_dylib_dirs`, and both a compile and a `.cdlb` load look there first.
//!
//! The library under test is a copy of the system zlib, renamed so that nothing
//! but a searched directory can turn it up. Where no zlib file can be copied
//! (macOS keeps it in the dyld cache rather than on disk) the tests skip.

mod built;
mod common;

use candela::Engine;
use candela::HostRegistry;
use candela::LoadError;
use candela::build_bytecode;
use candela::load_program;
use candela::set_dylib_dirs;
use candela_vm::rt::TargetOs;
use candela_vm::rt::resolve_library_filename;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Calls one zlib symbol through a logical `dylib` import. The name is
/// `fixture`, so the OS loader's own search path can never satisfy it.
const PROGRAM: &str = "dylib \"fixture\" {
    string zlibVersion();
}

fn main() {
    print(fixture::zlibVersion());
}
";

/// A unique scratch directory under the system temp dir.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela_dylib_dir_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// What `dylib "fixture"` maps to on this platform.
fn fixture_filename() -> String {
    resolve_library_filename("fixture", TargetOs::CURRENT)
}

/// This platform's dynamic-library extension, with no leading dot.
const fn extension() -> &'static str {
    TargetOs::CURRENT.dynamic_lib_extension()
}

/// Copies the system zlib into `dir` under the fixture name, and hands back the
/// copy once it opens. `None` means this machine has no zlib file to copy, and
/// the test that asked for it has nothing to run against.
fn install_fixture(dir: &Path) -> Option<PathBuf> {
    let destination = dir.join(fixture_filename());
    for source in zlib_files() {
        if std::fs::copy(&source, &destination).is_ok()
            && unsafe { libloading::Library::new(&destination) }.is_ok()
        {
            return Some(destination);
        }
    }
    eprintln!("skipping: no copyable zlib on this machine");
    None
}

/// Every zlib file in the usual system library directories.
fn zlib_files() -> Vec<PathBuf> {
    let directories = [
        "/usr/lib/x86_64-linux-gnu",
        "/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/lib",
        "C:/Windows/System32",
    ];
    let mut found = Vec::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if is_zlib(&name) && entry.path().is_file() {
                found.push(entry.path());
            }
        }
    }
    found
}

/// Whether `name` is a zlib library file, versioned or not.
fn is_zlib(name: &str) -> bool {
    match TargetOs::CURRENT {
        TargetOs::Windows => name.eq_ignore_ascii_case("zlib1.dll"),
        TargetOs::Macos => {
            name.starts_with("libz.")
                && Path::new(name)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dylib"))
        }
        TargetOs::Linux => name.starts_with("libz.so"),
    }
}

/// The library sits in a directory of its own, two levels away from the script.
/// The compile finds it once the host names that directory, and does not
/// without one.
#[test]
fn a_named_directory_is_searched() {
    let root = scratch_dir("named");
    let lib = root.join("lib");
    let script = root.join("src").join("app.cdl");
    std::fs::create_dir_all(&lib).expect("create lib dir");
    std::fs::create_dir_all(script.parent().unwrap()).expect("create src dir");
    let Some(_fixture) = install_fixture(&lib) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };

    let engine = Engine::new();
    let filename = script.to_str().expect("scratch path is utf-8");

    // Nothing beside the script, nothing in the working directory, and no
    // system library by that name: the import has nowhere to resolve.
    let error = engine
        .compile(PROGRAM, filename)
        .err()
        .expect("an unfindable library fails the compile");
    assert_eq!(error.code, "cannot_load_dynlib");

    // Named, the directory is where the library is found.
    let previous = set_dylib_dirs(vec![lib]);
    let compiled = engine.compile(PROGRAM, filename);
    set_dylib_dirs(previous);
    assert!(
        compiled.is_ok(),
        "the named directory must satisfy the import: {:?}",
        compiled.err()
    );

    std::fs::remove_dir_all(&root).ok();
}

/// With no directory named, a library beside the importing file is still what
/// the import resolves to.
#[test]
fn the_script_s_own_directory_still_resolves() {
    let root = scratch_dir("beside");
    let Some(_fixture) = install_fixture(&root) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };

    let script = root.join("app.cdl");
    let engine = Engine::new();
    let compiled = engine.compile(PROGRAM, script.to_str().expect("scratch path is utf-8"));
    assert!(
        compiled.is_ok(),
        "a library beside the script must resolve with no directory named: {:?}",
        compiled.err()
    );

    std::fs::remove_dir_all(&root).ok();
}

/// A path import is relative to the named directory too, so a program can point
/// into a subdirectory of it.
#[test]
fn a_path_import_is_relative_to_the_named_directory() {
    let root = scratch_dir("path");
    let nested = root.join("lib").join("native");
    std::fs::create_dir_all(&nested).expect("create native dir");
    let Some(fixture) = install_fixture(&nested) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };
    // A path import carries no `lib` prefix, so the file is named for the path
    // as written.
    std::fs::rename(&fixture, nested.join(format!("plug.{}", extension()))).expect("rename");

    let program = "dylib \"native/plug\" {
    string zlibVersion();
}

fn main() {
    print(plug::zlibVersion());
}
";
    let engine = Engine::new();
    let script = root.join("src").join("app.cdl");
    let filename = script.to_str().expect("scratch path is utf-8");

    assert_eq!(
        engine
            .compile(program, filename)
            .err()
            .expect("an unfindable library fails the compile")
            .code,
        "cannot_load_dynlib"
    );

    let previous = set_dylib_dirs(vec![root.join("lib")]);
    let compiled = engine.compile(program, filename);
    set_dylib_dirs(previous);
    assert!(
        compiled.is_ok(),
        "the path must resolve under the named directory: {:?}",
        compiled.err()
    );

    std::fs::remove_dir_all(&root).ok();
}

/// A `dylib` block is a namespace, reached with `::`. A dot on the block's
/// name is a compile error whose help names the `::` spelling.
#[test]
fn a_dot_on_the_library_namespace_is_refused() {
    let root = scratch_dir("dot");
    let Some(_fixture) = install_fixture(&root) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };

    let source = "dylib \"fixture\" {
    string zlibVersion();
}

fn main() { print(fixture.zlibVersion()); }
";
    let script = root.join("app.cdl");
    let err = Engine::new()
        .compile(source, script.to_str().expect("scratch path is utf-8"))
        .err()
        .expect("a dot on a block does not compile");
    assert_eq!(err.code, "block_dot_call");
    assert!(
        err.message.contains("fixture::zlibVersion"),
        "{}",
        err.message
    );

    std::fs::remove_dir_all(&root).ok();
}

/// A `.cdlb` records the library by name and re-opens it at load, so the load
/// honors the named directory the same way the compile did.
#[test]
fn an_artifact_load_searches_the_named_directory() {
    let root = scratch_dir("artifact");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).expect("create lib dir");
    let Some(_fixture) = install_fixture(&lib) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };

    let script = root.join("src").join("app.cdl");
    let filename = script.to_str().expect("scratch path is utf-8").to_owned();

    let previous = set_dylib_dirs(vec![lib]);
    let bytes = build_bytecode(
        PROGRAM.to_owned(),
        &filename,
        &candela::ImportResolver::new(),
    )
    .expect("builds to bytecode");
    let loaded = load_program(&bytes, &HostRegistry::new());
    set_dylib_dirs(previous);
    assert!(
        loaded.is_ok(),
        "the artifact must re-open the library from the named directory"
    );

    // The recipe carries the library's name, not its location, so the same
    // artifact has nothing to open once the directory is no longer named.
    match load_program(&bytes, &HostRegistry::new()) {
        Err(LoadError::LibraryOpen { spec, filename, .. }) => {
            assert_eq!(spec, "fixture");
            assert_eq!(filename, fixture_filename());
        }
        Err(other) => panic!("expected a library-open failure, got {other:?}"),
        Ok(_) => panic!("the library must not resolve with no directory named"),
    }

    std::fs::remove_dir_all(&root).ok();
}

/// The `candela-vm` binary, built for this run; see `built`.
fn candela_vm() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            built::build(
                &["-p", "candela-vm", "--bin", "candela-vm"],
                &format!("candela-vm{}", std::env::consts::EXE_SUFFIX),
            )
        })
        .clone()
}

/// `candela-vm` looks for a plainly named library beside the artifact, the way
/// `candela run` looks beside the source, from whatever directory it is run
/// in, and never in the working directory.
#[test]
fn candela_vm_finds_a_library_beside_the_artifact() {
    let root = scratch_dir("beside");
    let app = root.join("app");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&app).expect("create app dir");
    std::fs::create_dir_all(&elsewhere).expect("create working dir");
    let Some(fixture) = install_fixture(&app) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };
    let script = app.join("app.cdl");
    let bytes = build_bytecode(
        PROGRAM.to_owned(),
        script.to_str().expect("scratch path is utf-8"),
        &candela::ImportResolver::new(),
    )
    .expect("builds to bytecode");
    let artifact = app.join("app.cdlb");
    std::fs::write(&artifact, bytes).expect("write the artifact");

    let run = |cwd: &Path, artifact: &Path| {
        common::output_with_deadline(
            Command::new(candela_vm()).arg(artifact).current_dir(cwd),
            "candela-vm",
        )
    };
    let out = run(&elsewhere, &artifact);
    assert!(
        out.status.success(),
        "the library beside the artifact must load: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.stdout.is_empty(), "the program prints zlib's version");

    // The same library in the working directory and not beside the artifact
    // is not found, and the report carries the loader's own reason.
    std::fs::rename(&fixture, elsewhere.join(fixture_filename())).expect("move the library");
    let out = run(&elsewhere, &artifact);
    assert!(
        !out.status.success(),
        "the working directory is never searched"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot open dynamic library 'fixture'"),
        "{stderr}"
    );
    if cfg!(unix) {
        assert!(
            stderr.contains(&fixture_filename()) && !stderr.trim_end().ends_with("dlopen failed"),
            "the loader's message comes through: {stderr}"
        );
    }

    std::fs::remove_dir_all(&root).ok();
}

/// A relative path in a `dylib` block resolves against the directories an
/// artifact load searches, never against the working directory.
#[test]
fn an_artifact_resolves_a_relative_path_beside_itself() {
    let root = scratch_dir("relative");
    let native = root.join("app").join("native");
    std::fs::create_dir_all(&native).expect("create native dir");
    let Some(_fixture) = install_fixture(&native) else {
        std::fs::remove_dir_all(&root).ok();
        return;
    };
    // A path form is bound under its file name up to the first dot.
    let filename = fixture_filename();
    let namespace = filename.split('.').next().expect("a file name");
    let program = format!(
        "dylib \"./native/{filename}\" {{\n    string zlibVersion();\n}}\n\nfn main() {{\n    print({namespace}::zlibVersion());\n}}\n"
    );
    let script = root.join("app").join("app.cdl");
    let bytes = build_bytecode(
        program,
        script.to_str().expect("scratch path is utf-8"),
        &candela::ImportResolver::new(),
    )
    .expect("builds to bytecode");
    let artifact = root.join("app").join("app.cdlb");
    std::fs::write(&artifact, &bytes).expect("write the artifact");

    let out = common::output_with_deadline(
        Command::new(candela_vm()).arg(&artifact).current_dir(&root),
        "candela-vm",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The same artifact elsewhere, run from the directory that holds
    // `native/`, does not resolve the path against the working directory.
    let other = root.join("other");
    std::fs::create_dir_all(&other).expect("create other dir");
    let moved = other.join("app.cdlb");
    std::fs::write(&moved, &bytes).expect("write the moved artifact");
    let out = common::output_with_deadline(
        Command::new(candela_vm())
            .arg(&moved)
            .current_dir(root.join("app")),
        "candela-vm",
    );
    assert!(
        !out.status.success(),
        "the working directory is never searched"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no such file under"), "{stderr}");
    std::fs::remove_dir_all(&root).ok();
}

/// `candela check` opens no library, so a program whose C library a build step
/// has not produced yet checks; `candela build` still needs it.
#[test]
fn check_does_not_need_the_library() {
    let dir = scratch_dir("check");
    let path = dir.join("app.cdl");
    std::fs::write(&path, PROGRAM).expect("write program");

    let check = std::process::Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("candela runs");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );

    let build = std::process::Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg("build")
        .arg(&path)
        .output()
        .expect("candela runs");
    assert!(!build.status.success(), "a build opens the library");
    std::fs::remove_dir_all(&dir).ok();
}
