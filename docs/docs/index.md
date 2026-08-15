# candela

candela is a small statically typed scripting language with Rust-like syntax.
It is the scripting language of the Lumen UI framework, and it works just as
well on its own.

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
it is readable.

Editor support comes from `candela-lsp`, the language server, which runs the
same compiler your build does and reports what it finds as you type. Two
clients ship in the repository: a VS Code extension in `editors/vscode`, and a
plugin for the IntelliJ-based IDEs in `editors/jetbrains`. Both add
syntax highlighting from the same grammar and get diagnostics, hover,
completion, an outline, and go to definition from the server. Build them from
the repository; neither is published to a marketplace yet.

## Where to go next

**Getting started** takes you from nothing to a running program.

- [Install](getting-started/install.md): the install script, the Windows
  package, updates and pinning.
- [Hello, world](getting-started/hello-world.md): your first program, line by
  line.
- [Running programs](getting-started/running.md): source, the REPL, artifacts
  and the command line.

**Language** is the tour, meant to be read in order.

- [Variables](language/variables.md) and [Types](language/types.md).
- [Control flow](language/control-flow.md) and
  [Functions](language/functions.md).
- [Methods](language/methods.md), [Enums](language/enums.md) and
  [Collections](language/collections.md).
- [Error handling](language/error-handling.md),
  [Modules](language/modules.md) and [Macros](language/macros.md).

**Standard library** is the lookup reference: an
[overview](standard-library/overview.md) of how the library ships and how to
import it, the [built-in functions](standard-library/builtins.md) that need no
import, and a page for each module.

**Reference** covers the [operators](reference/operators.md), the
[error catalogue](reference/errors.md), the [CLI](reference/cli.md), and the
[`.cdlb` artifact format](reference/artifacts.md).

**Integration** is for host applications:
[embedding candela in Rust](integration/embedding.md) and
[calling dynamic C libraries](integration/c-libraries.md) from candela.

**Contributing** explains how to
[build the toolchain](contributing/building.md) yourself.

Using candela inside a Lumen application is covered by the Lumen documentation
rather than this site; follow the Lumen bindings link in the navigation.
