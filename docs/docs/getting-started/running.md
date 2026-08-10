# Running programs

There are three ways to run candela code: straight from source, a line at a
time in the REPL, or from a compiled artifact. This page covers all three and
the command line around them.

## Run a source file

Name the file:

```sh
candela hello.cdl
```

There is no `run` subcommand; the first argument is the path to the program.
The compiler lexes, parses and type-checks the file and everything it imports,
then runs `main`. Anything you put after the file name reaches the program
through `argv()`:

```rust
fn main() {
    print("Hello, " + argv()[0] + "!");
}
```

```sh
candela greet.cdl Ada
```

```
Hello, Ada!
```

This is the mode to use while you are writing something. Compilation happens
every time, so an edit takes effect on the next run with nothing to clean up.

## The REPL

Start `candela` with no arguments:

```sh
candela
```

```
CANDELA 0.0.3 REPL (read-eval-print-loop)
>
```

Type a statement at the prompt and press enter. A trailing semicolon is added
for you when a line does not already end in `;` or `}`.

```
> let x = 21;
> print(x * 2);
42
```

The REPL keeps every line you have entered and re-runs the whole session each
time you add one. Lines beginning with `import` are hoisted to the top of the
program; everything else goes inside a synthesised `main`. What you see printed
is the output that is new since the previous run.

Two consequences are worth knowing. Side effects repeat: a line that writes a
file or reads input runs again on every subsequent line. And a line that fails
to compile prints its error and is dropped, so the session is always left in a
state that still works.

Press Ctrl+C to leave.

## Build an artifact

`candela build` compiles a source file to a `.cdlb` bytecode artifact:

```sh
candela build hello.cdl
```

It reports the file it wrote and its size. `compile` is accepted as another
spelling of `build`. The output file replaces
the `.cdl` extension with `.cdlb`; choose a different path with `-o` or
`--output`:

```sh
candela build hello.cdl -o dist/hello.cdlb
```

A `.cdlb` holds the whole program, its imports included, so the artifact
travels on its own. See [Artifacts](../reference/artifacts.md) for what else it
records.

## Run an artifact

`candela-vm` runs a compiled artifact:

```sh
candela-vm hello.cdlb
```

`candela-vm` is the runtime on its own. It links no lexer, parser, compiler or
REPL, and it never checks for updates. A program behaves the same whether you
run it from source or from an artifact, because both use this same runtime.

## Which to use

- **Source** while you write. One command, no build step, errors as you go.
- **The REPL** to try an expression or check what a standard library function
  returns.
- **An artifact** when you ship. Build once, then distribute the `.cdlb`
  alongside `candela-vm` and skip compilation on every start.

## Command line summary

`candela`:

- `candela <file.cdl> [args...]` runs a source file.
- `candela` with no arguments starts the REPL.
- `candela build <file.cdl> [-o out.cdlb]` compiles to an artifact.
- `candela --help` or `-h` prints usage.
- `candela --version` or `-v` prints the version.

`candela-vm`:

- `candela-vm <file.cdlb>` runs an artifact.
- `candela-vm --help` or `-h` prints usage.
- `candela-vm --version` or `-v` prints the version.

The [CLI reference](../reference/cli.md) documents both in full.

## Next

Start the language tour at [Variables](../language/variables.md).
