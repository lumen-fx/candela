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

mod common;

use candela::HostRegistry;
use candela::load_program;
use std::path::Path;
use std::path::PathBuf;

/// A unique scratch directory under the system temp dir. `.cdlb` builds resolve
/// `import "..."` relative to the main file's path, so multi-file tests need
/// real files on disk.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "candela_cdlb_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

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
        load_program(&bytes, &HostRegistry::new()).is_ok(),
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
    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("method artifact must load on the VM-only path");
    program.run();
}

#[test]
fn empty_main_round_trips() {
    let bytes = candela::build_bytecode("fn main() {}".to_owned(), "empty.cdl").expect("compiles");
    assert!(
        load_program(&bytes, &HostRegistry::new()).is_ok(),
        "empty program must load"
    );
}

#[test]
fn bad_magic_is_rejected() {
    assert!(matches!(
        load_program(b"NOPE\x01garbage", &HostRegistry::new()),
        Err(candela::LoadError::BadMagic)
    ));
}

#[test]
fn truncated_is_rejected() {
    assert!(matches!(
        load_program(b"CD", &HostRegistry::new()),
        Err(candela::LoadError::Truncated)
    ));
}

#[test]
fn unknown_version_is_rejected() {
    // Correct magic, but a version byte this runtime does not understand.
    assert!(matches!(
        load_program(b"CDLB\xff", &HostRegistry::new()),
        Err(candela::LoadError::UnsupportedVersion(0xff))
    ));
}

#[test]
fn current_format_version_is_five_and_v2_is_rejected() {
    // The version byte was bumped to 5 when the export table was added. A
    // freshly built artifact must carry version 5.
    let bytes = candela::build_bytecode("fn main() {}".to_owned(), "v.cdl").expect("compiles");
    assert_eq!(bytes[4], 5, "current .cdlb format version must be 5");

    // A well-formed magic but a previous version must fail cleanly, not
    // mis-decode. (Bytes after the header are irrelevant; the version gate
    // rejects before decoding the body.)
    assert!(matches!(
        load_program(b"CDLB\x02anything", &HostRegistry::new()),
        Err(candela::LoadError::UnsupportedVersion(2))
    ));
}

#[test]
fn enum_values_roundtrip_through_cdlb() {
    // A whole-program artifact that constructs and matches an enum with a
    // payload must serialize and re-run on the VM-only path with no source tree.
    let src = "
        enum Shape { Circle(int), Rect(int, int), Unit }
        fn main() {
            let s = Shape::Rect(6, 7);
            let a = 0;
            match s {
                Circle(r) => { a = r; }
                Rect(w, h) => { a = w * h; }
                Unit => { a = -1; }
            }
            print(a);
        }
    ";
    let bytes = candela::build_bytecode(src.to_owned(), "enums.cdl").expect("compiles");
    assert_eq!(bytes[4], 5);
    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("enum artifact must load on the VM-only path");
    program.run();
}

/// A `.cdlb` must embed the whole program: every imported workspace `.cdl`
/// module is linked into the single artifact, so it runs under the VM-only path
/// with the entire source tree absent.
#[test]
fn multi_file_program_is_captured_whole() {
    let dir = scratch_dir("multifile");
    let util = dir.join("util.cdl");
    let app = dir.join("app.cdl");
    std::fs::write(&util, "fn double(x) { return x * 2; }\n").unwrap();
    std::fs::write(
        &app,
        "import \"util.cdl\" as util;\n\nfn main() { print(util::double(21)); }\n",
    )
    .unwrap();

    // Build with the imported module present; the artifact must fold util.cdl's
    // bytecode in.
    let source = std::fs::read_to_string(&app).unwrap();
    let bytes = candela::build_bytecode(source, app.to_str().unwrap())
        .expect("multi-file program compiles to a whole-program artifact");

    // Delete both source files: nothing on disk to fall back to.
    std::fs::remove_file(&app).unwrap();
    std::fs::remove_file(&util).unwrap();

    // The artifact still loads and runs, proof the imported module was
    // captured, not merely referenced.
    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("whole-program artifact must load with sources absent");
    program.run();

    std::fs::remove_dir_all(&dir).ok();
}

/// A program that `dylib`-imports a ubiquitous system library round-trips
/// through `.cdlb` and re-resolves the symbol by name at load. Uses zlib's
/// `zlibVersion()` (a zero-arg `const char*`), guarded so it skips cleanly where
/// `libz` is not present as a dlopen-able `lib<name>` file.
#[test]
fn dyn_lib_program_round_trips_and_rebinds() {
    // Match how the loader resolves a bare logical name on this OS, and only run
    // when that file is openable here.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let openable = unsafe { libloading::Library::new("libz.so") }.is_ok()
            || unsafe { libloading::Library::new("libz.dylib") }.is_ok()
            || unsafe { libloading::Library::new("z.dll") }.is_ok();
        if !openable {
            eprintln!(
                "skipping dyn_lib_program_round_trips_and_rebinds: libz not dlopen-able here"
            );
            return;
        }

        let src =
            "dylib \"z\" { string zlibVersion(); }\n\nfn main() { print(z::zlibVersion()); }\n";
        let bytes = candela::build_bytecode(src.to_owned(), "zt.cdl")
            .expect("dyn-lib program must now build to a .cdlb artifact");

        // The artifact stores only the recipe (name `z`, symbol `zlibVersion`,
        // signature), never the shared object's bytes.
        assert!(
            !contains_subslice(&bytes, b"\x7fELF"),
            "artifact must not embed the shared object's ELF bytes"
        );

        // Load re-opens libz through the OS loader and rebuilds the libffi CIF,
        // then runs to completion (prints the zlib version).
        let mut program = load_program(&bytes, &HostRegistry::new())
            .expect("dyn-lib artifact must re-resolve and load");
        program.run();
    }
}

/// Scans for a byte subsequence (used to assert the .so bytes are not embedded).
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A `host` block program builds to a `.cdlb` (the recipe is captured), but an
/// empty registry has no closure to bind the host fn to, so loading it must
/// fail with a clear error that names the missing function.
#[test]
fn host_block_program_builds_but_load_names_missing_host_fn() {
    let src = "host \"app\" { int rows(string); }\n\nfn main() { print(\"start\"); }\n";
    let bytes = candela::build_bytecode(src.to_owned(), "h.cdl")
        .expect("host-block program must now build to a .cdlb artifact");

    match load_program(&bytes, &HostRegistry::new()) {
        Err(candela::LoadError::HostBinding(candela::HostBindError::Unregistered(names))) => {
            assert_eq!(names, ["app::rows"], "the missing host fn must be named");
        }
        Err(e) => panic!("expected an unregistered host fn, got: {e}"),
        Ok(_) => panic!("load must not silently succeed when a host fn is unbound"),
    }
}

/// Full CLI round-trip: compile source with `candela`, run the `.cdlb` with the
/// VM-only `candela-vm` with the source tree removed, and require byte-identical
/// stdout to running the source directly. Skips if the `candela-vm` binary is
/// not built alongside `candela` (e.g. a plain `cargo test` that did not build
/// the vm package).
#[test]
fn cli_whole_program_output_matches_source_run() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela).parent().unwrap().join(if cfg!(windows) {
        "candela-vm.exe"
    } else {
        "candela-vm"
    });
    if !candela_vm.exists() {
        eprintln!(
            "skipping cli_whole_program_output_matches_source_run: {} not built",
            candela_vm.display()
        );
        return;
    }

    let dir = scratch_dir("cli");
    let util = dir.join("util.cdl");
    let app = dir.join("app.cdl");
    std::fs::write(&util, "fn triple(x) { return x * 3; }\n").unwrap();
    std::fs::write(
        &app,
        "import \"util.cdl\" as util;\n\nfn main() { print(util::triple(14)); print(\"done\"); }\n",
    )
    .unwrap();

    // Reference output: run the source directly.
    let mut src_cmd = std::process::Command::new(candela);
    src_cmd.arg(&app);
    let src_out = common::output_with_deadline(&mut src_cmd, "source run");
    assert!(src_out.status.success(), "candela source run failed");

    // Build the artifact, then delete the whole source tree.
    let cdlb = dir.join("app.cdlb");
    let mut build_cmd = std::process::Command::new(candela);
    build_cmd.arg("build").arg(&app).arg("-o").arg(&cdlb);
    let build = common::output_with_deadline(&mut build_cmd, "candela build");
    assert!(build.status.success(), "candela build failed");
    std::fs::remove_file(&app).unwrap();
    std::fs::remove_file(&util).unwrap();

    // Run the artifact with the VM-only binary and require identical stdout.
    let mut vm_cmd = std::process::Command::new(&candela_vm);
    vm_cmd.arg(&cdlb);
    let vm_out = common::output_with_deadline(&mut vm_cmd, "candela-vm run");
    assert!(vm_out.status.success(), "candela-vm run failed");
    assert_eq!(
        vm_out.stdout, src_out.stdout,
        "candela-vm output must match the source run byte-for-byte"
    );

    std::fs::remove_dir_all(&dir).ok();
}
