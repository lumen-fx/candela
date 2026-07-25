<p align="center">
  <img src="assets/colored-logo.png" alt="Candela" width="220">
</p>

# Candela

> [!WARNING]
> Candela is under active development and the API is unstable.

**Candela** is a fast, statically-typed interpreted language that aims to combine Rust-like syntax with Python's ease-of-use.

Its goal is to provide a faster alternative to Python that sits closer to low-level languages while remaining accessible to a wide audience.

Candela is the embedded scripting language for the Lumen UI framework and is driven through a host-embeddable `Engine`/`Program` API (see `src/embed.rs`).

## Origin and attribution

Candela is a fork of [keel](https://github.com/horacehoff/keel) by Horace Hoff.
The original work is licensed under the Apache License, Version 2.0; that license
and Horace Hoff's authorship are retained in full (see `LICENSE` and `NOTICE`).
Candela renames the language and crate and extends it with a host embedding API
(variadic host functions, array/map marshalling, structured diagnostics) for use
inside Lumen. All credit for the original language design and implementation goes
to Horace Hoff.

Upstream project: https://github.com/horacehoff/keel

## Why Candela?

- **Fast**: aggressive compile-time optimizations ([benchmarks](BENCHMARKS.md))
- **Familiar syntax**: Rust-like, with Python's ease-of-use
- **Statically typed, zero annotations**: full type inference, static type checking, polymorphism
- **FFI support**: call C/dynamic libraries directly from Candela
- **Embeddable**: register typed host functions and drive scripts from a Rust host
- **Built-in REPL**

[Browse examples](examples/)

## Quick showcase

```rust
struct Point { x: int, y: int }

fn add(a, b) {
    return a + b;
}

fn main() {
    let p = Point { x: 3, y: 4 };
    print(add(p.x, p.y)); // 7
    print(add("Hello, ", "world!")); // Hello, world!

    let nums = [4, 2, 6, 1, 7];
    if nums[0] == 4 {
        nums.sort();
        print(if nums[0] == 1 { nums[0..3] } else { -1 }); // [1,2,4]
    } else {
        throw("Error!");
    }
}
```

## Build from source

Make sure [Rust](https://rustup.rs/) is installed.

```sh
cargo build --release
./target/release/candela myfile.cdl
```

## Usage

```sh
candela program.cdl               # Run a file
candela build program.cdl         # Compile to program.cdlb bytecode
candela build program.cdl -o a.cdlb   # ... to a chosen path
candela                           # Start the REPL
candela -v/--version              # Print version
candela -h/--help                 # Print help
```

Candela source files use the `.cdl` extension.

## Ahead-of-time bytecode and the lean `candela-vm`

Candela ships as two binaries:

- **`candela`** -- the full toolchain: parser + compiler + VM + REPL + the
  `Engine`/`Program` embedding API.
- **`candela-vm`** -- a small, standalone runtime that loads and runs
  pre-compiled bytecode and carries no parser, compiler, or REPL. Our goal is to
  keep it under 1 MiB.

This mirrors an AOT model. The full `candela` toolchain (compiler + VM) compiles
a `.cdl` source to a compact, self-contained `.cdlb` bytecode artifact;
`candela-vm` just runs it:

```sh
candela build program.cdl         # emits program.cdlb
candela-vm program.cdlb           # loads + runs the bytecode
```

Running a `.cdlb` through `candela-vm` produces output (and, on a runtime error,
diagnostics -- the source is embedded in the artifact) identical to running the
`.cdl` directly through `candela`.

A `.cdlb` artifact is a 4-byte magic (`CDLB`), a 1-byte format version, and a
`postcard`-encoded image of the program's bytecode, constant pools, struct
table, and sources; a version mismatch is rejected cleanly.

A `.cdlb` captures the **whole program**: every imported workspace `.cdl` module
is linked into the single artifact, so `candela-vm app.cdlb` runs with no source
tree present.

`dylib` imports are captured **by reference, never by value**: the artifact
stores only the logical library name, each imported symbol, and its signature --
never the shared object's bytes. At load, `candela-vm` re-opens the library
through the OS loader and re-binds the symbols by name. A `dylib` referenced by a
bare logical name (e.g. `z`, `sqlite3`) is mapped to the per-OS filename
convention at load time -- `libz.so` on Linux, `libz.dylib` on macOS, `z.dll` on
Windows -- so the same source builds and runs across platforms; an explicit path
or filename is honored as given. (The library itself must be present on the
machine that runs the `.cdlb`, exactly as when running the `.cdl` directly.)

`host` blocks are also captured as recipes (the host function's name and
signature). Because a `host` function is bound by an embedding runtime, a
`host`-using `.cdlb` run through the standalone `candela-vm` fails to load with a
clear error naming the host function it cannot provide; such programs run through
the embedding `Engine`/`Program` API, which supplies the bindings.

`candela-vm` is the standalone runtime: it holds the VM executor, bytecode/data
types, GC, value marshalling, and the `.cdlb` load/run API. The full `candela`
binary runs on that same runtime, so a program behaves identically whether you
run its source through `candela` or its `.cdlb` through `candela-vm`.

## Editor support

The [VS Code extension](editors/vscode/) provides syntax highlighting plus a
language server (live diagnostics, hover, completion, outline,
go-to-definition) via [`candela-lsp`](lsp/), a separate crate in this
workspace built on candela's own lexer/parser/type-checker.
