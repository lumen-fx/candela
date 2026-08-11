# Embedding candela in Rust

candela runs inside a Rust program as a library. You register Rust functions the
script can call, compile a script once, and then call its functions repeatedly
while the interpreter keeps its state between calls.

Add the crate as a dependency and enable the `embed` feature:

```toml
[dependencies]
candela = { version = "0.0.3", features = ["embed"] }
```

The feature changes what a fatal error does: instead of ending the process, it
unwinds so the host survives and receives the error as a value. It needs a
profile that unwinds, which the crate provides as `embed`:

```sh
cargo build --profile embed --features embed
```

## A whole program

```rust
use candela::{Engine, Value};

fn main() -> Result<(), candela::Diagnostic> {
    let mut engine = Engine::new();
    engine.register_host_fn("app", "width", |name: &str| name.len() as i64);

    let mut program = engine.compile(
        r#"
        host "app" {
            int width(string);
        }

        fn banner(label: string) -> int {
            return app::width(label) + 2;
        }

        fn main() {}
        "#,
        "banner.cdl",
    )?;

    let cells = program.call("banner", &["title".into()])?;
    assert_eq!(cells, Value::Int(7));
    Ok(())
}
```

## Engine

`Engine` holds the table of registered host functions and compiles scripts.

### register_host_fn

```rust
engine.register_host_fn(namespace, name, closure);
```

Binds a Rust closure to the name a script reaches through a `host` block. The
closure may take up to five arguments of any of these types, and return one of
them or `()`:

| Rust | candela |
| --- | --- |
| `i64`, `i32` | `int` |
| `f64` | `float` |
| `bool` | `bool` |
| `String`, or a single `&str` argument | `string` |
| `Vec<T>` | `T[]` |
| `BTreeMap<String, T>`, `HashMap<String, T>` | `{string: T}` |
| `()` (return only) | `null` |

The closure's signature is derived from its Rust types and checked against the
script's `host` declaration when you compile. A disagreement in arity, argument
type or return type is returned as a `Diagnostic`, never a panic.

### register_host_fn_variadic

```rust
engine.register_host_fn_variadic("app", "log", |args: &[Value]| {
    for arg in args {
        println!("{arg:?}");
    }
    Value::Null
});
```

Binds a closure that receives every argument as a slice and returns one `Value`,
so mixed and dynamically shaped arguments cross the boundary without a fixed
Rust signature. The script must declare the function with `...`:

```rust
host "app" {
    log(...);
}
```

No arity or type checking happens at the call site; the closure interprets what
it is handed. A variadic declaration bound to a fixed closure, or the reverse,
is a `Diagnostic` at compile time.

### compile

```rust
let mut program = engine.compile(source, filename)?;
```

Parses and type-checks the source, binds every `host` function it declares to a
registered closure, and runs `main` once so top-level setup is done before the
host makes its first call. The filename is what error reports name.

Returns a `Diagnostic` when the script does not compile, when a declared `host`
function has no registered closure, when a registered closure disagrees with its
declaration, or when running `main` raises a runtime error.

## Program

`Program` is a compiled script with live interpreter state. Registers and heap
stay resident, so anything one call establishes is visible to the next. It is
single-threaded and neither `Send` nor `Sync`, matching the VM.

### call

```rust
let value = program.call("banner", &["title".into()])?;
```

Invokes a script function by name and returns its value, or `Value::Null` for a
function that returns nothing. Arguments are `Value`s; `.into()` covers the
scalars.

The call is type-checked against the function's signature, so a wrong argument
type comes back as a `Diagnostic` rather than corrupting the run. Annotate the
parameters of a function a host calls. A function called only from the script
takes its parameter types from the call site, but a host-called function has no
such call site, and the annotation is what the arguments are checked against:

```rust
fn banner(label: string) -> int {
    return app::width(label) + 2;
}
```

Leaving the parameters bare still works, and the types are then taken from the
first host call. Annotate when you want a mismatch reported against the
declaration rather than accepted as a new specialisation.

Returns a `Diagnostic` when the function is unknown, when the arguments do not
type-check, or when the call raises a runtime error.

## Values

`Value` is the type that crosses the boundary in both directions:

```rust
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Map(BTreeMap<String, Value>),
}
```

`From` is implemented for `i64`, `i32`, `f64`, `bool`, `String`, `&str` and
`()`, so `5i64.into()` and `"title".into()` build one. Going the other way,
`as_i64`, `as_f64`, `as_str`, `into_string`, `into_array` and `into_map` unwrap
one when it holds what you ask for.

candela integers are 32-bit; `Value::Int` is an `i64` for convenience on the
host side and narrows on the way in. Arrays are homogeneous and maps are
string-keyed, matching how candela types them. A struct read back from a script
arrives as a `Map` of its fields.

## Errors

Every fallible call returns `Diagnostic`:

```rust
pub struct Diagnostic {
    pub filename: String,
    pub span: Range<usize>,
    pub message: String,
    pub code: String,
}
```

`code` is a stable snake_case identifier you can match on, such as
`unknown_variable`, `argument_type_mismatch`, `index_out_of_bounds`,
`unregistered_host_fn` or `host_fn_signature_mismatch`. `message` is plain text
with the terminal colouring removed, and `span` is a byte range into the file
`filename` names, which is enough to underline the offending source yourself.
See [errors](../reference/errors.md).

Only one diagnostic comes back per call, because compilation stops at the first
error.

## Evaluating source at run time

Compiling is opt-in and explicit: a host that wants to evaluate new source calls
`Engine::compile` again. There is no `eval` inside the language, so a script
cannot compile new code by itself, and a host that never calls `compile` after
start-up cannot be made to.

This is also the difference between the two ways of shipping candela in a host.
Linking the `candela` crate brings the compiler, so scripts can be compiled at
run time and reloaded. Shipping precompiled `.cdlb` artifacts and linking only
`candela-vm` leaves the compiler out of the process entirely; see
[artifacts](../reference/artifacts.md). An artifact that declares a `host` block
needs a host to bind it, so the standalone runtime refuses to load one.

## One-shot execution

For running a script and collecting what it printed, without keeping any state,
the crate exports a C entry point: `candela_run` takes source and returns the
captured output, and `candela_free_output` releases it. Use `Engine` and
`Program` for anything that calls back and forth.
