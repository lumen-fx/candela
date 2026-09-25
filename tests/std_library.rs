//! Integration tests for the Candela standard library shipped under `libs/std`.
//!
//! Behavior is checked by running each module's `libs/std/tests/*.cdl` file
//! through the `candela` binary: the file uses `import "std/<module>"` and prints
//! an "ok" line, so a zero exit with that line means every check held. These
//! tests point `CANDELA_LIB_PATH` at this checkout's `libs` directory, since the
//! test binary is not laid out like an install.
//!
//! One test instead builds an install-style layout (the binary with `libs/`
//! beside it) and runs from an unrelated directory with nothing set, to confirm
//! the default exe-relative resolution needs no environment.

use candela::HostRegistry;
use candela::{build_bytecode, load_program};

mod common;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `libs/std/tests/<name>.cdl` through the `candela` binary with the library
/// path pointed at this checkout, and returns its stdout, asserting a clean exit.
fn run_std_test(name: &str) -> String {
    let path = repo().join("libs/std/tests").join(format!("{name}.cdl"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg(&path)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let output = common::output_with_deadline(&mut command, name);
    assert!(
        output.status.success(),
        "{name} exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn string_module() {
    assert!(run_std_test("test_string").contains("string ok"));
}

#[test]
fn list_reductions() {
    assert!(run_std_test("test_list").contains("list ok"));
}

#[test]
fn list_slices() {
    assert!(run_std_test("test_list_slices").contains("list slices ok"));
}

#[test]
fn list_methods() {
    assert!(run_std_test("test_list_methods").contains("list methods ok"));
}

#[test]
fn convert_module() {
    assert!(run_std_test("test_convert").contains("convert ok"));
}

#[test]
fn assert_module() {
    assert!(run_std_test("test_assert").contains("assert ok"));
}

/// Raising and catching across the library: a call inside a `try` returns, a
/// throw below the block reaches the `catch`, and the slicing helpers hold at
/// their boundaries.
#[test]
fn errors_across_modules() {
    assert!(run_std_test("test_errors").contains("errors ok"));
}

#[test]
fn option_module() {
    assert!(run_std_test("test_option").contains("option ok"));
}

#[test]
fn result_module() {
    assert!(run_std_test("test_result").contains("result ok"));
}

#[test]
fn json_module() {
    assert!(run_std_test("test_json").contains("json ok"));
}

#[test]
fn map_module() {
    assert!(run_std_test("test_map").contains("map ok"));
}

#[test]
fn set_module() {
    assert!(run_std_test("test_set").contains("set ok"));
}

/// The option/result enum modules must inline into a `.cdlb` and run under the
/// VM-only path with no source tree, exactly like the other pure-candela std
/// modules.
#[test]
fn option_result_inline_into_cdlb() {
    unsafe {
        std::env::set_var("CANDELA_LIB_PATH", repo().join("libs"));
    }
    let src = r#"
import "std/option";
import "std/result";
fn main() {
    print(None.unwrap_or(3));
    print(Some(7).unwrap());
    print(Ok(1).is_ok());
    print(Err("x").unwrap_err());
}
"#;
    let filename = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("option_result_probe.cdl")
        .to_string_lossy()
        .into_owned();
    let bytes = build_bytecode(src.to_owned(), &filename, &candela::ImportResolver::new())
        .expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB");
    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("artifact with inlined option/result must load");
    program.run();
}

/// A program that imports a library through a bare quoted path must run from any
/// working directory with nothing set, as long as `libs/` sits beside the
/// binary. This mirrors a clean install.
#[test]
fn default_resolution_needs_no_env() {
    let tmp = std::env::temp_dir().join(format!("candela_install_{}", std::process::id()));
    let bin_dir = tmp.join("bin");
    let std_dir = tmp.join("bin/libs/std");
    std::fs::create_dir_all(&std_dir).expect("create install layout");

    // The binary with `libs/std` beside it. `import "std/list"` needs only
    // list.cdl, which imports nothing else.
    let installed_bin = bin_dir.join("candela");
    std::fs::copy(env!("CARGO_BIN_EXE_candela"), &installed_bin).expect("copy binary");
    std::fs::copy(repo().join("libs/std/list.cdl"), std_dir.join("list.cdl")).expect("copy module");

    // A program in an unrelated directory.
    let work = tmp.join("work");
    std::fs::create_dir_all(&work).expect("create work dir");
    std::fs::write(
        work.join("prog.cdl"),
        "import \"std/list\";\nfn main() { print([5, 9, 2].max()); }\n",
    )
    .expect("write program");

    // A sibling test spawning its own child can inherit the write handle this
    // copy just closed, which makes the fresh binary briefly unexecutable
    // ("text file busy"). The handle goes away with that child, so retry.
    let mut attempt = 0;
    let output = loop {
        let mut command = Command::new(&installed_bin);
        command
            .arg("prog.cdl")
            .current_dir(&work)
            .env_remove("CANDELA_LIB_PATH");
        match common::try_output_with_deadline(&mut command, "zero-config run") {
            Ok(output) => break output,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 50 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("installed candela runs: {e}"),
        }
    };

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains('9'),
        "zero-config run failed: {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// A std module that binds a dynamic library is the one part of the library an
/// artifact cannot carry, so its recipe names the library relative to `libs`
/// and the runtime resolves it the way the compiler does. The artifact runs
/// from a directory with no `std_src` anywhere under it, once with
/// `CANDELA_LIB_PATH` naming the library directory and once with the layout an
/// install lays out beside the binaries and nothing set.
#[test]
fn a_std_dylib_artifact_runs_from_any_directory() {
    let candela_vm = Path::new(env!("CARGO_BIN_EXE_candela"))
        .parent()
        .expect("the test binary sits in a directory")
        .join(if cfg!(windows) {
            "candela-vm.exe"
        } else {
            "candela-vm"
        });
    if !candela_vm.exists() {
        eprintln!(
            "skipping a_std_dylib_artifact_runs_from_any_directory: {} not built",
            candela_vm.display()
        );
        return;
    }

    // The native module is compiled by the release workflow, not by cargo, so
    // a checkout that has not built it has nothing for the artifact to load.
    let native = if cfg!(windows) {
        "math.dll"
    } else if cfg!(target_os = "macos") {
        "math.dylib"
    } else {
        "math.so"
    };
    let native_src = repo()
        .join("libs")
        .join("std_src")
        .join("math")
        .join(native);
    if !native_src.exists() {
        eprintln!(
            "skipping a_std_dylib_artifact_runs_from_any_directory: {} not built",
            native_src.display()
        );
        return;
    }

    let tmp = std::env::temp_dir().join(format!("candela_std_dylib_{}", std::process::id()));
    let work = tmp.join("work");
    std::fs::create_dir_all(&work).expect("create work dir");
    std::fs::write(
        work.join("prog.cdl"),
        "import \"std/math\" as math;\nfn main() { print(math::sqrt(9.0)); }\n",
    )
    .expect("write program");

    let mut build = Command::new(env!("CARGO_BIN_EXE_candela"));
    build
        .arg("build")
        .arg("prog.cdl")
        .current_dir(&work)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let built = common::output_with_deadline(&mut build, "candela build");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr),
    );

    let mut by_env = Command::new(&candela_vm);
    by_env
        .arg("prog.cdlb")
        .current_dir(&work)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let named = common::output_with_deadline(&mut by_env, "candela-vm run");
    assert!(
        named.status.success() && String::from_utf8_lossy(&named.stdout).contains('3'),
        "run with CANDELA_LIB_PATH failed: {:?}\nstdout: {}\nstderr: {}",
        named.status.code(),
        String::from_utf8_lossy(&named.stdout),
        String::from_utf8_lossy(&named.stderr),
    );

    // The install layout: the binary at the top of the prefix, the library
    // under `libs/` beside it. Only the native module travels; the artifact
    // carries the candela source of every module it uses.
    let bin_dir = tmp.join("bin");
    let native_dir = bin_dir.join("libs").join("std_src").join("math");
    std::fs::create_dir_all(&native_dir).expect("create install layout");
    let installed_vm = bin_dir.join(candela_vm.file_name().expect("the binary has a name"));
    std::fs::copy(&candela_vm, &installed_vm).expect("copy binary");
    std::fs::copy(&native_src, native_dir.join(native)).expect("copy the native module");

    // A sibling test spawning its own child can inherit the write handle this
    // copy just closed, which makes the fresh binary briefly unexecutable
    // ("text file busy"). The handle goes away with that child, so retry.
    let mut attempt = 0;
    let beside = loop {
        let mut command = Command::new(&installed_vm);
        command
            .arg("prog.cdlb")
            .current_dir(&work)
            .env_remove("CANDELA_LIB_PATH");
        match common::try_output_with_deadline(&mut command, "installed candela-vm run") {
            Ok(output) => break output,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 50 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("installed candela-vm runs: {e}"),
        }
    };

    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        beside.status.success() && String::from_utf8_lossy(&beside.stdout).contains('3'),
        "run beside the install layout failed: {:?}\nstdout: {}\nstderr: {}",
        beside.status.code(),
        String::from_utf8_lossy(&beside.stdout),
        String::from_utf8_lossy(&beside.stderr),
    );
}

/// The json/map/set modules must inline into a `.cdlb` and run under the
/// VM-only path with no source tree, like the other pure-candela std modules.
#[test]
fn json_map_set_inline_into_cdlb() {
    unsafe {
        std::env::set_var("CANDELA_LIB_PATH", repo().join("libs"));
    }
    let src = r#"
import "std/json" as json;
import "std/set" as set;
fn main() {
    let obj = as_map(json::parse("{\"n\": 7, \"xs\": [1, 2, 3]}"));
    print(as_int(obj.get("n")));
    print(as_list(obj.get("xs")).len());
    print(json::stringify(json::parse("[1,2,3]")));
    let s = set::new<int>();
    s.add(1);
    s.add(1);
    s.add(2);
    let t = set::new<int>();
    t.add(2);
    t.add(3);
    print(s.len());
    print((s | t).members());
    print(json::stringify(s.members()));
}
"#;
    let filename = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("json_map_set_probe.cdl")
        .to_string_lossy()
        .into_owned();
    let bytes = build_bytecode(src.to_owned(), &filename, &candela::ImportResolver::new())
        .expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB");
    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("artifact with inlined json/map/set must load");
    program.run();
}

#[test]
fn std_inlines_into_cdlb() {
    // A program that imports pure-Candela std modules must build to a `.cdlb`
    // whose image carries the module bytecode inline, so the artifact loads with
    // no source tree present. `CANDELA_LIB_PATH` lets the compiler find the
    // modules from this checkout.
    unsafe {
        std::env::set_var("CANDELA_LIB_PATH", repo().join("libs"));
    }
    let src = r#"
import "std/string";
fn inc(x) { return x + 1; }
fn main() {
    print([1, 2, 3].sum());
    print([1, 2, 3].map(inc));
    print([1, 2, 3, 4].filter(fn(x) { return x % 2 == 0; }));
    print("hi".capitalize());
}
"#;
    let filename = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("std_cdlb_probe.cdl")
        .to_string_lossy()
        .into_owned();
    let bytes = build_bytecode(src.to_owned(), &filename, &candela::ImportResolver::new())
        .expect("builds to bytecode");
    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");
    assert!(
        load_program(&bytes, &HostRegistry::new()).is_ok(),
        "artifact with inlined std must load"
    );
}

/// Builds `libs/std_src/hash/hash.c` into a library directory of its own,
/// beside a copy of `libs/std`, and answers that directory; `None` where no C
/// compiler is on the path. The native modules are compiled by the release
/// workflow, not by cargo, so the test builds the one it needs.
fn lib_dir_with_hash(tag: &str) -> Option<PathBuf> {
    let libs = std::env::temp_dir().join(format!("candela_std_hash_{tag}_{}", std::process::id()));
    let std_dir = libs.join("std");
    let native_dir = libs.join("std_src").join("hash");
    std::fs::create_dir_all(&std_dir).expect("create std dir");
    std::fs::create_dir_all(&native_dir).expect("create native dir");
    for module in ["hash.cdl", "assert.cdl", "list.cdl"] {
        std::fs::copy(repo().join("libs/std").join(module), std_dir.join(module))
            .expect("copy module");
    }
    let library = native_dir.join(if cfg!(windows) {
        "hash.dll"
    } else if cfg!(target_os = "macos") {
        "hash.dylib"
    } else {
        "hash.so"
    });
    let compiler = std::env::var("CC").unwrap_or_else(|_| String::from("cc"));
    let mut command = Command::new(compiler);
    command.args(["-O2", "-std=c11", "-shared", "-fPIC", "-o"]);
    if cfg!(target_os = "macos") {
        command.arg("-dynamiclib");
    }
    let built = command
        .arg(&library)
        .arg(repo().join("libs/std_src/hash/hash.c"))
        .output();
    match built {
        Ok(output) if output.status.success() => Some(libs),
        Ok(output) => panic!(
            "building hash.c failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(e) => {
            eprintln!("skipping the hash module test: no C compiler ({e})");
            None
        }
    }
}

#[test]
fn hash_module() {
    let Some(libs) = lib_dir_with_hash("vectors") else {
        return;
    };
    let path = repo().join("libs/std/tests/test_hash.cdl");
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command.arg(&path).env("CANDELA_LIB_PATH", &libs);
    let output = common::output_with_deadline(&mut command, "test_hash");
    let _ = std::fs::remove_dir_all(&libs);
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("hash ok"),
        "test_hash exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// A NUL cannot cross into the C library, so hashing a string that holds one
/// raises an error instead of hashing the part before it.
#[test]
fn hashing_a_nul_raises() {
    let Some(libs) = lib_dir_with_hash("nul") else {
        return;
    };
    let work = libs.join("work");
    std::fs::create_dir_all(&work).expect("create work dir");
    std::fs::write(
        work.join("prog.cdl"),
        "import \"std/hash\" as hash;\nfn main() { print(hash::md5(\"a\\0b\")); }\n",
    )
    .expect("write program");
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg("prog.cdl")
        .current_dir(&work)
        .env("CANDELA_LIB_PATH", &libs);
    let output = common::output_with_deadline(&mut command, "hash a NUL");
    let _ = std::fs::remove_dir_all(&libs);
    assert!(
        !output.status.success(),
        "hashing a NUL ran: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("null byte"), "{stderr}");
}
