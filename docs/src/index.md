<!--
This file is derived from keel (https://github.com/horacehoff/keel),
Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
It has been modified by the candela authors. See the NOTICE file.
-->
# candela

candela is a small statically typed scripting language with Rust-like syntax.
It is the scripting language of the Lumen UI framework, and it works just as
well on its own.

It began as a fork of [keel](https://github.com/horacehoff/keel), a language
written by Horace Hoff; candela's compiler, virtual machine and type system
are built on that work.

## Who it is for

candela is for people who want a scripting language that catches mistakes
before the program starts. Types are checked at compile time, but you rarely
write one down: annotations are optional on variables and function parameters,
and the compiler works out the rest. What you get is a language that reads like
a script and fails like a compiler.

It suits two jobs in particular. The first is scripting an application: a host
program written in Rust can compile candela source, hand it functions to call,
and exchange values with it. The second is standalone programs that need a
short, predictable start, because you can compile ahead of time and ship a
runtime that carries no compiler.

## A taste

```rust
enum Shape {
    Square(int),
    Rect(int, int),
}

impl Shape {
    fn area(self) {
        match self {
            Square(s) => { return s * s; }
            Rect(w, h) => { return w * h; }
        }
    }
}

fn main() {
    let shapes = [Square(3), Rect(2, 5)];
    let total = 0;
    for shape in shapes {
        total += shape.area();
    }
    print("total area: " + str(total));
}
```

```
total area: 19
```

Enum variants carry payloads and `match` binds them. An `impl` block gives a
type methods you call with a dot. Lists are literals and `for` walks them. Not
one type annotation appears, yet every one of those types is known before the
program starts.

## The toolchain

Two programs install together. `candela` is the compiler: it runs source files,
hosts the REPL, and compiles a `.cdl` source file into a `.cdlb` bytecode
artifact. `candela-vm` is the runtime alone, which runs an artifact and links
no parser, compiler or REPL.

The standard library ships beside them as candela source, so every function in
it is readable. `lpm`, the registry client, installs alongside and fetches the
packages a project depends on.

Editor support comes from `candela-lsp`, the language server, which runs the
same compiler your build does and reports what it finds as you type.
Diagnostics, hover, completion, an outline, and go to definition come from it,
so every client gets the same answers. Highlighting is the client's own, from
one of two grammars, and `editors/` holds five clients:

- `editors/vscode`, for VS Code: a TextMate grammar, with snippets.
- `editors/jetbrains`, for the IntelliJ-based IDEs: the same TextMate grammar,
  which the build reads out of the VS Code extension.
- `editors/zed`, for Zed: the tree-sitter grammar, which Zed clones and
  compiles itself.
- `editors/tree-sitter/nvim`, for Neovim: the tree-sitter grammar and its
  queries, through `nvim-treesitter`.
- `editors/tree-sitter/helix`, for Helix: the tree-sitter grammar, fetched and
  built by `hx --grammar`.

Every one of them can be pointed at a server you built yourself instead of the
one on your path.

The two grammars are written separately and do not cover the same syntax, so a
construct may colour in one editor before the other. Install the VS Code
extension from the Marketplace or Open VSX; the rest build from this
repository. Setup for each is in its own `README.md`, except Neovim and Helix,
which share `editors/tree-sitter/README.md`.

## Where to go next

**Getting started** takes you from nothing to a running program.

- [Install](getting-started/install.md): the install script, the Windows
  package, updates and pinning.
- [Hello, world](getting-started/hello-world.md): your first program, line by
  line.
- [Running programs](getting-started/running.md): source, the REPL, artifacts
  and the command line.
- [Projects and packages](getting-started/packages.md): `candela.toml`,
  dependencies, the lock file and publishing.

**Language** is the tour, meant to be read in order.

- [Variables](language/variables.md) and [Types](language/types.md).
- [Control flow](language/control-flow.md) and
  [Functions](language/functions.md).
- [Methods](language/methods.md), [Enums](language/enums.md),
  [Generics](language/generics.md) and [Collections](language/collections.md).
- [Error handling](language/error-handling.md),
  [Modules](language/modules.md) and [Macros](language/macros.md).

**Standard library** is the lookup reference: an
[overview](standard-library/overview.md) of how the library ships and how to
import it, the [built-in functions](standard-library/builtins.md) that need no
import, and a page for each module.

**Reference** covers the [operators](reference/operators.md), the
[error catalogue](reference/errors.md), the [CLI](reference/cli.md), the
[manifest](reference/candela-toml.md), and the
[`.cdlb` artifact format](reference/artifacts.md).

**Integration** is for host applications:
[embedding candela in Rust](integration/embedding.md) and
[calling dynamic C libraries](integration/c-libraries.md) from candela.

**Contributing** explains how to
[build the toolchain](contributing/building.md) yourself.

Using candela inside a Lumen application is covered by the Lumen documentation
rather than this site; follow the Lumen bindings link in the navigation.
