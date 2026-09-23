//! The standard library's native modules in the browser build.
//!
//! `std/math`, `std/time` and `std/random` bind a C library on a desktop. The
//! browser build carries those modules and runs their bindings on the
//! runtime's own functions instead, so the same imports work there. Run with
//! `cargo test --target wasm32-unknown-unknown --test wasm_std` and
//! `wasm-bindgen-test-runner` as the target's runner.

#![cfg(target_arch = "wasm32")]

use candela::{HostRegistry, ImportResolver, Value, build_bytecode, get_output, load_program};
use wasm_bindgen_test::wasm_bindgen_test;

/// Runs `code` through the browser build's entry point and returns what it
/// printed.
fn run(code: &str) -> String {
    let _ = get_output();
    candela::run(String::from(code));
    get_output()
}

#[wasm_bindgen_test]
fn math_imports_and_computes() {
    let out = run(r#"
import "std/math" as math;
fn main() {
    print(math::sqrt(16.0));
    print(math::sin(0.0));
    print(math::pow(2.0, 10.0));
    print(math::floor(math::pi()));
}
"#);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        ["4.0", "0.0", "1024.0", "3.0"]
    );
}

/// Values a desktop's C maths library gives, including its answers at the
/// edges of a domain.
#[wasm_bindgen_test]
fn math_answers_what_a_desktop_answers() {
    let out = run(r#"
import "std/math" as math;
fn main() {
    print(math::logb(8.0));
    print(math::ilogb(0.0));
    print(math::erf(0.5));
    print(math::ldexp(1.0, 3));
    print(math::fmod(7.5, 2.0));
    print(math::sqrt(-1.0));
}
"#);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "3.0",
            "-2147483648",
            "0.5204998778130465",
            "8.0",
            "1.5",
            "NaN"
        ]
    );
}

#[wasm_bindgen_test]
fn time_reads_the_browser_clock() {
    let out = run(r#"
import "std/time" as time;
fn main() {
    print(time::now());
    print(time::format(0, "%Y"));
    print(time::format(1000036800, "%Y-%m-%d %j %a %B"));
}
"#);
    let mut lines = out.lines();
    let now: i64 = lines.next().unwrap().parse().unwrap();
    let expected = (js_sys_now() / 1000.0) as i64;
    assert!((now - expected).abs() <= 2, "{now} vs {expected}");
    // The epoch is 1970 in UTC and 1969 west of it.
    assert!(matches!(lines.next(), Some("1970" | "1969")), "{out}");
    // Midday UTC, so the same date in nearly every time zone.
    assert_eq!(lines.next(), Some("2001-09-09 252 Sun September"));
}

#[wasm_bindgen_test]
fn random_is_seedable_and_in_range() {
    let program = r#"
import "std/random" as random;
fn main() {
    random::seed(42);
    print(random::random_int_range(1, 6));
    print(random::random_int_range(1, 6));
    let x = random::random();
    print(x >= 0.0 && x < 1.0);
}
"#;
    let first = run(program);
    let second = run(program);
    assert_eq!(first, second, "one seed draws one sequence");
    let lines: Vec<&str> = first.lines().collect();
    for n in &lines[..2] {
        assert!((1..=6).contains(&n.parse::<i64>().unwrap()), "{first}");
    }
    assert_eq!(lines[2], "true");
}

/// What the desktop's C library draws for seed 42, recorded from it: a seeded
/// run draws the same sequence on every target.
#[wasm_bindgen_test]
fn a_seeded_run_draws_what_a_desktop_draws() {
    let out = run(r#"
import "std/random" as random;
fn main() {
    random::seed(42);
    print(random::random_int_range(1, 100));
    print(random::random_int_range(1, 100));
    print(random::random_int());
    print(random::random());
    print(random::random_range(5.0, 10.0));
}
"#);
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        [
            "66",
            "60",
            "-4637449449345556370",
            "0.7491247460711747",
            "7.523193188244477"
        ]
    );
}

#[wasm_bindgen_test]
fn an_unseeded_generator_draws() {
    let out = run(r#"
import "std/random" as random;
fn main() {
    let x = random::random();
    print(x >= 0.0 && x < 1.0);
}
"#);
    assert_eq!(out.trim(), "true");
}

/// An artifact records the std bindings by name; the browser's runtime binds
/// them to its own functions at load.
#[wasm_bindgen_test]
fn an_artifact_that_imports_math_loads_and_runs() {
    let bytes = build_bytecode(
        String::from(
            r#"
import "std/math" as math;
fn root(x: float) -> float { return math::sqrt(x); }
fn main() {}
"#,
        ),
        "main.cdl",
        &ImportResolver::new(),
    )
    .expect("builds");
    let mut program = load_program(&bytes, &HostRegistry::new()).expect("loads");
    assert_eq!(
        program.call("root", &[Value::Float(81.0)]).expect("runs"),
        Value::Float(9.0)
    );
}

fn js_sys_now() -> f64 {
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = Date)]
        fn now() -> f64;
    }
    now()
}
