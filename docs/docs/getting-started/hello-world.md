# Hello, world

This page walks through the smallest candela program there is. If you have not
installed the toolchain yet, start with [Install](install.md).

## Write it

Put this in a file called `hello.cdl`:

```rust
fn main() {
    print("Hello, world!");
}
```

## Run it

```sh
candela hello.cdl
```

```
Hello, world!
```

## What happened

Four things are worth pulling out of those three lines.

**Execution starts at `main`.** Every program declares `fn main()` in the file
you name on the command line, and that function is what runs. A file without a
`main` is a compile error.

**The top level holds declarations, not statements.** `let x = 1;` outside a
function does not compile. Only functions, imports, structs, enums, `impl`
blocks and library declarations live at the top level; the work goes inside a
function body.

**`print` is built in.** A handful of functions are available with no import at
all, `print` among them. It writes its argument followed by a newline. The
[built-in functions](../standard-library/builtins.md) page lists the rest.

**Nothing runs until it type-checks.** candela reads the whole program, parses
it and checks the types before the first line executes. A misspelt name or a
number where a string belongs is reported up front rather than partway through
a run.

Statements end with a semicolon, and blocks are delimited with braces.

## A little further

Programs get more interesting once values have names:

```rust
fn main() {
    let name = "world";
    print("Hello, " + name + "!");
}
```

`let` introduces a variable. You do not write a type: the compiler works out
that `name` holds a string, and `+` on two strings joins them. Try `+` between
a string and a number and the program does not compile.

## Next

[Running programs](running.md) covers the REPL, compiled artifacts, and the
rest of the command line. To carry on with the language itself, start at
[Variables](../language/variables.md).
