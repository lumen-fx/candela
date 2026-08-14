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

## Quick start

Install on Linux or macOS:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh
```

On Windows, run the per-user installer from
<https://github.com/lumen-fx/candela/releases/latest/download/candela-x86_64-windows.msi>.

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
- **Structs, enums and match.** Enum variants carry payloads, and `match`
  binds them by pattern.
- **Methods.** An `impl` block attaches methods to a struct or an enum, called
  as `value.method()`.
- **Functions as values.** Pass a named function or an anonymous `fn(x) { ... }`
  to another function.
- **Collections.** List and map literals, a set built on maps, and JSON parsing
  and serialisation in the standard library.
- **One import form.** `import "std/list" as list;` for a namespace, or
  `import "std/option";` to bring the module's symbols into scope.
- **A standard library written in candela.** The `.cdl` sources ship beside the
  toolchain, so you can read any of it.
- **Compiled artifacts.** `candela build` turns a source file into a `.cdlb`
  bytecode artifact, and `candela-vm` runs it. The runtime binary links no
  parser, compiler or REPL.
- **Editor support.** A language server, a VS Code extension, and a plugin for
  the IntelliJ-based IDEs live in this repository and build from source.
- **Embedding.** A Rust `Engine`/`Program` API lets a host compile a program,
  register host functions, and exchange values with it.

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
