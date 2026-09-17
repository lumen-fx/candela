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

/// A recursive function, then a function that holds more live registers than it
/// does. The second function is declared after the first, so under a
/// saved-register table shared with the function table its id would name the
/// recursive call site's entry.
const RECURSION_PROGRAM: &str = "
fn fib(n) {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}

fn poly(a, b, c) {
    let d = a * b;
    let e = d + c;
    let f = e * e;
    let g = f - a;
    return g + b + c + d + e;
}

fn main() {
    print(fib(10));
    print(poly(2, 3, 4));
}
";

/// Calls applied to what another call or an index handed back. The callee is
/// settled at compile time from its static function type, so the artifact has
/// to carry the same entry the source run reaches.
const INDIRECT_CALL_PROGRAM: &str = "
fn pick() {
    return fn(x) { return x * 2; };
}

fn make() {
    return [fn(x) { return x + 1; }];
}

fn main() {
    print(pick()(21));
    let fs = make();
    print(fs[0](41));
    print(make()[0](41));
}
";

#[test]
fn bytecode_round_trips_through_load() {
    let bytes = candela::build_bytecode(
        STRUCT_PROGRAM.to_owned(),
        "struct_program.cdl",
        &candela::ImportResolver::new(),
    )
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
    let bytes = candela::build_bytecode(
        METHOD_PROGRAM.to_owned(),
        "methods.cdl",
        &candela::ImportResolver::new(),
    )
    .expect("a program using impl methods should compile to bytecode");

    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");

    // Load it with the lean loader and run it exactly as `candela-vm` does
    // (`candela-vm`'s whole job is `load_program(..).run()`). Methods are just
    // ordinary calls in the bytecode, so this executes to completion unchanged.
    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("method artifact must load on the VM-only path");
    program.run();
}

/// A program built from generic declarations. Every instantiation is an
/// ordinary struct by the time the artifact is written, so the format carries
/// nothing new and the VM-only path runs it unchanged.
const GENERIC_PROGRAM: &str = "
struct Cell<T> { value: T }

impl Cell<T> {
    fn get(self) -> T { return self.value; }
    fn tagged<U>(self, extra: U) -> U { return extra; }
}

enum Slot<T> { Filled(T), Empty }

fn first<T>(items: T[]) -> T { return items[0]; }

fn main() {
    print(Cell<int>{ value: 3 }.get());
    print(Cell<string>{ value: \"ab\" }.get());
    print(first<int>([7, 8]));
    print(Cell<int>{ value: 1 }.tagged<string>(\"z\"));
    match Slot<int>::Filled(9) {
        Filled(x) => { print(x); }
        _ => { print(0); }
    }
}
";

#[test]
fn generic_program_round_trips_and_runs() {
    let bytes = candela::build_bytecode(
        GENERIC_PROGRAM.to_owned(),
        "generics.cdl",
        &candela::ImportResolver::new(),
    )
    .expect("a program using type parameters should compile to bytecode");

    assert_eq!(&bytes[0..4], b"CDLB", "artifact must start with the magic");

    let mut program = load_program(&bytes, &HostRegistry::new())
        .expect("generic artifact must load on the VM-only path");
    program.run();
}

#[test]
fn empty_main_round_trips() {
    let bytes = candela::build_bytecode(
        "fn main() {}".to_owned(),
        "empty.cdl",
        &candela::ImportResolver::new(),
    )
    .expect("compiles");
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
fn current_format_version_is_eight_and_v2_is_rejected() {
    // The version byte was bumped to 8 when the cell instructions a captured
    // variable is read and written through joined the set. A freshly built
    // artifact must carry version 8.
    let bytes = candela::build_bytecode(
        "fn main() {}".to_owned(),
        "v.cdl",
        &candela::ImportResolver::new(),
    )
    .expect("compiles");
    assert_eq!(bytes[4], 8, "current .cdlb format version must be 8");

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
    let bytes =
        candela::build_bytecode(src.to_owned(), "enums.cdl", &candela::ImportResolver::new())
            .expect("compiles");
    assert_eq!(bytes[4], 8);
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
    let bytes = candela::build_bytecode(
        source,
        app.to_str().unwrap(),
        &candela::ImportResolver::new(),
    )
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
        let bytes =
            candela::build_bytecode(src.to_owned(), "zt.cdl", &candela::ImportResolver::new())
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
    let bytes = candela::build_bytecode(src.to_owned(), "h.cdl", &candela::ImportResolver::new())
        .expect("host-block program must now build to a .cdlb artifact");

    match load_program(&bytes, &HostRegistry::new()) {
        Err(candela::LoadError::HostBinding(candela::HostBindError::Unregistered(names))) => {
            assert_eq!(names, ["app::rows"], "the missing host fn must be named");
        }
        Err(e) => panic!("expected an unregistered host fn, got: {e}"),
        Ok(_) => panic!("load must not silently succeed when a host fn is unbound"),
    }
}

/// Values past 32 bits: a literal at the top of the range, a sum that used to
/// wrap, a product of two values that each fit in 32 bits, and a conversion
/// back out to text.
const WIDE_INT_PROGRAM: &str = "
fn mul(a, b) {
    return a * b;
}

fn main() {
    print(9223372036854775807);
    print(2147483647 + 1);
    print(mul(4294967296, 2));
    print(str(-9000000000));
}
";

/// A value past 32 bits reads the same through `candela <file>` and through
/// `candela build` plus `candela-vm`, so the artifact carries the whole width
/// rather than the half the register used to hold. Skips if `candela-vm` is
/// not built alongside `candela`.
#[test]
fn wide_ints_agree_across_source_and_artifact() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela).parent().unwrap().join(if cfg!(windows) {
        "candela-vm.exe"
    } else {
        "candela-vm"
    });
    if !candela_vm.exists() {
        eprintln!(
            "skipping wide_ints_agree_across_source_and_artifact: {} not built",
            candela_vm.display()
        );
        return;
    }

    let dir = scratch_dir("wide_ints");
    let app = dir.join("app.cdl");
    std::fs::write(&app, WIDE_INT_PROGRAM).unwrap();

    let mut src_cmd = std::process::Command::new(candela);
    src_cmd.arg(&app);
    let src_out = common::output_with_deadline(&mut src_cmd, "source run");
    assert!(src_out.status.success(), "candela source run failed");
    assert_eq!(
        String::from_utf8_lossy(&src_out.stdout).replace("\r\n", "\n"),
        "9223372036854775807\n2147483648\n8589934592\n-9000000000\n"
    );

    let cdlb = dir.join("app.cdlb");
    let mut build_cmd = std::process::Command::new(candela);
    build_cmd.arg("build").arg(&app).arg("-o").arg(&cdlb);
    let build = common::output_with_deadline(&mut build_cmd, "candela build");
    assert!(build.status.success(), "candela build failed");
    std::fs::remove_file(&app).unwrap();

    let mut vm_cmd = std::process::Command::new(&candela_vm);
    vm_cmd.arg(&cdlb);
    let vm_out = common::output_with_deadline(&mut vm_cmd, "candela-vm run");
    assert!(vm_out.status.success(), "candela-vm run failed");
    assert_eq!(
        vm_out.stdout, src_out.stdout,
        "candela-vm must reach the same answers as the source run"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A recursive function followed by one that lives in more registers runs to
/// the same answers through `candela <file>` and through `candela build` plus
/// `candela-vm`. The saved-register table a recursive call site writes is what
/// both paths read on the way back out of a call, so a table keyed the wrong
/// way shows up here as a wrong number. Skips if `candela-vm` is not built
/// alongside `candela`.
#[test]
fn recursion_then_a_wider_function_agrees_across_source_and_artifact() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela).parent().unwrap().join(if cfg!(windows) {
        "candela-vm.exe"
    } else {
        "candela-vm"
    });
    if !candela_vm.exists() {
        eprintln!(
            "skipping recursion_then_a_wider_function_agrees_across_source_and_artifact: {} not built",
            candela_vm.display()
        );
        return;
    }

    let dir = scratch_dir("recursion");
    let app = dir.join("app.cdl");
    std::fs::write(&app, RECURSION_PROGRAM).unwrap();

    let mut src_cmd = std::process::Command::new(candela);
    src_cmd.arg(&app);
    let src_out = common::output_with_deadline(&mut src_cmd, "source run");
    assert!(src_out.status.success(), "candela source run failed");
    assert_eq!(
        String::from_utf8_lossy(&src_out.stdout).replace("\r\n", "\n"),
        "55\n121\n",
        "fib(10) then poly(2, 3, 4)"
    );

    let cdlb = dir.join("app.cdlb");
    let mut build_cmd = std::process::Command::new(candela);
    build_cmd.arg("build").arg(&app).arg("-o").arg(&cdlb);
    let build = common::output_with_deadline(&mut build_cmd, "candela build");
    assert!(build.status.success(), "candela build failed");
    std::fs::remove_file(&app).unwrap();

    let mut vm_cmd = std::process::Command::new(&candela_vm);
    vm_cmd.arg(&cdlb);
    let vm_out = common::output_with_deadline(&mut vm_cmd, "candela-vm run");
    assert!(vm_out.status.success(), "candela-vm run failed");
    assert_eq!(
        vm_out.stdout, src_out.stdout,
        "candela-vm must reach the same answers as the source run"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A call on what a call or an index returned reaches the same answers through
/// `candela <file>` and through `candela build` plus `candela-vm`. Skips if
/// `candela-vm` is not built alongside `candela`.
#[test]
fn indirect_calls_agree_across_source_and_artifact() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela).parent().unwrap().join(if cfg!(windows) {
        "candela-vm.exe"
    } else {
        "candela-vm"
    });
    if !candela_vm.exists() {
        eprintln!(
            "skipping indirect_calls_agree_across_source_and_artifact: {} not built",
            candela_vm.display()
        );
        return;
    }

    let dir = scratch_dir("indirect_calls");
    let app = dir.join("app.cdl");
    std::fs::write(&app, INDIRECT_CALL_PROGRAM).unwrap();

    let mut src_cmd = std::process::Command::new(candela);
    src_cmd.arg(&app);
    let src_out = common::output_with_deadline(&mut src_cmd, "source run");
    assert!(src_out.status.success(), "candela source run failed");
    assert_eq!(
        String::from_utf8_lossy(&src_out.stdout).replace("\r\n", "\n"),
        "42\n42\n42\n",
        "a returned closure, an indexed one, and both chained"
    );

    let cdlb = dir.join("app.cdlb");
    let mut build_cmd = std::process::Command::new(candela);
    build_cmd.arg("build").arg(&app).arg("-o").arg(&cdlb);
    let build = common::output_with_deadline(&mut build_cmd, "candela build");
    assert!(build.status.success(), "candela build failed");
    std::fs::remove_file(&app).unwrap();

    let mut vm_cmd = std::process::Command::new(&candela_vm);
    vm_cmd.arg(&cdlb);
    let vm_out = common::output_with_deadline(&mut vm_cmd, "candela-vm run");
    assert!(vm_out.status.success(), "candela-vm run failed");
    assert_eq!(
        vm_out.stdout, src_out.stdout,
        "candela-vm must reach the same answers as the source run"
    );

    std::fs::remove_dir_all(&dir).ok();
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

/// The call depth limit belongs to the VM, so it holds for a `.cdlb` run the
/// same way it holds for a source run: a runaway recursion stops with the
/// error, and a deep one that ends still reaches its answer. Skips if
/// `candela-vm` is not built alongside `candela`.
#[test]
fn the_call_depth_limit_holds_for_an_artifact_run() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela).parent().unwrap().join(if cfg!(windows) {
        "candela-vm.exe"
    } else {
        "candela-vm"
    });
    if !candela_vm.exists() {
        eprintln!(
            "skipping the_call_depth_limit_holds_for_an_artifact_run: {} not built",
            candela_vm.display()
        );
        return;
    }

    let dir = scratch_dir("call_depth");
    for (name, source, deep) in [
        (
            "runaway",
            "fn climb(n) {\n    return climb(n + 1);\n}\n\nfn main() { print(climb(0)); }\n",
            false,
        ),
        (
            "bounded",
            "fn countdown(n) {\n    if n == 0 { return 0; }\n    return countdown(n - 1) + 1;\n}\n\nfn main() { print(countdown(100000)); }\n",
            true,
        ),
    ] {
        let app = dir.join(format!("{name}.cdl"));
        std::fs::write(&app, source).unwrap();
        let cdlb = dir.join(format!("{name}.cdlb"));
        let mut build_cmd = std::process::Command::new(candela);
        build_cmd.arg("build").arg(&app).arg("-o").arg(&cdlb);
        let build = common::output_with_deadline(&mut build_cmd, "candela build");
        assert!(build.status.success(), "candela build failed");

        let mut vm_cmd = std::process::Command::new(&candela_vm);
        vm_cmd.arg(&cdlb);
        let vm_out = common::output_with_deadline(&mut vm_cmd, "candela-vm run");
        if deep {
            assert!(vm_out.status.success(), "a bounded deep recursion must run");
            assert_eq!(
                String::from_utf8_lossy(&vm_out.stdout).replace("\r\n", "\n"),
                "100000\n"
            );
        } else {
            assert!(
                !vm_out.status.success(),
                "a runaway recursion must stop the run"
            );
            let said = String::from_utf8_lossy(&vm_out.stderr);
            assert!(
                said.contains("climb"),
                "the report names the call it stopped at: {said}"
            );
        }
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// String positions count characters, and the VM is what counts them, so an
/// artifact run indexes, slices and iterates a string with characters wider
/// than one byte exactly as a source run does. Skips if `candela-vm` is not
/// built alongside `candela`.
#[test]
fn string_positions_count_characters_in_an_artifact_run() {
    let candela = env!("CARGO_BIN_EXE_candela");
    let candela_vm = Path::new(candela).parent().unwrap().join(if cfg!(windows) {
        "candela-vm.exe"
    } else {
        "candela-vm"
    });
    if !candela_vm.exists() {
        eprintln!(
            "skipping string_positions_count_characters_in_an_artifact_run: {} not built",
            candela_vm.display()
        );
        return;
    }

    let dir = scratch_dir("char_positions");
    let app = dir.join("app.cdl");
    std::fs::write(
        &app,
        "
fn main() {
    let word = \"caf\u{e9}\";
    print(word.len());
    print(word[3]);
    print(word[0..4]);
    print(word.find(\"\u{e9}\"));
    for c in word {
        print(c);
    }
    let mixed = \"a\u{1f600}b\u{4e2d}\";
    print(mixed.len());
    print(mixed[1..3]);
    try {
        print(word[4]);
    } catch \"index_out_of_bounds\" {
        print(\"past the end\");
    }
}
",
    )
    .unwrap();

    let cdlb = dir.join("app.cdlb");
    let mut build_cmd = std::process::Command::new(candela);
    build_cmd.arg("build").arg(&app).arg("-o").arg(&cdlb);
    let build = common::output_with_deadline(&mut build_cmd, "candela build");
    assert!(build.status.success(), "candela build failed");

    let mut vm_cmd = std::process::Command::new(&candela_vm);
    vm_cmd.arg(&cdlb);
    let vm_out = common::output_with_deadline(&mut vm_cmd, "candela-vm run");
    assert!(vm_out.status.success(), "candela-vm run failed");
    assert_eq!(
        String::from_utf8_lossy(&vm_out.stdout).replace("\r\n", "\n"),
        "4\n\u{e9}\ncaf\u{e9}\n3\nc\na\nf\n\u{e9}\n4\n\u{1f600}b\npast the end\n",
        "the artifact run counts characters everywhere a position appears"
    );

    std::fs::remove_dir_all(&dir).ok();
}
