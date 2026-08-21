//! Integration tests for the candela embedding / library API (`Engine`/`Program`).
//!
//! These exercise the embedding shape: register typed host
//! functions, declare them in a `host "..."` block, compile a script that calls
//! them, and invoke script functions by name with marshalled arguments, with
//! state persisting between calls and errors surfaced as `Diagnostic` values.

use candela::macros::{MacroError, scan_regions};
use candela::{Engine, HostError, HostType, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::rc::Rc;

/// Convenience for building a string-keyed record `Value::Map`.
fn record(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

/// Mirrors the author's snippet: `register_host_fn("app", "rows", |id: &str| ...)`
/// returning an `i64`, a `host "app" { int rows(string); }` block, and a script
/// function that calls `app::rows(...)` invoked via `program.call`.
#[test]
fn host_fn_roundtrip_matches_snippet() {
    let mut engine = Engine::new();
    engine.register_host_fn("app", "rows", |id: &str| id.len() as i64);

    let src = r#"
host "app" {
    int rows(string);
}

fn count(id) {
    return app::rows(id);
}

fn main() {}
"#;

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let result = program.call("count", &["board".into()]).expect("call ok");
    assert_eq!(result, Value::Int(5));

    // Different argument, same compiled specialization is reused.
    let result = program.call("count", &["a".into()]).expect("call ok");
    assert_eq!(result, Value::Int(1));
}

/// State established by one call must be visible to the next. Here the state
/// lives on the Rust side (a shared map two host functions read/write), which
/// is the canonical embedding pattern; the `Program` keeps the dispatch table
/// (and thus the captured `Rc`) alive across calls.
#[test]
fn state_persists_across_calls() {
    let rows: Rc<RefCell<HashMap<String, i64>>> = Rc::new(RefCell::new(HashMap::new()));

    let mut engine = Engine::new();
    {
        let rows = Rc::clone(&rows);
        engine.register_host_fn("app", "rows", move |id: &str| {
            *rows.borrow().get(id).unwrap_or(&0)
        });
    }
    {
        let rows = Rc::clone(&rows);
        engine.register_host_fn("app", "set_rows", move |id: String, n: i64| {
            rows.borrow_mut().insert(id, n);
        });
    }

    let src = r#"
host "app" {
    int rows(string);
    set_rows(string, int);
}

fn add(id, n) {
    app::set_rows(id, n);
    return app::rows(id);
}

fn get(id) {
    return app::rows(id);
}

fn main() {}
"#;

    let mut program = engine.compile(src, "main.cdl").expect("compiles");

    // First call sets state and reads it back.
    assert_eq!(
        program.call("add", &["board".into(), 5i64.into()]).unwrap(),
        Value::Int(5)
    );
    // A later, independent call sees the state the first one established.
    assert_eq!(
        program.call("get", &["board".into()]).unwrap(),
        Value::Int(5)
    );
    // Mutate again; persistence holds across the pair of calls.
    assert_eq!(
        program.call("add", &["board".into(), 8i64.into()]).unwrap(),
        Value::Int(8)
    );
    assert_eq!(
        program.call("get", &["board".into()]).unwrap(),
        Value::Int(8)
    );
}

/// Every supported scalar marshals in both directions.
#[test]
fn scalar_marshalling_roundtrips() {
    let mut engine = Engine::new();
    engine.register_host_fn("m", "add_one", |x: i64| x + 1);
    engine.register_host_fn("m", "half", |x: f64| x / 2.0);
    engine.register_host_fn("m", "negate", |b: bool| !b);
    engine.register_host_fn("m", "shout", |s: String| s.to_uppercase());

    let src = r#"
host "m" {
    int add_one(int);
    float half(float);
    bool negate(bool);
    string shout(string);
}

fn ai(x) { return m::add_one(x); }
fn hf(x) { return m::half(x); }
fn ng(b) { return m::negate(b); }
fn sh(s) { return m::shout(s); }
fn main() {}
"#;

    let mut program = engine.compile(src, "m.cdl").unwrap();
    assert_eq!(program.call("ai", &[41i64.into()]).unwrap(), Value::Int(42));
    assert_eq!(
        program.call("hf", &[9.0f64.into()]).unwrap(),
        Value::Float(4.5)
    );
    assert_eq!(
        program.call("ng", &[true.into()]).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        program.call("sh", &["hi".into()]).unwrap(),
        Value::String("HI".to_owned())
    );
}

/// A `host` function with no registered closure is a clean `Diagnostic`.
#[test]
fn missing_host_registration_is_a_diagnostic() {
    let engine = Engine::new();
    let src = r#"
host "app" {
    int rows(string);
}
fn main() {}
"#;
    let err = engine.compile(src, "main.cdl").err().unwrap();
    assert_eq!(err.code, "unregistered_host_fn");
    assert!(err.message.contains("app") && err.message.contains("rows"));
}

/// A registered closure whose signature disagrees with the `host` block is a
/// clean `Diagnostic`, not a panic.
#[test]
fn signature_mismatch_is_a_diagnostic() {
    let mut engine = Engine::new();
    // Declared `int rows(string)` but the closure takes an int.
    engine.register_host_fn("app", "rows", |_n: i64| 0i64);
    let src = r#"
host "app" {
    int rows(string);
}
fn main() {}
"#;
    let err = engine.compile(src, "main.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
}

/// Calling an unknown script function returns a `Diagnostic` rather than
/// aborting the process.
#[test]
fn unknown_script_fn_is_a_diagnostic() {
    let engine = Engine::new();
    let mut program = engine.compile("fn main() {}", "main.cdl").unwrap();
    let err = program.call("nope", &[]).unwrap_err();
    assert!(!err.code.is_empty());
    assert!(!err.message.is_empty());
}

/// A runtime error inside a called function surfaces as a `Diagnostic` and does
/// not corrupt the program for subsequent successful calls.
#[test]
fn runtime_error_surfaces_as_diagnostic() {
    let engine = Engine::new();
    let src = r"
fn boom(xs, i) {
    return xs[i];
}
fn ok(a, b) {
    return a + b;
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").unwrap();

    // Indexing the one-character string "x" at position 5 is an out-of-bounds
    // runtime error, which must come back as a diagnostic rather than abort.
    let err = program.call("boom", &["x".into(), 5i64.into()]);
    assert!(err.is_err(), "expected a runtime diagnostic");

    // The program remains usable for a subsequent good call.
    assert_eq!(
        program.call("ok", &[2i64.into(), 3i64.into()]).unwrap(),
        Value::Int(5)
    );
}

// ---------------------------------------------------------------------------
// ARRAY + MAP MARSHALLING
// ---------------------------------------------------------------------------

/// An array round-trips both as a host-fn argument and as a `Program::call`
/// return value.
#[test]
fn array_arg_and_return_roundtrip() {
    let mut engine = Engine::new();
    engine.register_host_fn("agg", "sum", |rows: Vec<i64>| rows.iter().sum::<i64>());

    let src = r#"
host "agg" {
    int sum(int[]);
}
fn total(xs) { return agg::sum(xs); }
fn make() { return [1, 2, 3, 4]; }
fn main() {}
"#;
    let mut program = engine.compile(src, "arr.cdl").unwrap();

    // Array passed from the host through a script fn into the host closure.
    let sum = program
        .call(
            "total",
            &[Value::Array(vec![10.into(), 20.into(), 5.into()])],
        )
        .unwrap();
    assert_eq!(sum, Value::Int(35));

    // Array returned by a script fn, read back out of the pools.
    let arr = program.call("make", &[]).unwrap();
    assert_eq!(
        arr,
        Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4)
        ])
    );
}

/// An array built by a host closure marshals into candela and back out.
#[test]
fn array_returned_by_host_fn() {
    let mut engine = Engine::new();
    engine.register_host_fn("nums", "seq", || vec![7i64, 8, 9]);

    let src = r#"
host "nums" {
    int[] seq();
}
fn get() { return nums::seq(); }
fn main() {}
"#;
    let mut program = engine.compile(src, "seq.cdl").unwrap();
    let got = program.call("get", &[]).unwrap();
    assert_eq!(
        got,
        Value::Array(vec![Value::Int(7), Value::Int(8), Value::Int(9)])
    );
}

/// A string-keyed map round-trips both directions.
#[test]
fn map_roundtrip_both_directions() {
    let mut engine = Engine::new();
    // Map argument: sum the values.
    engine.register_host_fn("cfg", "size", |m: BTreeMap<String, i64>| {
        m.values().sum::<i64>()
    });

    let src = r#"
host "cfg" {
    int size({string: int});
}
fn sz(m) { return cfg::size(m); }
fn conf() { return {"title": "Song", "artist": "Band"}; }
fn main() {}
"#;
    let mut program = engine.compile(src, "map.cdl").unwrap();

    // Map passed from the host into a host closure via a script fn.
    let mut m = BTreeMap::new();
    m.insert("a".to_owned(), Value::Int(1));
    m.insert("b".to_owned(), Value::Int(2));
    m.insert("c".to_owned(), Value::Int(4));
    let total = program.call("sz", &[Value::Map(m)]).unwrap();
    assert_eq!(total, Value::Int(7));

    // Map built by a script fn, read back as a Value::Map.
    let conf = program.call("conf", &[]).unwrap();
    assert_eq!(
        conf,
        record(&[
            ("artist", Value::String("Band".to_owned())),
            ("title", Value::String("Song".to_owned())),
        ])
    );
}

/// The shape Lumen's track list needs: an array of string-keyed records built
/// entirely on the host side, marshalled into candela and read back nested.
#[test]
fn nested_array_of_maps_track_list() {
    let mut engine = Engine::new();
    engine.register_host_fn("lib", "tracks", || -> Vec<BTreeMap<String, String>> {
        let mk = |title: &str, artist: &str, dur: &str, file: &str| {
            let mut r = BTreeMap::new();
            r.insert("title".to_owned(), title.to_owned());
            r.insert("artist".to_owned(), artist.to_owned());
            r.insert("dur".to_owned(), dur.to_owned());
            r.insert("file".to_owned(), file.to_owned());
            r
        };
        vec![
            mk("Intro", "Aphex", "83", "a.flac"),
            mk("Xtal", "Aphex", "294", "b.flac"),
        ]
    });

    let src = r#"
host "lib" {
    {string: string}[] tracks();
}
fn all() { return lib::tracks(); }
fn main() {}
"#;
    let mut program = engine.compile(src, "tracks.cdl").unwrap();
    let got = program.call("all", &[]).unwrap();

    let expected = Value::Array(vec![
        record(&[
            ("artist", Value::String("Aphex".to_owned())),
            ("dur", Value::String("83".to_owned())),
            ("file", Value::String("a.flac".to_owned())),
            ("title", Value::String("Intro".to_owned())),
        ]),
        record(&[
            ("artist", Value::String("Aphex".to_owned())),
            ("dur", Value::String("294".to_owned())),
            ("file", Value::String("b.flac".to_owned())),
            ("title", Value::String("Xtal".to_owned())),
        ]),
    ]);
    assert_eq!(got, expected);
}

/// A collection-typed signature mismatch (closure takes `string[]`, block
/// declares `int[]`) is a clean `Diagnostic`, not a panic.
#[test]
fn collection_signature_mismatch_is_a_diagnostic() {
    let mut engine = Engine::new();
    engine.register_host_fn("agg", "sum", |_rows: Vec<String>| 0i64);
    let src = r#"
host "agg" {
    int sum(int[]);
}
fn main() {}
"#;
    let err = engine.compile(src, "mismatch.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
}

// ---------------------------------------------------------------------------
// HOST NAMES THAT COLLIDE WITH BUILT-INS
// ---------------------------------------------------------------------------

/// A host function named like a built-in (`read`, `str`, `exists`, ...) is
/// typed from the `host` block it is declared in, not from the built-in that
/// shares its bare name. `gpio::read` returns `int` here, so the sum is
/// arithmetic.
#[test]
fn a_host_fn_named_like_a_builtin_keeps_its_declared_type() {
    let mut engine = Engine::new();
    engine.register_host_fn("gpio", "read", |pin: i64| pin);

    let src = r#"
host "gpio" {
    int read(int);
}
fn level(pin) { return gpio::read(pin) + 1; }
fn main() {}
"#;
    let mut program = engine.compile(src, "gpio.cdl").unwrap();
    assert_eq!(
        program.call("level", &[21i64.into()]).unwrap(),
        Value::Int(22)
    );
}

/// The same collision on every built-in name whose return type the inference
/// table pins, across the shapes a declaration can take.
#[test]
fn builtin_names_are_shadowed_by_their_host_declarations() {
    let mut engine = Engine::new();
    engine.register_host_fn("dev", "str", |n: i64| n * 2);
    engine.register_host_fn("dev", "exists", |n: i64| n + 1);
    engine.register_host_fn("dev", "range", |n: i64| format!("<{n}>"));
    engine.register_host_fn("dev", "argv", |n: f64| n / 2.0);

    let src = r#"
host "dev" {
    int str(int);
    int exists(int);
    string range(int);
    float argv(float);
}
fn doubled(n) { return dev::str(n) + 1; }
fn bumped(n) { return dev::exists(n) + 1; }
fn tagged(n) { return dev::range(n) + "!"; }
fn halved(n) { return dev::argv(n) + 0.5; }
fn main() {}
"#;
    let mut program = engine.compile(src, "dev.cdl").unwrap();
    assert_eq!(
        program.call("doubled", &[4i64.into()]).unwrap(),
        Value::Int(9)
    );
    assert_eq!(
        program.call("bumped", &[4i64.into()]).unwrap(),
        Value::Int(6)
    );
    assert_eq!(
        program.call("tagged", &[7i64.into()]).unwrap(),
        Value::String("<7>!".to_owned())
    );
    assert_eq!(
        program.call("halved", &[5.0f64.into()]).unwrap(),
        Value::Float(3.0)
    );
}

/// A variadic host function is not signature-checked, so its closure can hand
/// back a value of a type the block does not declare. Concatenating that value
/// is a runtime diagnostic naming what it turned out to be, and the program
/// stays usable afterwards.
#[test]
fn concatenating_a_non_string_is_a_diagnostic() {
    let mut engine = Engine::new();
    engine.register_host_fn_variadic("app", "label", |_args: &[Value]| Ok(Value::Int(7)));

    let src = r#"
host "app" {
    string label(...);
}
fn tagged() { return app::label("pin") + "!"; }
fn plain() { return "ok"; }
fn main() {}
"#;
    let mut program = engine.compile(src, "label.cdl").unwrap();

    let err = program.call("tagged", &[]).unwrap_err();
    assert_eq!(err.code, "not_a_string");
    assert!(err.message.contains("int"), "{}", err.message);

    assert_eq!(
        program.call("plain", &[]).unwrap(),
        Value::String("ok".to_owned())
    );
}

// ---------------------------------------------------------------------------
// HOST FUNCTIONS THAT FAIL
// ---------------------------------------------------------------------------

/// A closure that returns `Err` raises where the script called it. The
/// diagnostic names the function and carries what the host reported, and the
/// program is still usable afterwards.
#[test]
fn a_failing_host_fn_raises_at_the_call_site() {
    let mut engine = Engine::new();
    engine.register_host_fn("gpio", "read", |pin: i64| {
        if pin == 21 {
            Ok(1i64)
        } else {
            Err(HostError::new(format!("pin {pin} is not wired")))
        }
    });

    let src = r#"
host "gpio" {
    int read(int);
}
fn level(pin) { return gpio::read(pin); }
fn main() {}
"#;
    let mut program = engine.compile(src, "gpio.cdl").unwrap();

    let err = program.call("level", &[7i64.into()]).unwrap_err();
    assert_eq!(err.code, "host_fn_error");
    assert!(err.message.contains("gpio::read"), "{}", err.message);
    assert!(
        err.message.contains("pin 7 is not wired"),
        "{}",
        err.message
    );
    assert_eq!(err.filename, "gpio.cdl");
    assert!(err.span.start < err.span.end);

    assert_eq!(
        program.call("level", &[21i64.into()]).unwrap(),
        Value::Int(1)
    );
}

/// A function that returns nothing can fail too: `Result<(), HostError>` binds
/// to a declaration with no return type.
#[test]
fn a_void_host_fn_can_fail() {
    let mut engine = Engine::new();
    engine.register_host_fn("app", "save", |contents: &str| {
        if contents.is_empty() {
            Err(HostError::new("nothing to save"))
        } else {
            Ok(())
        }
    });

    let src = r#"
host "app" {
    save(string);
}
fn store(text) { app::save(text); }
fn main() {}
"#;
    let mut program = engine.compile(src, "save.cdl").unwrap();

    assert_eq!(
        program.call("store", &["rows".into()]).unwrap(),
        Value::Null
    );

    let err = program.call("store", &["".into()]).unwrap_err();
    assert_eq!(err.code, "host_fn_error");
    assert!(err.message.contains("app::save"), "{}", err.message);
}

/// The error is an ordinary candela runtime error, so a script handles it with
/// `try`/`catch` under the kind `host_fn_error`.
#[test]
fn a_script_catches_a_host_error() {
    let mut engine = Engine::new();
    engine.register_host_fn("gpio", "read", |_pin: i64| {
        Err::<i64, HostError>(HostError::new("the bus is down"))
    });

    let src = r#"
host "gpio" {
    int read(int);
}
fn level(pin) {
    try {
        return gpio::read(pin);
    } catch "host_fn_error" {
        return -1;
    }
}
fn main() {}
"#;
    let mut program = engine.compile(src, "gpio.cdl").unwrap();
    assert_eq!(
        program.call("level", &[7i64.into()]).unwrap(),
        Value::Int(-1)
    );
}

/// A variadic closure raises the same way a typed one does.
#[test]
fn a_failing_variadic_host_fn_raises() {
    let mut engine = Engine::new();
    engine.register_host_fn_variadic("app", "log", |args: &[Value]| {
        if args.is_empty() {
            Err(HostError::from("nothing to log"))
        } else {
            Ok(Value::Null)
        }
    });

    let src = r#"
host "app" {
    log(...);
}
fn quiet() { app::log(); }
fn loud() { app::log("hi"); }
fn main() {}
"#;
    let mut program = engine.compile(src, "log.cdl").unwrap();

    let err = program.call("quiet", &[]).unwrap_err();
    assert_eq!(err.code, "host_fn_error");
    assert!(err.message.contains("app::log"), "{}", err.message);
    assert!(err.message.contains("nothing to log"), "{}", err.message);

    assert_eq!(program.call("loud", &[]).unwrap(), Value::Null);
}

/// The signature a `Result`-returning closure is checked against is the one
/// inside the `Result`: a mismatch there is still a compile-time `Diagnostic`.
#[test]
fn a_fallible_closure_is_signature_checked_on_its_value_type() {
    let mut engine = Engine::new();
    engine.register_host_fn("gpio", "read", |_pin: i64| {
        Ok::<String, HostError>(String::new())
    });
    let src = r#"
host "gpio" {
    int read(int);
}
fn main() {}
"#;
    let err = engine.compile(src, "gpio.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
}

// ---------------------------------------------------------------------------
// SIGNATURES GIVEN AS DATA
// ---------------------------------------------------------------------------

/// A host that only knows its signatures at run time registers them as data.
/// The closure takes a slice, and the declaration is checked against the types
/// it was registered with, so the call is typed like any other.
#[test]
fn a_signature_given_as_data_binds_and_calls() {
    let mut engine = Engine::new();
    engine.register_host_fn_typed(
        "gpio",
        "read",
        vec![HostType::Int, HostType::String],
        HostType::Int,
        |args: &[Value]| {
            let pin = args[0].as_i64().unwrap_or(0);
            let mode = args[1].as_str().unwrap_or_default();
            Ok(Value::Int(if mode == "pullup" { pin + 1 } else { pin }))
        },
    );

    let src = r#"
host "gpio" {
    int read(int, string);
}
fn level(pin, mode) { return gpio::read(pin, mode) + 10; }
fn main() {}
"#;
    let mut program = engine.compile(src, "gpio.cdl").unwrap();
    assert_eq!(
        program
            .call("level", &[21i64.into(), "pullup".into()])
            .unwrap(),
        Value::Int(32)
    );
    assert_eq!(
        program
            .call("level", &[21i64.into(), "float".into()])
            .unwrap(),
        Value::Int(31)
    );
}

/// The declaration is held to the registered types, not waved through because
/// the closure is erased.
#[test]
fn a_declaration_that_disagrees_with_the_given_signature_is_a_diagnostic() {
    let mut engine = Engine::new();
    engine.register_host_fn_typed(
        "gpio",
        "read",
        vec![HostType::Int],
        HostType::Int,
        |_args: &[Value]| Ok(Value::Int(0)),
    );

    let src = r#"
host "gpio" {
    string read(int);
}
fn main() {}
"#;
    let err = engine.compile(src, "gpio.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
    assert!(err.message.contains("gpio::read"), "{}", err.message);
}

/// A signature given as data is a fixed one, so a `...` declaration does not
/// bind to it.
#[test]
fn a_given_signature_does_not_bind_to_a_variadic_declaration() {
    let mut engine = Engine::new();
    engine.register_host_fn_typed(
        "app",
        "log",
        vec![HostType::String],
        HostType::Unit,
        |_args: &[Value]| Ok(Value::Null),
    );

    let src = r#"
host "app" {
    log(...);
}
fn main() {}
"#;
    let err = engine.compile(src, "log.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
}

/// A closure registered this way raises like any other host function.
#[test]
fn a_given_signature_can_fail() {
    let mut engine = Engine::new();
    engine.register_host_fn_typed(
        "gpio",
        "read",
        vec![HostType::Int],
        HostType::Int,
        |args: &[Value]| match args[0].as_i64() {
            Some(21) => Ok(Value::Int(1)),
            Some(pin) => Err(HostError::new(format!("pin {pin} is not wired"))),
            None => Err(HostError::new("a pin is an int")),
        },
    );

    let src = r#"
host "gpio" {
    int read(int);
}
fn level(pin) { return gpio::read(pin); }
fn main() {}
"#;
    let mut program = engine.compile(src, "gpio.cdl").unwrap();

    let err = program.call("level", &[7i64.into()]).unwrap_err();
    assert_eq!(err.code, "host_fn_error");
    assert!(
        err.message.contains("pin 7 is not wired"),
        "{}",
        err.message
    );

    assert_eq!(
        program.call("level", &[21i64.into()]).unwrap(),
        Value::Int(1)
    );
}

// ---------------------------------------------------------------------------
// VARIADIC HOST FUNCTIONS
// ---------------------------------------------------------------------------

/// A `...` host fn is callable with any argument count and any mix of types;
/// the closure receives them all as a `&[Value]` slice.
#[test]
fn variadic_host_fn_receives_all_args() {
    let calls: Rc<RefCell<Vec<Vec<Value>>>> = Rc::new(RefCell::new(Vec::new()));
    let mut engine = Engine::new();
    {
        let calls = Rc::clone(&calls);
        engine.register_host_fn_variadic("app", "log", move |args: &[Value]| {
            calls.borrow_mut().push(args.to_vec());
            Ok(Value::Null)
        });
    }

    let src = r#"
host "app" {
    log(...);
}
fn none() { app::log(); }
fn one() { app::log("hi"); }
fn mixed() { app::log("tag", 42, true); }
fn main() {}
"#;
    let mut program = engine.compile(src, "log.cdl").expect("compiles");

    program.call("none", &[]).unwrap();
    program.call("one", &[]).unwrap();
    program.call("mixed", &[]).unwrap();

    let calls = calls.borrow();
    assert_eq!(calls.len(), 3);
    assert!(calls[0].is_empty());
    assert_eq!(calls[1], vec![Value::String("hi".to_owned())]);
    assert_eq!(
        calls[2],
        vec![
            Value::String("tag".to_owned()),
            Value::Int(42),
            Value::Bool(true),
        ]
    );
}

/// A variadic closure bound to a fixed (non-`...`) declaration is a clean
/// `Diagnostic`, not a silent mismatch.
#[test]
fn variadic_closure_with_fixed_declaration_is_a_diagnostic() {
    let mut engine = Engine::new();
    engine.register_host_fn_variadic("app", "log", |_args: &[Value]| Ok(Value::Null));
    let src = r#"
host "app" {
    log(string);
}
fn main() {}
"#;
    let err = engine.compile(src, "log.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
}

/// A `...` declaration bound to an ordinary fixed closure is likewise a clean
/// `Diagnostic`.
#[test]
fn fixed_closure_with_variadic_declaration_is_a_diagnostic() {
    let mut engine = Engine::new();
    engine.register_host_fn("app", "log", |_s: String| {});
    let src = r#"
host "app" {
    log(...);
}
fn main() {}
"#;
    let err = engine.compile(src, "log.cdl").err().unwrap();
    assert_eq!(err.code, "host_fn_signature_mismatch");
}

/// The Lumen `derive` shape compiles today with no new candela feature: a fixed
/// `(string, string[], string)` signature, an array literal argument, and a
/// function referenced by name. Registered state persists so the host can
/// invoke the named function later.
#[test]
fn derive_shape_array_literal_and_named_fn() {
    let recorded: Rc<RefCell<Vec<(String, Vec<String>, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let mut engine = Engine::new();
    {
        let recorded = Rc::clone(&recorded);
        engine.register_host_fn(
            "lumen",
            "derive",
            move |name: String, deps: Vec<String>, f: String| {
                recorded.borrow_mut().push((name, deps, f));
            },
        );
    }

    let src = r#"
host "lumen" {
    derive(string, string[], string);
}
fn on_start() {
    lumen::derive("total", ["price", "qty"], "compute_total");
}
fn compute_total(price, qty) { return price * qty; }
fn main() {}
"#;
    let mut program = engine.compile(src, "derive.cdl").expect("compiles");
    program.call("on_start", &[]).unwrap();

    let recorded = recorded.borrow();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "total");
    assert_eq!(recorded[0].1, vec!["price".to_owned(), "qty".to_owned()]);
    assert_eq!(recorded[0].2, "compute_total");

    // The host can invoke the referenced function by name later.
    let out = program
        .call("compute_total", &[6i64.into(), 7i64.into()])
        .unwrap();
    assert_eq!(out, Value::Int(42));
}

// ---------------------------------------------------------------------------
// NESTED HOST CALLS
// ---------------------------------------------------------------------------

/// A host call nested inside another call expression binds its own arguments.
/// The VM collects `StoreFuncArg` operands in one scratch list that a call
/// consumes and clears, so the operands of an outer call have to be stored in
/// one run directly before it. Each argument also needs its own register:
/// `both_args_nested` subtracts rather than adds, so it catches a later
/// argument reusing the register of an earlier one.
#[test]
fn nested_host_call_binds_its_own_args() {
    let mut engine = Engine::new();
    engine.register_host_fn("m", "add", |a: i64, b: i64| a + b);
    engine.register_host_fn("m", "sub", |a: i64, b: i64| a - b);
    engine.register_host_fn("m", "one", || 1i64);
    engine.register_host_fn("m", "three", || 3i64);
    engine.register_host_fn("m", "double", |x: i64| x * 2);

    let src = r#"
host "m" {
    int add(int, int);
    int sub(int, int);
    int one();
    int three();
    int double(int);
}

fn last_arg_nested(x) { return m::add(x, m::one()); }
fn first_arg_nested(x) { return m::add(m::one(), x); }
fn both_args_nested() { return m::sub(m::three(), m::one()); }
fn deep(x) { return m::add(m::double(m::add(x, m::one())), x); }
fn split(x) {
    let n = m::one();
    return m::add(x, n);
}
fn main() {}
"#;

    let mut program = engine.compile(src, "nested.cdl").expect("compiles");
    assert_eq!(
        program.call("split", &[10i64.into()]).unwrap(),
        Value::Int(11)
    );
    assert_eq!(
        program.call("last_arg_nested", &[10i64.into()]).unwrap(),
        Value::Int(11)
    );
    assert_eq!(
        program.call("first_arg_nested", &[10i64.into()]).unwrap(),
        Value::Int(11)
    );
    assert_eq!(
        program.call("both_args_nested", &[]).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        program.call("deep", &[10i64.into()]).unwrap(),
        Value::Int(32)
    );
}

/// The same rule covers the builtin methods, whose arguments the VM pops off
/// that scratch list: a host call nested in a method argument must not consume
/// the operands the method already stored.
#[test]
fn nested_host_call_in_method_arg() {
    let mut engine = Engine::new();
    engine.register_host_fn("m", "pick", |s: String| s);

    let src = r#"
host "m" {
    string pick(string);
}

fn swap(s) { return s.replace(m::pick("a"), "b"); }
fn find_in(xs) { return xs.contains(m::pick("y")); }
fn main() {}
"#;

    let mut program = engine.compile(src, "method.cdl").expect("compiles");
    assert_eq!(
        program.call("swap", &["cat".into()]).unwrap(),
        Value::String("cbt".to_owned())
    );
    assert_eq!(
        program
            .call(
                "find_in",
                &[Value::Array(vec!["x".into(), "y".into(), "z".into()])]
            )
            .unwrap(),
        Value::Bool(true)
    );
}

/// A parameter carrying a `: Type` annotation is the natural shape for a
/// function a host calls: there is no in-script call site to infer from. The
/// annotation is checked against the marshalled argument rather than rejected.
#[test]
fn annotated_params_accept_matching_host_arguments() {
    let engine = Engine::new();

    let src = r"
fn banner(label: string, pad: int) -> int {
    return label.len() + pad;
}

fn main() {}
";

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let result = program
        .call("banner", &["title".into(), 2.into()])
        .expect("call ok");
    assert_eq!(result, Value::Int(7));
}

/// The same annotation rejects an argument of the wrong type, and the host sees
/// it as a `Diagnostic` rather than a corrupted run.
#[test]
fn annotated_params_reject_mismatched_host_arguments() {
    let engine = Engine::new();

    let src = r"
fn banner(label: string) -> int {
    return label.len();
}

fn main() {}
";

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let err = program
        .call("banner", &[7.into()])
        .expect_err("argument type is checked");
    assert_eq!(err.code, "argument_type_mismatch");
    assert!(err.message.contains("string"));
}

/// Leaving a host-called function's parameters un-annotated still works: the
/// checker takes the types from the marshalled arguments.
#[test]
fn unannotated_params_still_take_host_argument_types() {
    let engine = Engine::new();

    let src = r"
fn width(label) {
    return label.len();
}

fn main() {}
";

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let result = program.call("width", &["abcd".into()]).expect("call ok");
    assert_eq!(result, Value::Int(4));
}

// ---------------------------------------------------------------------------
// Macros
//
// `name!( ... )` is a raw region the embedder gives meaning to. The engine
// registers an expander for a name; the region body reaches it untouched and
// the candela source it returns is parsed at the macro.
// ---------------------------------------------------------------------------

/// A markup-flavoured stub standing in for what a UI host would register:
/// the region is its own little language, and the expansion is candela.
fn markup_engine() -> Engine {
    let mut engine = Engine::new();
    engine.register_macro("lmn", |body: &str| {
        let tags = body.matches('<').count();
        Ok::<String, MacroError>(format!("\"{}\" + \"{tags}\"", body.trim()))
    });
    engine
}

#[test]
fn a_registered_macro_expands_where_an_expression_goes() {
    let engine = markup_engine();

    let src = r"
fn markup() -> string {
    return lmn!(<p>hello</p>);
}

fn main() {}
";

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let value = program.call("markup", &[]).expect("call ok");
    assert_eq!(value, Value::String(String::from("<p>hello</p>2")));
}

#[test]
fn a_macro_expansion_is_an_argument_like_any_other() {
    let mut engine = markup_engine();
    engine.register_host_fn("app", "width", |markup: &str| markup.len() as i64);

    let src = r#"
host "app" {
    int width(string);
}

fn measure() -> int {
    return app::width(lmn!(<b/>));
}

fn main() {}
"#;

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    assert_eq!(
        program.call("measure", &[]).expect("call ok"),
        Value::Int(5)
    );
}

#[test]
fn an_unregistered_macro_fails_the_compile() {
    let engine = Engine::new();
    let err = engine
        .compile("fn main() { let m = lmn!(<p/>); }", "main.cdl")
        .err()
        .expect("nothing gives lmn! a meaning");
    assert_eq!(err.code, "unknown_macro");
    assert!(err.message.contains("lmn!"), "{}", err.message);
}

#[test]
fn unknown_macros_can_be_allowed_for_tooling() {
    let mut engine = Engine::new();
    engine.allow_unknown_macros(true);

    let src = r"
fn markup() {
    return lmn!(<p/>);
}

fn main() {}
";

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    assert_eq!(program.call("markup", &[]).expect("call ok"), Value::Null);
}

#[test]
fn an_expander_error_lands_on_the_offending_byte() {
    let mut engine = Engine::new();
    engine.register_macro("lmn", |body: &str| {
        Err::<String, _>(MacroError::at("unclosed tag", body.find("<span").unwrap()))
    });

    let src = "fn main() { let m = lmn!(<div><span>); }";
    let err = engine
        .compile(src, "main.cdl")
        .err()
        .expect("the expander refused");
    assert_eq!(err.code, "macro_expansion_failed");
    assert!(err.message.contains("unclosed tag"), "{}", err.message);
    assert_eq!(err.span.start, src.find("<span").unwrap());
    assert_eq!(err.filename, "main.cdl");
}

#[test]
fn scan_regions_finds_what_the_lexer_finds() {
    let src = r#"
fn main() {
    let quoted = "lmn!(not a macro)";
    // lmn!(not a macro either)
    let one = lmn!(<p>(a)</p>);
    let two = lmn!(<b/>);
}
"#;

    let regions = scan_regions(src, "lmn");
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].body, "<p>(a)</p>");
    assert_eq!(regions[1].body, "<b/>");
    assert_eq!(&src[regions[0].span.clone()], "lmn!(<p>(a)</p>)");
    assert_eq!(&src[regions[1].body_start..regions[1].span.end - 1], "<b/>");

    // What the scanner reports is what compiling the same source expands.
    let mut engine = Engine::new();
    engine.register_macro("lmn", |body: &str| {
        Ok::<String, MacroError>(format!("\"{body}\""))
    });
    engine.compile(src, "main.cdl").expect("compiles");
}

/// A function reached through `Program::call` is specialized after the program
/// was compiled, so the generic declarations it uses have to still be there.
#[test]
fn a_call_into_a_generic_function_specializes_after_compilation() {
    let engine = Engine::new();

    let src = r"
struct Cell<T> {
    value: T,
}

impl Cell<T> {
    fn get(self) -> T {
        return self.value;
    }
}

fn wrap<T>(x) {
    return Cell<T>{ value: x };
}

fn twice(n) {
    return wrap<int>(n).get() * 2;
}

fn main() {}
";

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let result = program.call("twice", &[21i64.into()]).expect("call ok");
    assert_eq!(result, Value::Int(42));
}
