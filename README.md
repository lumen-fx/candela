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
candela program.cdl   # Run a file
candela               # Start the REPL
candela -v/--version  # Print version
candela -h/--help     # Print help
```

Candela source files use the `.cdl` extension.

## Editor support

The [VS Code extension](editors/vscode/) provides syntax highlighting plus a
language server (live diagnostics, hover, completion, outline,
go-to-definition) via [`candela-lsp`](candela-lsp/), a separate crate in this
workspace built on candela's own lexer/parser/type-checker.
