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

/// A host namespace is reached with a dot as well as with `::`, including for a
/// function that returns nothing and is called as a statement.
#[test]
fn host_fn_reached_through_a_dot() {
    let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);

    let mut engine = Engine::new();
    engine.register_host_fn("app", "rows", |id: &str| id.len() as i64);
    engine.register_host_fn("app", "note", move |line: &str| {
        recorder.borrow_mut().push(line.to_owned());
    });

    let src = r#"
host "app" {
    int rows(string);
    note(string);
}

fn count(id: string) -> int {
    app.note(id);
    return app.rows(id);
}

fn both(id: string) -> bool {
    return app.rows(id) == app::rows(id);
}

fn main() {}
"#;

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    assert_eq!(
        program.call("count", &["board".into()]).expect("call ok"),
        Value::Int(5)
    );
    assert_eq!(
        program.call("both", &["board".into()]).expect("call ok"),
        Value::Bool(true)
    );
    assert_eq!(seen.borrow().as_slice(), ["board".to_owned()]);
}

/// A variable takes the name back: with `app` bound to a value, `app.rows(id)`
/// is that value's own method and the host block is reachable only through
/// `::`.
#[test]
fn variable_shadows_the_host_block() {
    let mut engine = Engine::new();
    engine.register_host_fn("app", "rows", |id: &str| id.len() as i64);

    let src = r#"
host "app" {
    int rows(string);
}

struct Board { offset: int }

impl Board {
    fn rows(self, id: string) -> int {
        return self.offset + id.len();
    }
}

fn count(id: string) -> int {
    let app = Board { offset: 42 };
    return app.rows(id);
}

fn host_count(id: string) -> int {
    let app = Board { offset: 42 };
    return app::rows(id);
}

fn main() {}
"#;

    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    assert_eq!(
        program.call("count", &["board".into()]).expect("call ok"),
        Value::Int(47)
    );
    assert_eq!(
        program
            .call("host_count", &["board".into()])
            .expect("call ok"),
        Value::Int(5)
    );
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

/// `compile` compiles every fully annotated function at its declared parameter
/// types, so a body error in one `main` never calls is reported by `compile`.
#[test]
fn an_annotated_function_is_checked_at_compile() {
    let engine = Engine::new();
    let err = engine
        .compile(
            "fn on_click(id: string) { nope(); }\nfn main() {}\n",
            "main.cdl",
        )
        .err()
        .expect("a body that does not compile must fail the compile");
    assert!(err.message.contains("nope"), "{}", err.message);

    // A struct-typed parameter is an annotation the compiler can specialise on
    // even though no host `Value` fits it, so it is checked too.
    let err = engine
        .compile(
            "struct Point { x: int, y: int }\nfn draw(p: Point) { nope(); }\nfn main() {}\n",
            "main.cdl",
        )
        .err()
        .expect("a struct-typed parameter is checked even though it is not exported");
    assert!(err.message.contains("nope"), "{}", err.message);

    let mut program = engine
        .compile(
            "fn double(x: int) -> int { return x * 2; }\nfn main() {}\n",
            "main.cdl",
        )
        .expect("a sound annotated function compiles");
    assert_eq!(
        program.call("double", &[Value::Int(21)]).unwrap(),
        Value::Int(42)
    );
}

/// A bare parameter has no declared type to compile against, so such a function
/// stays lazy and is first checked by the call that reaches it.
#[test]
fn a_bare_parameter_function_is_checked_at_the_call() {
    let engine = Engine::new();
    let mut program = engine
        .compile("fn on_click(id) { nope(); }\nfn main() {}\n", "main.cdl")
        .expect("nothing to check without a call site");
    let err = program
        .call("on_click", &[Value::String("x".to_owned())])
        .unwrap_err();
    assert!(err.message.contains("nope"), "{}", err.message);
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

/// A compile diagnostic thrown while lazily specializing a called function
/// (here: a call into a host function no `host` block declares) must not
/// corrupt the resident compiler/VM state. A prior successful call into the
/// same program must still work after it.
///
/// `a` and `b` take a bare parameter so `Engine::compile` leaves them for the
/// call to specialize; a fully annotated function is compiled up front and the
/// diagnostic would come out of `compile` instead.
#[test]
fn a_diagnostic_mid_call_does_not_corrupt_a_later_call() {
    let mut engine = Engine::new();
    engine.register_host_fn("lumen", "ping", |_n: i64| {});
    let src = r#"
host "lumen" {
    ping(int);
}

struct Item {
    title: string,
}

fn helper(item) {
    return item.title;
}

fn a(lazy) {
    let items = [];
    items.push(Item { title: "one" });
    items.push(Item { title: "two" });
    let t = helper(items[0]);
    lumen::no_such_builtin(0.7);
    return t;
}

fn b(lazy) {
    let items = [];
    items.push(Item { title: "three" });
    items.push(Item { title: "four" });
    let t = helper(items[0]);
    let out = "";
    for item in items {
        out = out + item.title;
    }
    return out + t;
}

fn main() {}
"#;
    let mut program = engine.compile(src, "main.cdl").unwrap();

    let err = program.call("a", &[Value::Null]);
    assert!(
        err.is_err(),
        "expected a diagnostic from the undeclared call"
    );

    // The program must remain usable for a later call, even one that reuses a
    // function specialization ("helper") the aborted call compiled partway.
    assert_eq!(
        program.call("b", &[Value::Null]).unwrap(),
        Value::String("threefourthree".to_owned())
    );
}

/// The same shape as `a_diagnostic_mid_call_does_not_corrupt_a_later_call`,
/// stripped of every loop, so a stale specialization address left behind by
/// the aborted call lands on the very next instruction the second call runs
/// instead of somewhere it can only be found by chance. Before the fix, this
/// is what turned up the VM's `debug_assert!(self.is_struct() ||
/// self.is_enum())` in `as_struct()`: the second call's `CallFunc` jumped to
/// the address the aborted call's compile recorded for `helper`, which
/// `self.instructions` never actually held, and read whatever landed there as
/// `helper`'s struct-typed parameter. `a` and `b` take a bare parameter for the
/// same reason as in that test.
#[test]
fn a_diagnostic_mid_call_does_not_leave_a_stale_specialization_address() {
    let mut engine = Engine::new();
    engine.register_host_fn("lumen", "ping", |_n: i64| {});
    let src = r#"
host "lumen" {
    ping(int);
}

struct Item {
    title: string,
}

fn helper(item) {
    return item.title;
}

fn a(lazy) {
    let x = Item { title: "one" };
    let t = helper(x);
    lumen::no_such_builtin(0.7);
    return t;
}

fn b(lazy) {
    let y = Item { title: "two" };
    return helper(y);
}

fn main() {}
"#;
    let mut program = engine.compile(src, "main.cdl").unwrap();

    let err = program.call("a", &[Value::Null]);
    assert!(
        err.is_err(),
        "expected a diagnostic from the undeclared call"
    );

    assert_eq!(
        program.call("b", &[Value::Null]).unwrap(),
        Value::String("two".to_owned())
    );
}

/// A closure literal hoists to a fresh entry in the function table the first
/// time it is reached. An aborted `Program::call` used to leave that entry
/// behind while a truncate elsewhere removed the saved-register slot pushed
/// with it, so the next call's own closure took a function id one past where
/// its register list had landed and panicked instead of running. `a` and `b`
/// take a bare parameter for the same reason as in
/// `a_diagnostic_mid_call_does_not_corrupt_a_later_call`.
#[test]
fn a_diagnostic_mid_call_does_not_desync_a_later_closure() {
    let mut engine = Engine::new();
    engine.register_host_fn("lumen", "ping", |_n: i64| {});
    let src = r#"
host "lumen" {
    ping(int);
}

fn a(lazy) {
    let f = fn(x) { return x + 1; };
    let t = f(1);
    lumen::no_such_builtin(0.7);
    return t;
}

fn b(lazy) {
    let g = fn(y) { return y + 2; };
    return g(3);
}

fn main() {}
"#;
    let mut program = engine.compile(src, "main.cdl").unwrap();

    let err = program.call("a", &[Value::Null]);
    assert!(
        err.is_err(),
        "expected a diagnostic from the undeclared call"
    );

    assert_eq!(program.call("b", &[Value::Null]).unwrap(), Value::Int(5));
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

/// An enum a script function returns reaches the host as the variant it holds,
/// payload included, through the resident compiler path.
#[test]
fn enum_return_names_its_variant() {
    let engine = Engine::new();

    let src = r"
enum Shape {
    Dot,
    Line(int, int),
    Group(Shape[]),
}

fn dot() -> Shape { return Shape::Dot; }
fn line(a: int, b: int) -> Shape { return Shape::Line(a, b); }
fn pair() -> Shape { return Shape::Group([Shape::Dot, Shape::Line(1, 2)]); }

fn main() {}
";
    let mut program = engine.compile(src, "enums.cdl").unwrap();

    assert_eq!(
        program.call("dot", &[]).unwrap(),
        Value::Enum {
            variant: "Dot".to_owned(),
            payload: Vec::new(),
        }
    );
    assert_eq!(
        program.call("line", &[5.into(), 6.into()]).unwrap(),
        Value::Enum {
            variant: "Line".to_owned(),
            payload: vec![Value::Int(5), Value::Int(6)],
        }
    );
    assert_eq!(
        program.call("pair", &[]).unwrap(),
        Value::Enum {
            variant: "Group".to_owned(),
            payload: vec![Value::Array(vec![
                Value::Enum {
                    variant: "Dot".to_owned(),
                    payload: Vec::new(),
                },
                Value::Enum {
                    variant: "Line".to_owned(),
                    payload: vec![Value::Int(1), Value::Int(2)],
                },
            ])],
        }
    );
}

/// An enum goes outward only. Handed back as an argument it is refused, in
/// every position: an `any` parameter that accepts every other value, and
/// inside a list whose element type was never pinned. The report names the
/// variant, and the program stays callable afterwards.
#[test]
fn an_enum_argument_is_refused() {
    let engine = Engine::new();

    let src = r"
enum Shape {
    Dot,
    Line(int, int),
}

fn line(a: int, b: int) -> Shape { return Shape::Line(a, b); }
fn echo(x: any) -> string { return str(x); }
fn first(xs: any[]) -> string { return str(xs[0]); }

fn main() {}
";
    let mut program = engine.compile(src, "enums.cdl").unwrap();
    let line = program.call("line", &[5.into(), 6.into()]).unwrap();

    let err = program
        .call("echo", std::slice::from_ref(&line))
        .expect_err("an enum is no argument, `any` included");
    assert_eq!(err.code, "argument_type_mismatch");
    assert!(
        err.message.contains("enum variant Line"),
        "the report names the variant: {}",
        err.message
    );

    let err = program
        .call("first", &[Value::Array(vec![line])])
        .expect_err("an enum inside a list is refused too");
    assert_eq!(err.code, "argument_type_mismatch");

    assert_eq!(
        program.call("echo", &["ok".into()]).unwrap(),
        Value::String(String::from("ok"))
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

// ---------------------------------------------------------------------------
// OUT-OF-RANGE INDICES
// ---------------------------------------------------------------------------
//
// `runtime_error_surfaces_as_diagnostic` above covers the base case: a plain
// out-of-range read comes back as a `Diagnostic` instead of aborting. The
// tests below pin the same channel across the other shapes an index can take:
// negative, a write target, a generic element type, a call several recursion
// frames deep, and a two-call sequence shaped like a reactive derivation that
// re-reads a list after a lookup misses.

/// A negative index is out of range the same way one past the end is; the
/// diagnostic names it as such rather than wrapping or panicking.
#[test]
fn a_negative_index_is_a_diagnostic() {
    let engine = Engine::new();
    let src = r"
fn last_of(xs) {
    return xs[-1];
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let err = program
        .call("last_of", &[Value::Array(vec![1i64.into()])])
        .unwrap_err();
    assert_eq!(err.code, "index_out_of_bounds");
}

/// Assigning through an out-of-range index is the same bounds check as
/// reading through one.
#[test]
fn an_out_of_range_write_is_a_diagnostic() {
    let engine = Engine::new();
    let src = r"
fn set_at(xs, i, v) {
    xs[i] = v;
    return xs;
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let err = program
        .call(
            "set_at",
            &[Value::Array(vec![1i64.into()]), 1i64.into(), 9i64.into()],
        )
        .unwrap_err();
    assert_eq!(err.code, "index_out_of_bounds");
}

/// A string indexes the same way a list does, for both a read and a write
/// through the index.
#[test]
fn a_string_index_read_and_write_out_of_range_are_diagnostics() {
    let engine = Engine::new();
    let src = r"
fn char_at(s, i) {
    return s[i];
}
fn set_char(s, i, c) {
    s[i] = c;
    return s;
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let read_err = program
        .call("char_at", &["a".into(), 1i64.into()])
        .unwrap_err();
    assert_eq!(read_err.code, "index_out_of_bounds");
    let write_err = program
        .call("set_char", &["a".into(), 1i64.into(), "z".into()])
        .unwrap_err();
    assert_eq!(write_err.code, "index_out_of_bounds");
}

/// A slice whose bounds fall outside the collection is its own error kind,
/// distinct from a single out-of-range index, but the same non-panicking
/// channel.
#[test]
fn a_slice_out_of_range_is_a_diagnostic() {
    let engine = Engine::new();
    let src = r"
fn tail(xs, a, b) {
    return xs[a..b];
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let err = program
        .call(
            "tail",
            &[Value::Array(vec![1i64.into()]), 0i64.into(), 5i64.into()],
        )
        .unwrap_err();
    assert_eq!(err.code, "slice_out_of_bounds");
}

/// A generic function's element type does not change how its index is
/// bounds-checked: the check runs on the array at the register the
/// specialization compiled, whatever `T` turned out to be.
#[test]
fn a_generic_functions_index_is_bounds_checked() {
    let engine = Engine::new();
    let src = r"
fn at<T>(xs: T[], i: int) -> T {
    return xs[i];
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let err = program
        .call("at", &[Value::Array(vec![1i64.into()]), 1i64.into()])
        .unwrap_err();
    assert_eq!(err.code, "index_out_of_bounds");
}

/// The same check applies however deep the call is when it runs: a bound hit
/// several recursion frames down still raises through the call frames back to
/// `Program::call` as a diagnostic, not a panic that unwinds past them.
#[test]
fn a_recursive_calls_index_is_bounds_checked() {
    let engine = Engine::new();
    let src = r"
fn rec(xs: int[], i: int, depth: int) -> int {
    if depth <= 0 {
        return xs[i];
    }
    return rec(xs, i, depth - 1);
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let err = program
        .call(
            "rec",
            &[Value::Array(vec![1i64.into()]), 1i64.into(), 5i64.into()],
        )
        .unwrap_err();
    assert_eq!(err.code, "index_out_of_bounds");
}

/// Mirrors a reactive derivation that looks a current selection up by id
/// (returning -1 on a miss) and indexes the list with the result: the first
/// call is an ordinary hit, and the second, on the same resident `Program`, is
/// a miss that feeds a negative index straight into the list. Both come back
/// as values, not panics, and the diagnostic on the second call does not
/// disturb the state a later call would read.
#[test]
fn a_lookup_miss_feeding_a_negative_index_is_a_diagnostic() {
    let engine = Engine::new();
    let src = r"
fn find_idx(list, id) {
    let i = 0;
    while i < list.len() {
        if list[i] == id { return i; }
        i += 1;
    }
    return -1;
}
fn current(list, id) {
    let idx = find_idx(list, id);
    return list[idx];
}
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    let list = Value::Array(vec![1i64.into(), 2i64.into()]);

    let hit = program
        .call("current", &[list.clone(), 1i64.into()])
        .unwrap();
    assert_eq!(hit, Value::Int(1));

    let miss = program.call("current", &[list, 99i64.into()]).unwrap_err();
    assert_eq!(miss.code, "index_out_of_bounds");
}

/// A body compiled from an imported module declares into that module's scope,
/// not the entry file's: a closure bound by `let` registers under its name
/// there. An aborted `Program::call` used to leave that registration behind,
/// because the rollback truncated the entry file's scope alone. A later call
/// that passed the name to a higher-order function then took it for a function
/// reference, and the id it carried had since been handed to an unrelated
/// closure, so `probe` returned that closure's value instead of reporting a
/// name nothing declares.
#[test]
fn a_diagnostic_mid_call_does_not_leave_a_name_in_an_imported_scope() {
    let dir =
        std::env::temp_dir().join(format!("candela_embed_import_scope_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(
        dir.join("helper.cdl"),
        "fn leak(lazy) {\n    \
             let sneaky = fn(y) { return y; };\n    \
             lumen::no_such_builtin(0.7);\n    \
             return sneaky(1);\n\
         }\n\
         fn apply(f) {\n    \
             return f(1);\n\
         }\n\
         fn probe() {\n    \
             return apply(sneaky);\n\
         }\n",
    )
    .expect("write module");
    let entry = dir.join("prog.cdl");

    let mut engine = Engine::new();
    engine.register_host_fn("lumen", "ping", |_n: i64| {});
    let mut program = engine
        .compile(
            "host \"lumen\" {\n    ping(int);\n}\n\n\
             import \"helper.cdl\";\n\n\
             fn refill(lazy) {\n    \
                 let g = fn(z) { return 42; };\n    \
                 return g(1);\n\
             }\n\n\
             fn main() {}\n",
            entry.to_str().expect("utf-8 scratch path"),
        )
        .expect("the entry file compiles");

    assert!(
        program.call("leak", &[Value::Null]).is_err(),
        "expected a diagnostic from the undeclared call"
    );
    // A call that succeeds now takes the function id the aborted one gave up,
    // so a `sneaky` the module's scope kept no longer looks dangling: it names
    // this closure.
    assert_eq!(
        program.call("refill", &[Value::Null]).unwrap(),
        Value::Int(42)
    );

    // `sneaky` is a local of the aborted body, never a name the module
    // declares, so looking it up has to fail rather than reach whichever
    // function has landed on that id since.
    assert!(
        program.call("probe", &[]).is_err(),
        "the aborted call left `sneaky` in the module's scope"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `Engine::compile` compiles an entry point for every fully annotated function
/// in the file, which is the same pass `candela check` runs. A generic function
/// whose type parameter no argument pins is checked there with no type argument
/// named, so it has to compile with that parameter standing for `any` rather
/// than reporting a type it cannot resolve.
#[test]
fn a_generic_function_with_an_unnamed_type_parameter_passes_the_check() {
    let src = r"
struct Signal<T> {
    name: string,
}

fn signal<T>(name: string) -> Signal<T> {
    return Signal<T>{ name: name };
}

impl Signal<any> {
    fn width(self) -> int {
        return self.name.len();
    }
}

fn read(id: string) -> int {
    return signal(id).width();
}

fn main() {}
";

    let engine = Engine::new();
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    assert_eq!(program.call("read", &["abc".into()]), Ok(Value::Int(3)));
}

/// `Engine::compile` hands back the warnings the compile raised on the
/// program. The `any` attempt that failed is undone, so the program stays
/// callable.
#[test]
fn compile_warnings_reach_the_program() {
    let engine = Engine::new();
    let src = "
        fn on_click(id) { let n = 1; n.uppercase(); }
        fn echo(x) { return x; }
        fn main() {}
    ";
    let mut program = engine
        .compile(src, "warn.cdl")
        .expect("warnings do not fail a compile");
    let codes: Vec<&str> = program.warnings().iter().map(|w| w.code.as_str()).collect();
    assert_eq!(
        codes,
        [
            "unannotated_host_parameter",
            "no_host_entry_point",
            "unannotated_host_parameter"
        ]
    );
    assert!(program.warnings()[2].message.contains("echo"));
    assert_eq!(
        program.call("echo", &[Value::Int(2)]).unwrap(),
        Value::Int(2)
    );
    assert!(program.call("on_click", &[Value::Int(1)]).is_err());
    assert_eq!(
        program
            .call("echo", &[Value::String("s".to_owned())])
            .unwrap(),
        Value::String("s".to_owned())
    );
}

/// `Engine::check` binds a `dylib` block from its declared signatures and
/// opens nothing, so a script whose library is not built yet checks, where
/// `compile` needs the library.
#[test]
fn check_binds_a_dylib_without_its_library() {
    let engine = Engine::new();
    let src = "
        dylib \"nosuch_lib_xyz\" {
            int f(int);
        }
        fn g(x: int) -> int { return nosuch_lib_xyz::f(x) + 1; }
        fn main() {}
    ";
    assert_eq!(engine.check(src, "lib.cdl"), Ok(Vec::new()));
    let error = engine
        .compile(src, "lib.cdl")
        .err()
        .expect("a compile opens the library");
    assert_eq!(error.code, "cannot_load_dynlib");
}

/// A library named by a path is called through the namespace its file name
/// gives, and a signature returning a struct or taking an array binds the same
/// way a scalar one does.
#[test]
fn check_binds_a_path_dylib_and_its_types() {
    let engine = Engine::new();
    let src = "
        struct Point { x: int, y: float }
        dylib \"../native/mylib\" {
            Point origin(int);
            int sum(int[], int);
            reset();
        }
        fn total(xs: int[]) -> int {
            mylib::reset();
            return mylib::sum(xs, xs.len()) + mylib::origin(0).x;
        }
    ";
    assert_eq!(engine.check(src, "path.cdl"), Ok(Vec::new()));
}

/// A check still type-checks the calls: an argument of the wrong type is the
/// error it is in a compile, and a signature type C cannot represent is
/// refused rather than bound.
#[test]
fn check_reports_what_a_compile_reports() {
    let engine = Engine::new();
    let wrong_argument = "
        dylib \"nosuch_lib_xyz\" { int f(int); }
        fn g() -> int { return nosuch_lib_xyz::f(\"s\"); }
    ";
    let error = engine.check(wrong_argument, "arg.cdl").unwrap_err();
    assert_eq!(error.code, "argument_type_mismatch", "{}", error.message);

    for signature in ["bool f(int);", "f({string: int});"] {
        let src = format!("dylib \"nosuch_lib_xyz\" {{ {signature} }}\n");
        let error = engine.check(&src, "c.cdl").unwrap_err();
        assert_eq!(error.code, "no_c_representation", "{}", error.message);
    }
}

/// A check wants no `main`, binds no `host` block to a closure, and hands back
/// the warnings the compile raised.
#[test]
fn check_returns_warnings_and_needs_no_host() {
    let engine = Engine::new();
    let src = "
        host \"app\" { int width(string); }
        fn on_click(id) { return app::width(\"x\"); }
    ";
    let warnings = engine
        .check(src, "host.cdl")
        .expect("nothing is registered and it checks");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "unannotated_host_parameter");
}

/// The script the collection tests drive: `grow` leaves a list of rows behind
/// in its registers, where the next call still finds it, and `frame` builds,
/// links, reorders and reads rows of its own. `tests/vm_embedding.rs` runs the
/// same script through an artifact. Rows are built by `row` rather than by a
/// literal in the loop, which keeps clear of lumen-fx/candela#184.
const COLLECTING: &str = r#"
struct Row {
    label: string,
    attrs: {string: string},
    cells: int[],
}

fn row(label: string, key: string, cell: int) -> Row {
    return Row { label: label, attrs: {"key": key}, cells: [cell] };
}

fn grow(n: int) -> int {
    let rows = [];
    for i in 0..n {
        rows.push(row("a row label long enough to pool " + str(i), "an attribute value " + str(i), i));
    }
    return n;
}

fn frame(names: string[], extra: {string: int}, n: int) -> int {
    let rows = [];
    for i in 0..n {
        let name = names[i % names.len()];
        let r = row(name + " row label long enough to pool " + str(i), "an attribute value " + str(i), i);
        r.cells.push(extra.get("bonus"));
        rows.push(r);
    }
    for i in 1..n {
        rows[i].cells.push(rows[i - 1].cells[0]);
        rows[i - 1].attrs.insert("next", rows[i].label);
        rows[i - 1].attrs.insert("key", "a replaced attribute value " + str(i));
    }
    rows.reverse();
    let dropped = rows[0];
    rows.remove(0);
    let sum = dropped.label.len();
    for r in rows {
        sum += r.label.len() + r.attrs.get("key").len() + r.cells.len() + r.cells[0];
        if r.attrs.contains("next") {
            sum += r.attrs.get("next").len();
        }
    }
    return sum;
}

fn main() {}
"#;

/// Compiles [`COLLECTING`].
fn collecting() -> candela::Program {
    Engine::new()
        .compile(COLLECTING, "collecting.cdl")
        .expect("compiles")
}

/// The arguments `frame` is called with: a list and a map, which cross the
/// boundary as fresh heap objects.
fn frame_args(n: i64) -> [Value; 3] {
    [
        Value::Array(vec![
            Value::String(String::from("first host name")),
            Value::String(String::from("second host name")),
        ]),
        Value::Map(BTreeMap::from([(String::from("bonus"), Value::Int(7))])),
        Value::Int(n),
    ]
}

/// Grows the retained heap a step at a time until an idle collection with
/// `budget` does some work, and answers what that call returned.
fn grow_until_collecting(program: &mut candela::Program, budget: u32) -> bool {
    for step in 1..200 {
        program.call("grow", &[Value::Int(step * 300)]).unwrap();
        let before = program.gc_stats();
        let done = program.collect(budget);
        if program.gc_stats().units > before.units {
            return done;
        }
        assert!(done, "a collection that did no work has work left");
    }
    panic!("the heap grew without an idle collection ever starting");
}

/// An unlimited budget runs a whole collection in one call, and a second
/// call right after has nothing to do.
#[test]
fn collect_with_no_limit_finishes_in_one_call() {
    let mut program = collecting();
    let cycles = program.gc_stats().cycles;
    assert!(grow_until_collecting(&mut program, u32::MAX));
    let after = program.gc_stats();
    assert!(after.cycles > cycles);
    assert!(program.collect(u32::MAX));
    assert_eq!(program.gc_stats(), after);
}

/// A cycle left half done by an idle collection carries on through calls
/// whose list and map arguments are marshalled into the heap mid-cycle, and
/// through the trampolines those calls compile, and nothing any of them reads
/// is freed under it.
#[test]
fn calls_between_collection_slices_stay_correct() {
    let mut control = collecting();
    let small = control.call("frame", &frame_args(8)).unwrap();
    let large = control.call("frame", &frame_args(300)).unwrap();

    let mut program = collecting();
    assert!(
        !grow_until_collecting(&mut program, 1),
        "one unit finished a whole cycle"
    );
    let cycle = program.gc_stats().cycles;
    let mut calls = 0;
    loop {
        assert_eq!(program.call("frame", &frame_args(8)).unwrap(), small);
        calls += 1;
        if program.collect(8) || program.gc_stats().cycles > cycle {
            break;
        }
    }
    assert!(calls > 1, "the cycle ended before a second call");
    assert_eq!(program.call("frame", &frame_args(300)).unwrap(), large);
    assert!(program.collect(u32::MAX));
    assert_eq!(program.call("frame", &frame_args(300)).unwrap(), large);
}

/// A function that builds structs in a loop answers every call a host makes,
/// not only the first: the struct literal's template outlives the call.
#[test]
fn a_struct_building_function_answers_every_call() {
    let src = "
struct R { c: int }
fn g(n: int) -> int {
    let rows = [];
    for i in 0..n { rows.push(R { c: i }); }
    return rows.len();
}
fn main() {}
";
    let mut program = Engine::new().compile(src, "rows.cdl").expect("compiles");
    for n in [3i64, 5, 2, 7] {
        assert_eq!(program.call("g", &[n.into()]).unwrap(), Value::Int(n));
    }
}

/// Functions a host calls every frame with a fresh list or map argument.
const PER_FRAME: &str = "
fn total(xs: int[][]) -> int {
    let s = 0;
    for row in xs { for x in row { s = s + x; } }
    return s;
}
fn weigh(m: {string: int[]}) -> int {
    return m.get(\"first entry\")[0] + m.get(\"second entry\")[1];
}
fn double(n: int) -> int { return n * 2; }
fn main() {}
";

/// A nested list argument for frame `i`; its elements add up to `3 * i + 3`.
fn rows(i: i64) -> Value {
    Value::Array(vec![
        Value::Array(vec![Value::Int(i), Value::Int(1)]),
        Value::Array(vec![Value::Int(2 * i), Value::Int(2)]),
    ])
}

/// A map argument for frame `i` with long keys, which the script looks up by
/// the same text; `weigh` answers `2 * i + 1`.
fn entries(i: i64) -> Value {
    record(&[
        (
            "first entry",
            Value::Array(vec![Value::Int(i), Value::Int(0)]),
        ),
        (
            "second entry",
            Value::Array(vec![Value::Int(0), Value::Int(i + 1)]),
        ),
    ])
}

/// The most slots a pool may hold after thousands of calls that each leave
/// their arguments behind as garbage: the first collection's threshold plus
/// what a cycle lets the program allocate while it runs.
const POOL_BOUND: usize = 512;

/// A list or map argument takes a freed slot the way an allocation the script
/// makes does, so a host that passes one every frame keeps the pools bounded
/// instead of adding a slot per frame.
#[test]
fn per_frame_arguments_reuse_freed_slots() {
    let mut program = Engine::new()
        .compile(PER_FRAME, "frames.cdl")
        .expect("compiles");
    for i in 0..5000 {
        assert_eq!(
            program.call("total", &[rows(i)]).unwrap(),
            Value::Int(3 * i + 3)
        );
        assert_eq!(
            program.call("weigh", &[entries(i)]).unwrap(),
            Value::Int(2 * i + 1)
        );
    }
    let stats = program.gc_stats();
    assert!(stats.cycles > 0, "the garbage never started a collection");
    assert!(stats.arrays.len < POOL_BOUND, "{stats:?}");
    assert!(stats.maps.len < POOL_BOUND, "{stats:?}");
}

/// The same with the host collecting a unit at a time between frames, so most
/// arguments are built while a cycle is marking or sweeping, and nothing an
/// argument holds is freed while it is built.
#[test]
fn per_frame_arguments_stay_whole_mid_cycle() {
    let mut program = Engine::new()
        .compile(PER_FRAME, "frames.cdl")
        .expect("compiles");
    for i in 0..5000 {
        program.collect(1);
        assert_eq!(
            program.call("total", &[rows(i)]).unwrap(),
            Value::Int(3 * i + 3)
        );
        program.collect(1);
        assert_eq!(
            program.call("weigh", &[entries(i)]).unwrap(),
            Value::Int(2 * i + 1)
        );
    }
    let stats = program.gc_stats();
    assert!(stats.cycles > 0, "the garbage never started a collection");
    assert!(stats.arrays.len < POOL_BOUND, "{stats:?}");
    assert!(stats.maps.len < POOL_BOUND, "{stats:?}");
}

/// A call compiles once per argument types and runs again after that, so a
/// host can call every frame for as long as it runs: compiling one per call
/// ran the instruction stream past what a jump can address after some twenty
/// thousand calls.
#[test]
fn a_function_called_every_frame_keeps_answering() {
    let mut program = Engine::new()
        .compile(PER_FRAME, "frames.cdl")
        .expect("compiles");
    for i in 0..40_000 {
        assert_eq!(
            program.call("double", &[Value::Int(i)]).unwrap(),
            Value::Int(2 * i)
        );
    }
}

/// Each set of argument types gets its own compiled call, so a bare parameter
/// still takes its type from every call, an empty list included.
#[test]
fn a_call_with_new_argument_types_compiles_its_own_entry() {
    let src = "fn show(x) { return str(x); }\nfn main() {}";
    let mut program = Engine::new().compile(src, "show.cdl").expect("compiles");
    let calls = [
        (Value::Int(1), "1"),
        (
            Value::String(String::from("a longer text")),
            "a longer text",
        ),
        (Value::Int(2), "2"),
        (Value::Array(vec![]), "[]"),
        (Value::Array(vec![Value::Int(3)]), "[3]"),
        (
            Value::Array(vec![Value::String(String::from("x"))]),
            "[\"x\"]",
        ),
        (Value::Int(4), "4"),
    ];
    for (arg, shown) in calls {
        assert_eq!(
            program.call("show", &[arg]).unwrap(),
            Value::String(String::from(shown))
        );
    }
}

/// A function that parses a small document on every call and drops it.
const PARSE_PER_FRAME: &str = "
fn p() -> int {
    let v = as_list(json_parse(\"[[1, 2], {\\\"a\\\": 3}]\"));
    return as_int(as_list(v[0])[1]) + as_int(as_map(v[1]).get(\"a\"));
}
fn main() {}
";

/// A parsed list or map takes a freed slot the way one the script builds
/// does, so a program that parses every frame keeps the pools bounded, with
/// the host's idle collection running between frames or not.
#[test]
fn json_parse_every_frame_reuses_freed_slots() {
    for idle in [0, 1] {
        let mut program = Engine::new()
            .compile(PARSE_PER_FRAME, "j.cdl")
            .expect("compiles");
        for _ in 0..3000 {
            if idle > 0 {
                program.collect(idle);
            }
            assert_eq!(program.call("p", &[]).unwrap(), Value::Int(5));
        }
        let stats = program.gc_stats();
        assert!(stats.cycles > 0, "the parses never started a collection");
        assert!(stats.arrays.len < POOL_BOUND, "{stats:?}");
        assert!(stats.maps.len < POOL_BOUND, "{stats:?}");
    }
}
