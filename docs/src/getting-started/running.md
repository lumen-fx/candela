# Running programs

There are four ways to run candela code: straight from source, as a project, a
line at a time in the REPL, or from a compiled artifact. This page covers all
four and the command line around them.

## Run a source file

Name the file:

```sh
candela hello.cdl
```

The first argument is the path to the program; one file needs no verb in front
of it. The compiler lexes, parses and type-checks the file and everything it
imports, then runs `main`. Anything you put after the file name reaches the
program through `argv()`:

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
CANDELA <version> REPL (read-eval-print-loop)
Type exit to leave, or press Ctrl+D (Ctrl+Z then enter on Windows).
>
```

Type a line at the prompt and press enter. A trailing semicolon is added for
you when a line does not already end in `;`, `}` or `{`.

```
> let x = 21;
> print(x * 2);
42
```

A line takes anything a file takes. A statement goes inside a synthesised
`main`, and a top-level declaration goes above it, so a `struct`, an `enum`, an
`impl` block, a function, an `import`, a `dylib` block and a `host` block are
all typed at the prompt as they are written in a file:

```
> struct Point { x: int }
> impl Point { fn twice(self) -> int { return self.x * 2; } }
> let p = Point{x: 21};
> print(p.twice());
42
```

A line has to be complete on its own: a declaration that spans several lines
in a file is typed as one line here, and a line opening with `{` reads as a
block rather than a map literal.

A line that is an expression prints its value, so a name, a calculation or a
call with a result needs no `print` around it:

```
> x * 2
42
> p
Point {x:21}
```

The REPL keeps every line you have entered and re-runs the whole session each
time you add one. What you see printed is the output that is new since the
previous run.

Two consequences are worth knowing. Side effects repeat: a line that writes a
file or reads input runs again on every subsequent line. And a line that fails
to compile prints its error and is dropped, so the session is always left in a
state that still works.

Type `exit` to leave, or press Ctrl+D, or Ctrl+Z then enter on Windows. Ctrl+C
also works. `exit` takes a status the way the built-in of that name does, so
`exit(2)` leaves the prompt with status 2, and the argument can be anything the
session works out: `exit(code)` leaves with what `code` holds. The REPL stops
as soon as its input runs out, so a file of statements piped into `candela`
runs to the end and exits.

## Run a project

Once a program is more than one file, or depends on a package somebody else
wrote, it becomes a project: a directory with a `candela.toml` in it.

```sh
candela new hello
cd hello
candela run
```

`candela run` compiles the entry point the manifest names and runs it, with the
packages the project depends on resolved first. `candela check` compiles the
same program and stops before running, the body of every fully annotated
function in that file included, so a mistake in a function `main` never calls is
reported. A function an import brought in is compiled by the call that reaches
it, as it is on a run. Both take a file if you want a different one, and both
work from anywhere inside the project.

[Projects and packages](packages.md) covers the manifest, dependencies and
publishing.

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

That is the only way to name the output. A second path with no `-o` in front of
it is rejected rather than taken as the destination.

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
Arguments after the artifact reach the program through `argv()`, just as they do
after the file name of a source run:

```sh
candela-vm greet.cdlb Ada
```

## Which to use

- **Source** while you write. One command, no build step, errors as you go.
- **A project** once there is more than one file, or a dependency to pull in.
- **The REPL** to try an expression or check what a standard library function
  returns.
- **An artifact** when you ship. Build once, then distribute the `.cdlb`
  alongside `candela-vm` and skip compilation on every start.

## Command line summary

`candela`:

- `candela <file.cdl> [args...]` runs a source file.
- `candela` with no arguments starts the REPL.
- `candela new <name>` starts a project.
- `candela run [file.cdl] [args...]` runs the project.
- `candela check [file.cdl]` compiles without running.
- `candela build [file.cdl] [-o out.cdlb]` compiles to an artifact.
- `candela add`, `remove`, `fetch`, `update` and `publish` work on the
  project's dependencies.
- `candela --help` or `-h` prints usage.
- `candela --version` or `-v` prints the version.

`candela-vm`:

- `candela-vm <file.cdlb> [args...]` runs an artifact.
- `candela-vm --help` or `-h` prints usage.
- `candela-vm --version` or `-v` prints the version.

The [CLI reference](../reference/cli.md) documents both in full.

## Next

Start the language tour at [Variables](../language/variables.md).
