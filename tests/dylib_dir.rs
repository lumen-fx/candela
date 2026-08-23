//! Integration tests for where a `dylib` import looks for its library file.
//!
//! A library is looked for beside the importing file, which is wrong for an
//! application that keeps its sources in one directory and its native libraries
//! in another. Such a host names the library directory with `set_dylib_dir`,
//! and both a compile and a `.cdlb` load look there first.
//!
//! The library under test is a copy of the system zlib, renamed so that nothing
//! but a searched directory can turn it up. Where no zlib file can be copied
//! (macOS keeps it in the dyld cache rather than on disk) the tests skip.

use candela::Engine;
use candela::HostRegistry;
use candela::LoadError;
use candela::build_bytecode;
use candela::load_program;
use candela::set_dylib_dir;
use candela_vm::rt::TargetOs;
use candela_vm::rt::resolve_library_filename;
use std::path::Path;
use std::path::PathBuf;

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
    let previous = set_dylib_dir(Some(lib));
    let compiled = engine.compile(PROGRAM, filename);
    set_dylib_dir(previous);
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

    let previous = set_dylib_dir(Some(root.join("lib")));
    let compiled = engine.compile(program, filename);
    set_dylib_dir(previous);
    assert!(
        compiled.is_ok(),
        "the path must resolve under the named directory: {:?}",
        compiled.err()
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

    let previous = set_dylib_dir(Some(lib));
    let bytes = build_bytecode(PROGRAM.to_owned(), &filename).expect("builds to bytecode");
    let loaded = load_program(&bytes, &HostRegistry::new());
    set_dylib_dir(previous);
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
