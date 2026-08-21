# Embedding candela in Rust

candela runs inside a Rust program as a library. You register Rust functions the
script can call, load a script once, and then call its functions repeatedly
while the interpreter keeps its state between calls.

There are two ways to do it, and they differ in one thing: whether the compiler
is in your process.

- Link `candela` and use `Engine`/`Program` to compile source at run time. Scripts
  can be edited and reloaded while the program runs.
- Link `candela-vm` alone and use `HostRegistry`/`RuntimeProgram` to load a
  `.cdlb` artifact built beforehand. The compiler is absent, the binary is
  smaller, and no source is shipped.

Everything else is shared: the same host functions, the same `Value` type, the
same call-by-name. Start with the first if you are unsure; jump to
[running a precompiled artifact](#running-a-precompiled-artifact) for the second.

Add the crate as a dependency and enable the `embed` feature:

```toml
[dependencies]
candela = { package = "candela-lang", version = "0.0.4", features = ["embed"] }
```

The crate publishes as `candela-lang`, because the name `candela` on crates.io
belongs to an unrelated project. The `package` key renames it back, so your code
keeps writing `use candela::...` as everything below does.

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

A namespace is a namespace, so a host function may take a name a built-in
already has: `gpio::read` is the `int` its block declares, not the `read` that
returns a string.

A closure that can fail returns `Result<T, HostError>` in place of `T`:

```rust
use candela::HostError;

engine.register_host_fn("fs", "read", |path: &str| {
    std::fs::read_to_string(path).map_err(HostError::new)
});
```

`HostError::new` takes anything that renders, so an error from the work the
closure was doing carries through with `map_err`. What the script sees is a
runtime error at the call, naming the function and repeating the message; it
can be caught with `catch "host_fn_error"`, and it reaches the host as a
`Diagnostic` with that code when it is not. The type checked against the
declaration is the `T` inside the `Result`, so the two spellings bind to the
same `host` signature.

### register_host_fn_variadic

```rust
engine.register_host_fn_variadic("app", "log", |args: &[Value]| {
    for arg in args {
        println!("{arg:?}");
    }
    Ok(Value::Null)
});
```

Binds a closure that receives every argument as a slice and returns one `Value`,
or a `HostError` to raise in the script, so mixed and dynamically shaped
arguments cross the boundary without a fixed Rust signature. The script must
declare the function with `...`:

```rust
host "app" {
    log(...);
}
```

No arity or type checking happens at the call site; the closure interprets what
it is handed. A variadic declaration bound to a fixed closure, or the reverse,
is a `Diagnostic` at compile time.

### register_host_fn_typed

```rust
use candela::{HostType, Value};

engine.register_host_fn_typed(
    "gpio",
    "read",
    vec![HostType::Int],
    HostType::Int,
    |args: &[Value]| Ok(Value::Int(args[0].as_i64().unwrap_or(0))),
);
```

Binds a closure that takes a slice, like the variadic form, but with the
signature handed over as data instead of read off a Rust closure. The
declaration is checked against those types the same way, so it must not use
`...`, and a script calling `gpio::read` gets an `int`.

Use it when the signature is only known at run time: a plugin table, a
generated binding, anything a fixed Rust closure cannot spell.

```rust
pub enum HostType {
    Int,
    Float,
    Bool,
    String,
    Unit,
    Array(Box<HostType>),
    Map(Box<HostType>),
}
```

These are the same types the table above lists, in the order arguments are
passed. `Unit` is candela's `null`, which is what a function that returns
nothing declares. `Map` is always string-keyed, so only the value type is
carried.

### register_macro

```rust
use candela::macros::MacroError;

engine.register_macro("lmn", |body: &str| {
    Ok::<String, MacroError>(format!("\"{}\"", body.trim()))
});
```

Gives `lmn!( ... )` a meaning in the scripts this engine compiles. The closure
receives the raw text between the parentheses, which candela does not interpret,
and returns candela source for one expression, which is parsed where the macro
stands. This is how a host puts its own syntax into a script; see
[macros](../language/macros.md) for what a script author sees. An expansion may
use a macro itself, up to 32 levels deep, so an expander that emits its own
macro fails the compile instead of running out of stack.

Returning `MacroError` instead fails the compile at the macro:

```rust
pub struct MacroError {
    pub message: String,
    pub offset: Option<usize>,
}
```

`offset` is a byte offset into the region body. Set it (`MacroError::at`) and
the diagnostic points at that position in the file the macro was written in;
leave it out (`MacroError::new`) and it covers the whole invocation. The region
ends at the parenthesis balancing the one that opened it, ignoring parentheses
inside candela string literals and after `//`.

### allow_unknown_macros

```rust
engine.allow_unknown_macros(true);
```

A macro with no registered expander fails the compile by default, naming it.
Turning this on compiles it as `null` instead. It is for tools that read scripts
written for a host they are not part of, and would otherwise report every one of
that host's macros as an error; candela's own language server does this.

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

## Finding macro regions

A build tool often needs to know where a macro is used before anything is
compiled: to collect the markup in a project, to hash it, to generate assets
from it. `scan_regions` runs the scanner the lexer runs, over plain source:

```rust
use candela::macros::scan_regions;

for region in scan_regions(&source, "lmn") {
    println!("{} at {:?}", region.body, region.span);
}
```

```rust
pub struct Region<'a> {
    pub body: &'a str,
    pub body_start: usize,
    pub span: Range<usize>,
}
```

`body` is the raw text between the parentheses, `body_start` its byte offset in
the source, and `span` the byte range of the whole invocation. The results match
what compiling the same source expands, including which `name!(` occurrences do
not count: one written inside a string literal or after `//` is not an
invocation. Scanning stops at a region the source never closes, so every region
returned is complete.

## Running a precompiled artifact

Build the script to a `.cdlb` first, with `candela build` or `build_bytecode`,
then link only `candela-vm` in the program that runs it:

```toml
[dependencies]
candela-vm = "0.0.4"
```

```rust
use candela_vm::{HostRegistry, Value, load_program};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut hosts = HostRegistry::new();
    hosts.register_host_fn("app", "width", |name: &str| name.len() as i64);

    let bytes = std::fs::read("banner.cdlb")?;
    let mut program = load_program(&bytes, &hosts)?;
    program.run();

    let cells = program.call("banner", &["title".into()])?;
    assert_eq!(cells, Value::Int(7));
    Ok(())
}
```

### HostRegistry

The table of closures a script's `host` blocks bind to. `register_host_fn`,
`register_host_fn_typed` and `register_host_fn_variadic` take the same arguments
and derive the same signatures as their `Engine` counterparts above.

Binding happens in `load_program`, and it checks what compiling a script checks:
every declared function must be registered, and each closure's arity, argument
types and return type must match the declaration. A `LoadError::HostBinding`
comes back naming what is missing or what disagrees, before any instruction runs.
`Engine` holds one of these registries internally, which is why the two paths
accept the same closures.

### run

Runs `main`, the same way `candela-vm` does: a runtime error prints its report
and ends the process. Call it once, before the first `call`, so top-level setup
is done. To keep a failing script from taking the process with it, run it inside
`collect_diagnostic`, which turns the error into a `Diagnostic` you can handle.

### call

```rust
let value = program.call("banner", &["title".into()])?;
```

Invokes a function by name against the resident state, returning its value or
`Value::Null` for a function that returns nothing. Arguments are checked against
the declared parameter types first.

Only functions the artifact exports are callable, and `program.exports()` lists
them. A function is exported when it is defined in the file that was built, is
reachable by its bare name, is not `main`, and annotates every parameter with a
type a host value can be. See [artifacts](../reference/artifacts.md) for the full
rule. This is the difference that matters when moving a script from `Engine` to
an artifact: bare parameters take their types from the first host call there, but
an artifact has no compiler to specialise them later, so annotate them.

Errors come back as a `CallError`: the name is not exported, the argument count
or an argument type disagrees with the declaration, or the call raised a runtime
error, which arrives as the `Diagnostic` it produced.

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

A [generic](../language/generics.md) type is instantiated at compile time, so
nothing generic reaches the host: `Cell<int>` crosses the boundary as the
ordinary struct it compiles to.

## Errors

Every fallible `Engine`/`Program` call returns `Diagnostic`:

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
`unregistered_host_fn`, `host_fn_signature_mismatch` or `host_fn_error` (one of
your own closures returned a `HostError`). `message` is plain text
with the terminal colouring removed, and `span` is a byte range into the file
`filename` names, which is enough to underline the offending source yourself.
See [errors](../reference/errors.md).

Only one diagnostic comes back per call, because compilation stops at the first
error.

On the artifact path the errors are `LoadError` and `CallError` instead, both of
which print themselves. A runtime error inside a call still arrives as the
`Diagnostic` above, wrapped in `CallError::Runtime`.

## Evaluating source at run time

Compiling is opt-in and explicit: a host that wants to evaluate new source calls
`Engine::compile` again. There is no `eval` inside the language, so a script
cannot compile new code by itself, and a host that never calls `compile` after
start-up cannot be made to.

This is also the difference between the two ways of shipping candela in a host.
Linking the `candela` crate brings the compiler, so scripts can be compiled at
run time and reloaded. Shipping precompiled `.cdlb` artifacts and linking only
`candela-vm` leaves the compiler out of the process entirely, and with it any
way to evaluate source at all; see [artifacts](../reference/artifacts.md).

## One-shot execution

For running a script and collecting what it printed, without keeping any state,
the crate exports a C entry point: `candela_run` takes source and returns the
captured output, and `candela_free_output` releases it. Use `Engine` and
`Program` for anything that calls back and forth.
