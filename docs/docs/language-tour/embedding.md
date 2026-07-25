---
icon: lucide/cpu
---
# Embedding Candela

!!! warning

    This API is subject to change, particularly once instruction and memory
    limits are implemented.

Candela is built to embed in other programs. There are two ways to do it: a
one-shot C ABI for running a script and collecting its output, and a Rust
`Engine`/`Program` API for registering host functions and calling script
functions.

## One-shot C ABI

Download the `libcandela-*` artifact from
[the latest release](https://github.com/lumen-fx/candela/releases/latest), or
build it from source as a dynamic library:

```sh
cargo build --profile embed --features embed
```

Two functions are exposed:

```c
// Compile and run the code, returning its captured output.
extern char* candela_run(const char* code);
// Free a string returned by candela_run.
extern void candela_free_output(char* output);
```

`candela_run` compiles the source, runs `main`, and returns whatever the script
printed. Errors come back in the output string rather than crashing the host.

## Host functions and the `Engine`/`Program` API

A Rust host embeds Candela through the `Engine`/`Program` API (see
`src/engine.rs`). Unlike the one-shot C ABI, it keeps compiler and VM state
resident, so you register typed host functions, compile a script once, and call
its functions repeatedly with state persisting between calls.

Declare the functions your host provides in a `host` block, then call them
through the namespace you gave the block:

```rust
host "app" {
    int rows(string);
}

fn count(id) {
    return app::rows(id);
}

fn main() {}
```

On the Rust side, register a closure for each declared function, compile the
script, and call into it by name:

```rust
let mut engine = candela::Engine::new();
engine.register_host_fn("app", "rows", |id: &str| id.len() as i64);

let mut program = engine.compile(src, "main.cdl")?;
let rows = program.call("count", &["board".into()])?;
assert_eq!(rows, candela::Value::Int(5));
```

`compile` binds each `host` function to its registered closure and checks the
declared signature against the closure. A missing registration, a signature
mismatch, or a parse or type error is returned as a structured `Diagnostic`
rather than aborting the process. Values crossing the boundary (`int`, `float`,
`bool`, `string`, arrays, and string-keyed maps) marshal through the `Value`
type.

A closure that takes every argument as a slice can be registered with
`register_host_fn_variadic`, matched by a `...` argument list in the `host`
block:

```rust
host "app" {
    log(...);
}
```

Because host functions are supplied by the embedding runtime, a program that
uses a `host` block cannot run through the standalone `candela-vm`; run it
through the `Engine`/`Program` API, which provides the bindings.
