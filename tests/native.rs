//! Machine code in `.cdlb` artifacts.
//!
//! A release build puts machine code for the functions the code generator
//! handles into the artifact, and the VM runs it in their place. Everything
//! observable has to stay as the interpreter makes it: output, errors and
//! their spans, `try` and `catch`, the call depth limit, the collector, and a
//! panic in a host function. These tests build each program twice, with and
//! without machine code, and hold the two runs to the same result; others
//! check that the code covers what it should and that an artifact the VM
//! cannot run the code of still runs on bytecode.

#![cfg(all(
    feature = "native",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]

use candela::BuildOptions;
use candela::Diagnostic;
use candela::HostRegistry;
use candela::ImportResolver;
use candela::NativeCode;
use candela::RuntimeProgram;
use candela::Value;
use candela::build_artifact;
use candela::collect_diagnostic;
use candela::load_program;
use candela_vm::artifact::NativeArch;
use candela_vm::artifact::NativeOs;
use candela_vm::artifact::read_artifact;
use candela_vm::artifact::serialize_image;
use candela_vm::captured_output::CAPTURED_OUTPUT;
use candela_vm::captured_output::set_capturing;

/// Builds `src` into artifact bytes with the given machine code.
fn build(src: &str, native: NativeCode) -> Vec<u8> {
    build_artifact(
        src.to_owned(),
        "app.cdl",
        &ImportResolver::new(),
        &BuildOptions {
            release: true,
            native,
        },
    )
    .expect("the program builds")
}

fn load(bytes: &[u8]) -> RuntimeProgram {
    load_program(bytes, &HostRegistry::new()).expect("the artifact loads")
}

/// What a run printed, and how it ended.
fn run(program: &mut RuntimeProgram) -> (String, Result<(), Diagnostic>) {
    CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    let was = set_capturing(true);
    let result = collect_diagnostic(|| program.run());
    set_capturing(was);
    (CAPTURED_OUTPUT.with(|o| o.take()), result)
}

/// Runs `src` from an artifact with machine code and from one without, and
/// answers the machine code run, after checking the two agree and that
/// machine code ran.
fn agrees(src: &str) -> (String, Result<(), Diagnostic>) {
    let mut native = load(&build(src, NativeCode::Host));
    let mut bytecode = load(&build(src, NativeCode::None));
    assert!(
        native.native_functions() > 0,
        "some function of the program runs as machine code"
    );
    assert_eq!(bytecode.native_functions(), 0);
    let with = run(&mut native);
    let without = run(&mut bytecode);
    assert_eq!(with, without, "machine code behaves as the bytecode does");
    with
}

/// The names of the functions of `src` that run as machine code: `main`, or
/// one some call reaches. A function every call to which the release profile
/// inlined has machine code nothing runs.
fn native_names(src: &str) -> Vec<String> {
    let (image, table) = read_artifact(&build(src, NativeCode::Host)).expect("reads back");
    let table = table.expect("a release build writes the function table");
    let section = image.native.first().expect("a section for this machine");
    let mut names: Vec<String> = section
        .functions
        .iter()
        .enumerate()
        .filter(|(k, f)| {
            f.loc == 0
                || section
                    .sites
                    .iter()
                    .any(|site| site.function as usize == *k)
        })
        .filter_map(|(_, f)| table.functions.iter().find(|t| t.entry == f.loc))
        .map(|t| t.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

const NBODY: &str = include_str!("../examples/nbody/nbody_lua.cdl");

/// The functions of the n-body benchmark all run as machine code: `advance`
/// and the two that reuse one register for values of different types,
/// `energy` and `offsetMomentum`.
#[test]
fn every_nbody_function_but_main_is_machine_code() {
    let names = native_names(NBODY);
    for name in ["advance", "energy", "offsetMomentum"] {
        assert!(
            names.contains(&name.to_owned()),
            "{name} runs as machine code: {names:?}"
        );
    }
}

#[test]
fn recursion_runs_as_machine_code() {
    let (printed, result) = agrees(
        "
        fn fib(n) {
            if n <= 1 { return n; }
            return fib(n - 1) + fib(n - 2);
        }
        fn main() {
            print(fib(20));
            print(fib(1));
        }
        ",
    );
    assert_eq!(printed, "6765\n1\n");
    assert!(result.is_ok());
    assert_eq!(
        native_names(
            "fn fib(n) { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fn main() { print(fib(5)); }"
        ),
        vec!["fib", "main"]
    );
}

#[test]
fn floats_structs_and_lists() {
    let (printed, _) = agrees(
        "
        struct P { x: float, y: float, n: int }
        fn step(ps: P[], dt: float) {
            for i in 0..ps.len() {
                let p = ps[i];
                p.x += p.y * dt;
                p.n = p.n + 1;
            }
        }
        fn total(ps: P[]) -> float {
            let t = 0.0;
            for i in 0..ps.len() { t += ps[i].x + float(ps[i].n); }
            return t;
        }
        fn main() {
            let ps = [P { x: 1.0, y: 2.0, n: 0 }, P { x: -3.5, y: 0.25, n: 4 }];
            for k in 0..10 { step(ps, 0.5); }
            print(total(ps));
            print(ps[0].x, ps[1].n);
        }
        ",
    );
    assert_eq!(printed, "32.75\n11.0\n14\n");
}

/// An index error raised in machine code, caught by an interpreted `try`
/// with native frames between, and one nobody catches, reported at the same
/// expression.
#[test]
fn errors_in_machine_code_unwind_like_the_interpreter() {
    let (printed, result) = agrees(
        "
        fn get(xs: int[], i: int) -> int { return xs[i]; }
        fn sum_to(xs: int[], n: int) -> int {
            let s = 0;
            for i in 0..n { s = s + get(xs, i); }
            return s;
        }
        fn safe(xs: int[], n: int) -> int {
            let total = -1;
            try { total = sum_to(xs, n); } catch e { print(e); }
            return total;
        }
        fn main() {
            let xs = [1, 2, 3];
            print(safe(xs, 3));
            print(safe(xs, 5));
            print(sum_to(xs, 4));
        }
        ",
    );
    assert_eq!(printed, "6\nindex_out_of_bounds\n-1\n");
    let error = result.expect_err("the last call fails");
    assert_eq!(error.code, "index_out_of_bounds");
}

/// Machine code calls a function it could not compile through the
/// interpreter, which throws; a `try` two frames further out catches it.
#[test]
fn an_error_thrown_in_a_call_back_into_the_interpreter_is_caught_outside() {
    let (printed, result) = agrees(
        r#"
        fn boom(n: int) -> int {
            if n > 2 { throw("n" + str(n)); }
            if n == 0 { return 0; }
            let label = "n" + str(n);
            return boom(n - 1) + label.len() - 1;
        }
        fn walk(n: int) -> int {
            let t = 0;
            for i in 0..n { t = t + boom(i); }
            return t;
        }
        fn main() {
            try {
                print(walk(3));
                print(walk(6));
            } catch e {
                print("caught " + e);
            }
            print(walk(2));
        }
        "#,
    );
    assert_eq!(printed, "3\ncaught n3\n1\n");
    assert!(result.is_ok());
}

/// A shift count out of range raises the same error from machine code.
#[test]
fn shifts_and_bits() {
    let (printed, result) = agrees(
        "
        fn mix(a: int, b: int) -> int { return ((a << 3) ^^ (b >> 1)) & ~(a | 5); }
        fn shift(a: int, n: int) -> int { return a << n; }
        fn main() {
            print(mix(12345, -99));
            print(shift(1, 63));
            print(shift(1, 64));
        }
        ",
    );
    assert_eq!(printed.lines().count(), 2);
    assert_eq!(
        result.expect_err("a shift by 64 fails").code,
        "shift_count_out_of_range"
    );
}

const HELD: &str = "
        fn drop(h: int[][], k: int) {
            if k == 0 { return; }
            h.remove(0);
            drop(h, k - 1);
        }
        fn churn(n: int) -> int {
            if n == 0 { return 0; }
            let t = [n, n + 1, n + 2];
            return t[2] + churn(n - 1);
        }
        fn keep(h: int[][]) -> int {
            let xs = h[0];
            drop(h, 1);
            let n = churn(300);
            return xs[0] + xs[1] + n;
        }
        fn mid(n: int, k: int) -> int {
            if k == 0 { return churn(n); }
            return mid(n, k - 1) + 1;
        }
        fn keep_through(h: int[][]) -> int {
            let xs = h[0];
            drop(h, 1);
            let n = mid(300, 2);
            return xs[0] + xs[1] + n;
        }
        fn make(i: int) -> int[][] {
            let h = [[0, 0]];
            h.remove(0);
            for k in 0..3 { h.push([i + k, 2 * (i + k)]); }
            return h;
        }
        fn main() {
            let total = 0;
            for i in 0..40 {
                total = total + keep(make(i)) + keep_through(make(i));
            }
            print(total);
        }
        ";

/// A reference machine code holds only in its own variables survives a
/// collection a call back into the interpreter runs, whether the call goes
/// there directly or through native callees: `keep` takes the first list out
/// of `h`, and nothing else holds it once `drop` removes it there.
/// Run the suite with the `gc-torture` feature of candela-vm to collect on
/// every allocation.
#[test]
fn a_reference_held_across_an_allocating_call_stays_alive() {
    let (printed, result) = agrees(HELD);
    assert_eq!(printed, "3664760\n");
    assert!(result.is_ok());
    let names = native_names(HELD);
    for name in ["keep", "keep_through", "mid"] {
        assert!(
            names.contains(&name.to_owned()),
            "{name} runs as machine code: {names:?}"
        );
    }
}

/// A store from machine code shades what it overwrites while the collector
/// marks. `swap_ends` moves a list from the end of a long one, which marking
/// walks last, into its first slot, which marking walked already; without
/// the barrier that list would go unmarked and be freed while `h` still
/// holds it. Under the `gc-torture` feature every finished mark is checked against
/// a fresh trace.
#[test]
fn stores_from_machine_code_keep_the_collector_right() {
    let src = "
        fn swap_ends(h: int[][], times: int) {
            if times == 0 { return; }
            let n = h.len();
            let a = h[0];
            h[0] = h[n - 2];
            h[n - 2] = a;
            swap_ends(h, times - 1);
        }
        fn fresh(i: int) -> int { if i == 0 { return 0; } let t = [i]; return t[0] + fresh(i - 1) - i; }
        fn main() {
            let h = [[0]];
            h.remove(0);
            for k in 0..300 { h.push([k, k + 1]); }
            let spent = 0;
            for round in 0..200 {
                spent = spent + fresh(20);
                swap_ends(h, 1);
            }
            let total = 0;
            for k in 0..300 { total = total + h[k][0] * (k + 1) + h[k][1]; }
            print(total, h[0][0], h[299][0], spent);
        }
    ";
    let (printed, result) = agrees(src);
    assert!(result.is_ok());
    assert_eq!(printed.lines().count(), 4);
    assert!(native_names(src).contains(&"swap_ends".to_owned()));
}

/// A recursion past the call depth limit stops at the same call with the
/// same error, a `try` around it catches it, and a deep recursion that ends
/// still reaches its answer: native frames give way to the interpreter's
/// before the thread runs out of stack.
#[test]
fn the_call_depth_limit_holds_in_machine_code() {
    let (printed, result) = agrees(
        "
        fn climb(n: int) -> int { return climb(n + 1); }
        fn countdown(n: int) -> int {
            if n == 0 { return 0; }
            return countdown(n - 1) + 1;
        }
        fn main() {
            print(countdown(300000));
            try { print(climb(0)); } catch e { print(e); }
            print(climb(0));
        }
        ",
    );
    assert_eq!(printed, "300000\ncall_depth_exceeded\n");
    let error = result.expect_err("the runaway recursion fails");
    assert_eq!(error.code, "call_depth_exceeded");
    assert!(error.message.contains("climb"), "{}", error.message);
}

/// `print` from machine code shows every kind of value.
#[test]
fn printing_from_machine_code() {
    let (printed, _) = agrees(
        r#"
        struct S { a: int, b: string }
        fn show(xs: int[], s: S, f: float, b: bool, t: string) {
            print(xs);
            print(s);
            print(f);
            print(b);
            print(t);
        }
        fn main() { show([1, 2], S { a: 3, b: "four" }, 2.5, true, "text"); }
        "#,
    );
    assert_eq!(printed, "[1,2]\nS {a:3,b:\"four\"}\n2.5\ntrue\ntext\n");
}

/// A host calls an exported function that runs as machine code.
#[test]
fn an_export_calls_into_machine_code() {
    let src = "
        fn fib(n: int) -> int {
            if n <= 1 { return n; }
            return fib(n - 1) + fib(n - 2);
        }
        fn main() {}
    ";
    let mut program = load(&build(src, NativeCode::Host));
    assert!(program.native_functions() > 0);
    program.run();
    assert_eq!(
        program.call("fib", &[Value::Int(24)]).unwrap(),
        Value::Int(46368)
    );
    assert_eq!(
        program.call("fib", &[Value::Int(2)]).unwrap(),
        Value::Int(1)
    );
}

/// A host function that panics inside a call machine code makes back into
/// the interpreter panics out of the host's call, as it does without machine
/// code, instead of taking the process down.
#[test]
fn a_panic_in_a_host_function_reaches_the_host() {
    let src = r#"
        host "app" { int boom(int); }
        fn relay(n: int, k: int) -> int {
            if k == 0 { return app::boom(n); }
            return relay(n, k - 1);
        }
        fn outer(n: int) -> int {
            let t = 0;
            for i in 0..n { t = t + relay(i, 1); }
            return t;
        }
        fn main() {}
    "#;
    let mut hosts = HostRegistry::new();
    hosts.register_host_fn("app", "boom", |n: i64| -> i64 {
        assert!(n < 2, "boom at {n}");
        n
    });
    assert!(native_names(src).contains(&"outer".to_owned()));
    let bytes = build(src, NativeCode::Host);
    let mut program = load_program(&bytes, &hosts).expect("loads");
    assert_eq!(
        program.call("outer", &[Value::Int(2)]).unwrap(),
        Value::Int(1)
    );
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = program.call("outer", &[Value::Int(5)]);
    }))
    .expect_err("the host function's panic comes out of the call");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("boom at 2"), "{message}");
}

/// The debug profile and `--no-native` build no machine code.
#[test]
fn only_a_release_build_carries_machine_code() {
    let src = "fn main() { let t = 0; for i in 0..10 { t = t + i; } print(t); }";
    let (image, _) = read_artifact(&build(src, NativeCode::Host)).unwrap();
    assert_eq!(image.native.len(), 1);
    let (image, _) = read_artifact(&build(src, NativeCode::None)).unwrap();
    assert!(image.native.is_empty());
    let debug = build_artifact(
        src.to_owned(),
        "app.cdl",
        &ImportResolver::new(),
        &BuildOptions {
            release: false,
            native: NativeCode::Host,
        },
    )
    .unwrap();
    assert!(read_artifact(&debug).unwrap().0.native.is_empty());
}

const LOOP: &str = "
    fn sum(n: int) -> int { let t = 0; for i in 0..n { t = t + i * i; } return t; }
    fn main() { print(sum(1000)); }
";

/// Code for another machine rides in the artifact, and this machine runs
/// the bytecode instead.
#[test]
fn code_built_for_another_machine_runs_as_bytecode_here() {
    for (triple, arch, os) in [
        ("aarch64-apple-darwin", NativeArch::Aarch64, NativeOs::MacOs),
        (
            "aarch64-unknown-linux-gnu",
            NativeArch::Aarch64,
            NativeOs::Linux,
        ),
        (
            "x86_64-pc-windows-msvc",
            NativeArch::X86_64,
            NativeOs::Windows,
        ),
        ("x86_64-apple-darwin", NativeArch::X86_64, NativeOs::MacOs),
        (
            "x86_64-unknown-linux-gnu",
            NativeArch::X86_64,
            NativeOs::Linux,
        ),
    ] {
        let bytes = build(LOOP, NativeCode::Target(triple.to_owned()));
        let (image, _) = read_artifact(&bytes).unwrap();
        let section = &image.native[0];
        assert_eq!((section.arch, section.os), (arch, os), "{triple}");
        assert!(!section.code.is_empty());
        let mut program = load(&bytes);
        let here = cfg!(target_arch = "x86_64") == (arch == NativeArch::X86_64)
            && cfg!(target_os = "macos") == (os == NativeOs::MacOs)
            && cfg!(target_os = "windows") == (os == NativeOs::Windows);
        assert_eq!(program.native_functions() > 0, here, "{triple}");
        assert_eq!(run(&mut program).0, "332833500\n");
    }
    let refused = build_artifact(
        LOOP.to_owned(),
        "app.cdl",
        &ImportResolver::new(),
        &BuildOptions {
            release: true,
            native: NativeCode::Target(String::from("riscv64gc-unknown-linux-gnu")),
        },
    );
    assert!(refused.is_err());
}

/// A section the VM cannot trust or cannot run on this machine leaves the
/// program on bytecode, with the same output.
#[test]
fn a_section_that_does_not_fit_runs_as_bytecode() {
    let bytes = build(LOOP, NativeCode::Host);
    assert!(load(&bytes).native_functions() > 0);
    let damage: [(&str, fn(&mut candela_vm::artifact::NativeImage)); 5] = [
        ("another contract", |s| s.abi += 1),
        ("a processor feature this machine lacks", |s| {
            s.cpu_features.push(String::from("no-such-feature"));
        }),
        ("another object layout", |s| s.object_layout[1] ^= 8),
        ("a call site that is not a call", |s| {
            if let Some(site) = s.sites.first_mut() {
                site.at = 0;
            }
            s.sites
                .push(candela_vm::artifact::NativeSiteImage { at: 0, function: 0 });
        }),
        ("an entry outside the code", |s| {
            s.functions[0].offset = u32::MAX;
        }),
    ];
    for (what, change) in damage {
        let (mut image, table) = read_artifact(&bytes).unwrap();
        change(&mut image.native[0]);
        let changed = serialize_image(&image, table.as_ref()).unwrap();
        let mut program = load(&changed);
        assert_eq!(program.native_functions(), 0, "{what}");
        assert_eq!(run(&mut program).0, "332833500\n", "{what}");
    }
}
