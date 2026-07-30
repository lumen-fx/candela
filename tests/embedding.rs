//! Integration tests for the candela embedding / library API (`Engine`/`Program`).
//!
//! These exercise the exact shape the author asked for: register typed host
//! functions, declare them in a `host "..."` block, compile a script that calls
//! them, and invoke script functions by name with marshalled arguments — with
//! state persisting between calls and errors surfaced as `Diagnostic` values.

use candela::{Engine, Value};
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
            Value::Null
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
    engine.register_host_fn_variadic("app", "log", |_args: &[Value]| Value::Null);
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
