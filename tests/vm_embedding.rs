//! Integration tests for the VM-only embedding path.
//!
//! `tests/embedding.rs` covers the same ground with the compiler resident
//! (`Engine`/`Program`). Here the compiler is gone by the time anything runs:
//! `candela build` produces a `.cdlb`, a `HostRegistry` supplies the closures
//! its `host` blocks bind to, and `RuntimeProgram::call` invokes exported
//! functions through the trampolines the build recorded.

use candela::CallError;
use candela::HostBindError;
use candela::HostError;
use candela::HostRegistry;
use candela::HostType;
use candela::LoadError;
use candela::RuntimeProgram;
use candela::Value;
use candela::build_bytecode;
use candela::load_program;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Builds `src` to an artifact and loads it against `hosts`.
fn load(src: &str, filename: &str, hosts: &HostRegistry) -> RuntimeProgram {
    let bytes = build_bytecode(src.to_owned(), filename).expect("source must build to an artifact");
    load_program(&bytes, hosts).expect("artifact must load against the registry")
}

const HOST_PROGRAM: &str = r#"
host "app" {
    int width(string);
    log(string);
}

fn banner(label: string) -> int {
    app::log(label);
    return app::width(label) + 2;
}

fn shout(label: string) -> string {
    return label.uppercase();
}

fn main() {}
"#;

/// The whole point of the export table: an artifact full of host calls loads
/// without a compiler, and a host drives it by name.
#[test]
fn host_calls_bind_at_load_and_exports_run_them() {
    let logged: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&logged);

    let mut hosts = HostRegistry::new();
    hosts.register_host_fn("app", "width", |name: &str| name.len() as i64);
    hosts.register_host_fn("app", "log", move |line: String| {
        sink.borrow_mut().push(line);
    });

    let mut program = load(HOST_PROGRAM, "banner.cdl", &hosts);
    program.run();

    assert_eq!(
        program.call("banner", &["title".into()]).unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        program.call("shout", &["title".into()]).unwrap(),
        Value::String(String::from("TITLE"))
    );
    assert_eq!(*logged.borrow(), vec![String::from("title")]);
}

/// One trampoline serves every call to its function, run after run, against
/// the resident register and heap state.
#[test]
fn a_trampoline_is_reusable() {
    let src = "
        fn double(n: int) -> int { return n * 2; }
        fn main() {}
    ";
    let mut program = load(src, "double.cdl", &HostRegistry::new());
    program.run();

    for n in 0..8i64 {
        assert_eq!(
            program.call("double", &[Value::Int(n)]).unwrap(),
            Value::Int(n * 2)
        );
    }
}

/// Arrays and maps cross the boundary in both directions, allocated into the
/// resident heap pools on the way in.
#[test]
fn collections_cross_the_boundary() {
    let src = "
        fn total(xs: int[]) -> int {
            let sum = 0;
            for x in xs {
                sum = sum + x;
            }
            return sum;
        }

        fn pick(m: {string: int}) -> int {
            return m.get(\"b\");
        }

        fn main() {}
    ";
    let mut program = load(src, "collections.cdl", &HostRegistry::new());
    program.run();

    let list = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
    assert_eq!(program.call("total", &[list]).unwrap(), Value::Int(6));

    let map = Value::Map(BTreeMap::from([
        (String::from("a"), Value::Int(1)),
        (String::from("b"), Value::Int(2)),
    ]));
    assert_eq!(program.call("pick", &[map]).unwrap(), Value::Int(2));
}

/// A variadic `host` declaration binds to a variadic closure through the
/// artifact exactly as it does through `Engine`.
#[test]
fn variadic_host_fn_binds_through_an_artifact() {
    let src = "
        host \"app\" {
            log(...);
        }

        fn report(label: string, count: int) {
            app::log(label, count, true);
        }

        fn main() {}
    ";
    let seen: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);

    let mut hosts = HostRegistry::new();
    hosts.register_host_fn_variadic("app", "log", move |args: &[Value]| {
        sink.borrow_mut().extend_from_slice(args);
        Ok(Value::Null)
    });

    let mut program = load(src, "variadic.cdl", &hosts);
    program.run();

    assert_eq!(
        program
            .call("report", &["rows".into(), Value::Int(3)])
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        *seen.borrow(),
        vec![
            Value::String(String::from("rows")),
            Value::Int(3),
            Value::Bool(true)
        ]
    );
}

/// A registry covering only part of the `host` surface fails the load, naming
/// every function it cannot supply.
#[test]
fn a_partly_covered_registry_names_what_is_missing() {
    let mut hosts = HostRegistry::new();
    hosts.register_host_fn("app", "width", |name: &str| name.len() as i64);

    let bytes = build_bytecode(HOST_PROGRAM.to_owned(), "banner.cdl").expect("builds");
    match load_program(&bytes, &hosts) {
        Err(LoadError::HostBinding(HostBindError::Unregistered(names))) => {
            assert_eq!(names, ["app::log"]);
        }
        Err(e) => panic!("expected an unregistered host fn, got: {e}"),
        Ok(_) => panic!("load must not succeed while a host fn is unbound"),
    }
}

/// The signature check at load is the one the compiler applies: a closure of
/// the wrong shape is refused before anything runs.
#[test]
fn a_closure_of_the_wrong_shape_is_refused_at_load() {
    let src = "host \"app\" { int width(string); }\n\nfn main() {}\n";
    let bytes = build_bytecode(src.to_owned(), "shape.cdl").expect("builds");

    let mut wrong_return = HostRegistry::new();
    wrong_return.register_host_fn("app", "width", |name: &str| name.to_owned());
    match load_program(&bytes, &wrong_return).err() {
        Some(LoadError::HostBinding(HostBindError::SignatureMismatch(message))) => {
            assert!(message.contains("app::width"), "{message}");
            assert!(message.contains("string"), "{message}");
        }
        Some(e) => panic!("expected a signature mismatch, got: {e}"),
        None => panic!("a closure of the wrong shape must not bind"),
    }

    let mut wrong_arity = HostRegistry::new();
    wrong_arity.register_host_fn("app", "width", || 0i64);
    assert!(matches!(
        load_program(&bytes, &wrong_arity),
        Err(LoadError::HostBinding(HostBindError::SignatureMismatch(_)))
    ));
}

/// Only functions the build could give a trampoline are callable; anything else
/// is reported as unknown rather than mis-dispatched.
#[test]
fn calling_a_name_the_artifact_does_not_export() {
    let src = "
        fn known(x: int) -> int { return x; }
        fn bare(x) { return x + 1; }
        fn main() {}
    ";
    let mut program = load(src, "exports.cdl", &HostRegistry::new());

    let names: Vec<&str> = program.exports().collect();
    assert_eq!(
        names,
        ["known"],
        "a bare parameter has no signature to check"
    );

    match program.call("bare", &[Value::Int(1)]) {
        Err(CallError::UnknownFunction(name)) => assert_eq!(name, "bare"),
        other => panic!("expected an unknown function, got: {other:?}"),
    }
    assert!(matches!(
        program.call("missing", &[]),
        Err(CallError::UnknownFunction(_))
    ));
    assert!(
        !program.exports().any(|name| name == "main"),
        "`main` is run, not called"
    );
}

/// Arguments are checked against the declared parameter types before the
/// trampoline runs.
#[test]
fn arguments_are_checked_against_the_declaration() {
    let src = "fn greet(name: string) -> string { return name; }\nfn main() {}\n";
    let mut program = load(src, "greet.cdl", &HostRegistry::new());

    match program.call("greet", &[Value::Int(1)]) {
        Err(CallError::ArgType {
            function,
            index,
            expected,
            found,
        }) => {
            assert_eq!((function.as_str(), index), ("greet", 1));
            assert_eq!((expected.as_str(), found.as_str()), ("string", "int"));
        }
        other => panic!("expected an argument type error, got: {other:?}"),
    }

    match program.call("greet", &[]) {
        Err(CallError::ArgCount {
            expected, found, ..
        }) => assert_eq!((expected, found), (1, 0)),
        other => panic!("expected an argument count error, got: {other:?}"),
    }
}

/// A runtime error inside a called function comes back as a value; the host
/// stays alive and can call again.
#[test]
fn a_runtime_error_comes_back_as_a_diagnostic() {
    let src = "
        fn at(xs: int[], i: int) -> int { return xs[i]; }
        fn main() {}
    ";
    let mut program = load(src, "bounds.cdl", &HostRegistry::new());
    program.run();

    let list = Value::Array(vec![Value::Int(1)]);
    match program.call("at", &[list.clone(), Value::Int(9)]) {
        Err(CallError::Runtime(diagnostic)) => {
            assert_eq!(diagnostic.filename, "bounds.cdl");
            assert!(!diagnostic.code.is_empty());
        }
        other => panic!("expected a runtime error, got: {other:?}"),
    }

    assert_eq!(
        program.call("at", &[list, Value::Int(0)]).unwrap(),
        Value::Int(1)
    );
}

/// A signature handed over as data is checked at load like a derived one, and
/// binds a closure the artifact then calls.
#[test]
fn a_signature_given_as_data_binds_at_load() {
    let src = "
        host \"gpio\" {
            int read(int);
        }

        fn level(pin: int) -> int { return gpio::read(pin) + 1; }

        fn main() {}
    ";
    let bytes = build_bytecode(src.to_owned(), "gpio.cdl").expect("builds");

    let mut wrong = HostRegistry::new();
    wrong.register_host_fn_typed(
        "gpio",
        "read",
        vec![HostType::String],
        HostType::Int,
        |_args: &[Value]| Ok(Value::Int(0)),
    );
    assert!(matches!(
        load_program(&bytes, &wrong),
        Err(LoadError::HostBinding(HostBindError::SignatureMismatch(_)))
    ));

    let mut hosts = HostRegistry::new();
    hosts.register_host_fn_typed(
        "gpio",
        "read",
        vec![HostType::Int],
        HostType::Int,
        |args: &[Value]| Ok(Value::Int(args[0].as_i64().unwrap_or(0) * 2)),
    );
    let mut program = load_program(&bytes, &hosts).expect("binds");
    program.run();

    assert_eq!(
        program.call("level", &[Value::Int(20)]).unwrap(),
        Value::Int(41)
    );
}

/// A host closure that returns an error raises in the artifact the same way it
/// does with the compiler resident: the call comes back as a runtime error
/// naming the function, and the program keeps working.
#[test]
fn a_host_fn_error_comes_back_from_an_artifact_call() {
    let src = "
        host \"gpio\" {
            int read(int);
        }

        fn level(pin: int) -> int { return gpio::read(pin); }

        fn main() {}
    ";
    let mut hosts = HostRegistry::new();
    hosts.register_host_fn("gpio", "read", |pin: i64| {
        if pin == 21 {
            Ok(1i64)
        } else {
            Err(HostError::new("no such pin"))
        }
    });

    let mut program = load(src, "gpio.cdl", &hosts);
    program.run();

    match program.call("level", &[Value::Int(7)]) {
        Err(CallError::Runtime(diagnostic)) => {
            assert_eq!(diagnostic.code, "host_fn_error");
            assert!(diagnostic.message.contains("gpio::read"), "{diagnostic:?}");
            assert!(diagnostic.message.contains("no such pin"), "{diagnostic:?}");
        }
        other => panic!("expected a host function error, got: {other:?}"),
    }

    assert_eq!(
        program.call("level", &[Value::Int(21)]).unwrap(),
        Value::Int(1)
    );
}

/// `any` is a real annotation, so a function that takes whatever it is given is
/// still exported and still checked: every value satisfies it.
#[test]
fn an_any_parameter_accepts_every_value() {
    let src = "
        fn echo(x: any) -> any { return x; }
        fn main() {}
    ";
    let mut program = load(src, "any.cdl", &HostRegistry::new());
    program.run();

    assert_eq!(
        program.call("echo", &[Value::Int(7)]).unwrap(),
        Value::Int(7)
    );
    assert_eq!(
        program.call("echo", &["seven".into()]).unwrap(),
        Value::String(String::from("seven"))
    );
}

// ---------------------------------------------------------------------------
// OUT-OF-RANGE INDICES
// ---------------------------------------------------------------------------
//
// `a_runtime_error_comes_back_as_a_diagnostic` above covers the base case
// through this loaded-artifact path: an out-of-range read comes back as a
// `Diagnostic` and the artifact keeps working for the next call. The tests
// below pin the same channel across a negative index, a write target, a
// string, a slice, and the two-call lookup-miss shape a host that reruns a
// reactive derivation on each tick actually drives `RuntimeProgram::call`
// with.

/// A negative index through a loaded artifact is out of range the same way
/// one past the end is.
#[test]
fn a_negative_index_through_an_artifact_is_a_diagnostic() {
    let src = "
        fn last_of(xs: int[]) -> int { return xs[-1]; }
        fn main() {}
    ";
    let mut program = load(src, "neg.cdl", &HostRegistry::new());
    program.run();

    match program.call("last_of", &[Value::Array(vec![Value::Int(1)])]) {
        Err(CallError::Runtime(diagnostic)) => {
            assert_eq!(diagnostic.code, "index_out_of_bounds");
        }
        other => panic!("expected a runtime error, got: {other:?}"),
    }
}

/// Assigning through an out-of-range index is the same bounds check as
/// reading through one, and a string indexes the same way a list does.
#[test]
fn write_and_string_index_out_of_range_through_an_artifact_are_diagnostics() {
    let src = r"
        fn set_at(xs: int[], i: int, v: int) -> int[] { xs[i] = v; return xs; }
        fn char_at(s: string, i: int) -> string { return s[i]; }
        fn set_char(s: string, i: int, c: string) -> string { s[i] = c; return s; }
        fn main() {}
    ";
    let mut program = load(src, "write.cdl", &HostRegistry::new());
    program.run();

    match program.call(
        "set_at",
        &[
            Value::Array(vec![Value::Int(1)]),
            Value::Int(1),
            Value::Int(9),
        ],
    ) {
        Err(CallError::Runtime(diagnostic)) => assert_eq!(diagnostic.code, "index_out_of_bounds"),
        other => panic!("expected a runtime error, got: {other:?}"),
    }

    match program.call("char_at", &[Value::from("a"), Value::Int(1)]) {
        Err(CallError::Runtime(diagnostic)) => assert_eq!(diagnostic.code, "index_out_of_bounds"),
        other => panic!("expected a runtime error, got: {other:?}"),
    }

    match program.call(
        "set_char",
        &[Value::from("a"), Value::Int(1), Value::from("z")],
    ) {
        Err(CallError::Runtime(diagnostic)) => assert_eq!(diagnostic.code, "index_out_of_bounds"),
        other => panic!("expected a runtime error, got: {other:?}"),
    }
}

/// A slice whose bounds fall outside the collection is its own error kind,
/// distinct from a single out-of-range index, but the same non-panicking
/// channel.
#[test]
fn a_slice_out_of_range_through_an_artifact_is_a_diagnostic() {
    let src = "
        fn tail(xs: int[], a: int, b: int) -> int[] { return xs[a..b]; }
        fn main() {}
    ";
    let mut program = load(src, "slice.cdl", &HostRegistry::new());
    program.run();

    match program.call(
        "tail",
        &[
            Value::Array(vec![Value::Int(1)]),
            Value::Int(0),
            Value::Int(5),
        ],
    ) {
        Err(CallError::Runtime(diagnostic)) => assert_eq!(diagnostic.code, "slice_out_of_bounds"),
        other => panic!("expected a runtime error, got: {other:?}"),
    }
}

/// Mirrors how a host reruns a reactive derivation: a lookup by id returns -1
/// on a miss, and that result indexes the list directly. The first call is an
/// ordinary hit; the second, on the same resident artifact, is a miss that
/// feeds a negative index into the list. Both come back as values through
/// `call`, never a panic, and the artifact is still usable afterward.
#[test]
fn a_lookup_miss_feeding_a_negative_index_through_an_artifact_is_a_diagnostic() {
    let src = r"
        fn find_idx(list: int[], id: int) -> int {
            let i = 0;
            while i < list.len() {
                if list[i] == id { return i; }
                i += 1;
            }
            return -1;
        }
        fn current(list: int[], id: int) -> int {
            let idx = find_idx(list, id);
            return list[idx];
        }
        fn main() {}
    ";
    let mut program = load(src, "derive.cdl", &HostRegistry::new());
    program.run();

    let list = Value::Array(vec![Value::Int(1), Value::Int(2)]);

    assert_eq!(
        program
            .call("current", &[list.clone(), Value::Int(1)])
            .unwrap(),
        Value::Int(1)
    );

    match program.call("current", &[list, Value::Int(99)]) {
        Err(CallError::Runtime(diagnostic)) => {
            assert_eq!(diagnostic.code, "index_out_of_bounds");
        }
        other => panic!("expected a runtime error, got: {other:?}"),
    }
}
