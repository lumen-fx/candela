<!--
This file is derived from keel (https://github.com/horacehoff/keel),
Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
It has been modified by the candela authors. See the NOTICE file.
-->
# candela

[![coverage](https://codecov.io/gh/lumen-fx/candela/branch/main/graph/badge.svg)](https://codecov.io/gh/lumen-fx/candela)

A small statically typed scripting language with Rust-like syntax.

## Why candela

candela is the scripting language of the Lumen UI framework, and it works on
its own as well. Programs are type-checked before they run, so a mistake shows
up as a compile error instead of surfacing halfway through a run, and the
syntax stays close enough to Rust to read without a tour.

Reach for it when you want to script an application without embedding a large
runtime, or when you want a small language for programs that start quickly and
behave the same every time.

candela began as a fork of [keel](https://github.com/horacehoff/keel), a
language written by Horace Hoff, and its compiler, virtual machine and type
system are built on that work. What changed since the fork is recorded in
[NOTICE](NOTICE) and in the repository's history, which carries keel's commits.

## Quick start

Install on Linux or macOS:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh
```

On Windows, run the per-user installer from
<https://github.com/lumen-fx/candela/releases/latest/download/candela-windows-x86_64.msi>.

Write `hello.cdl`:

```rust
fn main() {
    print("Hello, world!");
}
```

Run it:

```sh
candela hello.cdl
```

Run `candela` with no arguments for a REPL.

## What you get

- **Types without ceremony.** Annotations are optional on locals and function
  parameters; the compiler infers the rest and reports mismatches before the
  program starts.
- **Structs, enums and match.** Struct fields can declare defaults, and
  `Opts { cwd: "a", ..Default::default() }` writes only what differs. Enum
  variants carry payloads, and `match` binds them by pattern.
- **Methods.** An `impl` block attaches methods to a struct, an enum, or a
  built-in type, called as `value.method()`.
- **Functions as values.** Pass a named function or an anonymous `fn(x) { ... }`
  to another function.
- **Collections.** List and map literals, a set built on maps, and JSON parsing
  and serialisation in the standard library.
- **One import form.** `import "std/json" as json;` for a namespace, or
  `import "std/option";` to bring the module's symbols into scope.
- **Code per target.** `@cfg(web)` compiles a declaration or a statement only
  where the host turns the flag on, so one program can carry a browser and a
  desktop version of the same function.
- **A standard library written in candela.** The `.cdl` sources ship beside the
  toolchain, so you can read any of it.
- **Projects and packages.** `candela new` starts a project, `candela add`
  pulls a package in from the registry, and `candela run` compiles and runs it
  against what the manifest lists. A package is another place an import
  resolves from, so nothing about importing changes.
- **Compiled artifacts.** `candela build` turns a source file into a `.cdlb`
  artifact, and `candela-vm` runs it. The artifact carries machine code for the
  functions the code generator handles, beside the bytecode for everything. The
  runtime binary links no parser, compiler, code generator or REPL.
- **Editor support.** A language server and five clients live in this
  repository: extensions for VS Code, the IntelliJ-based IDEs and Zed, and the
  grammar and configuration Neovim and Helix install. The VS Code extension
  installs from the Marketplace or Open VSX; the rest build from source.
- **Embedding.** A Rust host registers functions of its own, calls a script's
  functions by name, and exchanges values with it, either compiling the source
  in-process or loading a `.cdlb` with the compiler left out.
- **Macros the host defines.** `name!( ... )` hands the raw region between the
  parentheses to the embedding program, which returns candela source to parse in
  its place. candela ships no macros of its own and never interprets a region.

## Performance

Median wall-clock time in milliseconds for `candela file.cdl`, against Python 3
and LuaJIT with its JIT off, on an Intel Core i9-12900K. Lower is faster.

| Program | candela | Python 3 | LuaJIT (-joff) |
| --- | --- | --- | --- |
| Recursive fib | 94.2 | 273 | 132 |
| Iterative fib | 30.3 | 532 | 29.4 |
| Binary trees | 299 | 759 | 740 |
| Sqrt loop | 104 | 582 | 68.0 |
| FizzBuzz | 16.2 | 97.6 | 50.4 |

A `.cdlb` built with `candela build` runs faster again where its functions
become machine code. [BENCHMARKS.md](BENCHMARKS.md) has the full set, with
artifacts and LuaJIT's JIT, the programs, and how to run them yourself.

## Limitations

candela is pre-1.0 and the language is not stable; expect breaking changes
between releases. Anonymous functions do not capture their surrounding
scope, so pass what they need as arguments. The REPL re-runs the whole session
on every line, which repeats any side effects earlier lines had.

## Documentation

Full documentation is at <https://candela.lumenfx.dev>, and mirrored under
<https://docs.lumenfx.dev/candela>. To work on candela itself, read
[CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

Apache-2.0. See [LICENSE](LICENSE). candela is a fork of
[keel](https://github.com/horacehoff/keel) by Horace Hoff; see
[NOTICE](NOTICE).
