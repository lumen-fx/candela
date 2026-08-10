# Command line

The toolchain installs two commands. `candela` compiles and runs source, and
`candela-vm` runs a compiled artifact.

## candela

### candela &lt;file.cdl&gt; [arguments]

Compiles the file and everything it imports, then runs `main`. There is no
`run` subcommand; the first argument is the path.

```sh
candela hello.cdl
```

Arguments after the path are passed to the program, which reads them with
`argv()`.

### candela

With no arguments, starts the REPL. Each line you enter is appended to the
session and the whole session is re-run, so earlier definitions and output stay
in place; only new output is printed. A line that does not already end in `;` or
`}` gets a semicolon added, `import` lines are hoisted above everything else,
and a line that fails to compile is dropped so the session stays usable. Leave
with Ctrl+D, Ctrl+Z then enter on Windows, or Ctrl+C. The REPL also exits when
its input ends, so a file of statements can be piped in.

### candela build &lt;file.cdl&gt; [-o out.cdlb]

Compiles the file to a `.cdlb` bytecode artifact and writes it out, reporting
the path and the size. `compile` is accepted as the same command.

```sh
candela build game.cdl
candela build game.cdl -o dist/game.cdlb
```

Without `-o` (or its long form `--output`), the output name is the input with
`.cdl` replaced by `.cdlb`. A path that does not end in `.cdl` gets `.cdlb`
appended.

`-o` is the only way to name the output. Any other argument after the source
file is an error, so `candela build game.cdl dist/game.cdlb` says what it wants
instead of writing `dist/game.cdlb`.

The program is compiled exactly as it is for a normal run, so every compile
error listed in [errors](errors.md) can come out of this command. See
[artifacts](artifacts.md) for what the file contains and how to run it.

### candela --help

Prints the usage summary. Also the point at which an available update is
reported, if one is due. `-h` is the same.

### candela --version

Prints the version. `-v` is the same.

Neither flag takes anything else. An argument after either one is an error
rather than an argument the command quietly drops.

### Development flags

A `candela` built with debug assertions accepts two extra flags after the file
name. `--debug` prints the compilation and execution time around the run, and
`--debug-parser` compiles the file and stops without running it. Neither is
present in a released build.

### Exit status

`candela` exits with zero when the program finishes and non-zero when anything
fails: a file it cannot read, a compile error, an uncaught runtime error. A
program can choose its own status with `exit()`.

## candela-vm

`candela-vm` runs a `.cdlb` artifact. It contains no parser, compiler or REPL,
and it never checks for updates.

Its own options come before the artifact path, because everything from the path
onwards belongs to the program. An option it does not know is an error.

### candela-vm &lt;file.cdlb&gt; [arguments]

Loads the artifact and runs it. Arguments after the artifact are passed to the
program, which reads them with `argv()`, the same as arguments after the file
name of a source run.

```sh
candela-vm game.cdlb
candela-vm greet.cdlb Ada
```

### candela-vm --help

Prints the usage summary. `-h` is the same.

### candela-vm --version

Prints the version. `-v` is the same. The runtime ships with the toolchain and
carries the same version number as `candela`.

### Exit status

Zero when the program finishes. Two when the command line is wrong: no
arguments, or an option the runtime does not recognise. One when the file cannot
be read or the artifact cannot be loaded; the load failures are listed in
[errors](errors.md). An uncaught runtime error inside the program ends the
process the same way it does under `candela`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `CANDELA_LIB_PATH` | Names the directory holding the shipped `std/` and `std_src/` library directories, overriding the default location beside the executable |
| `CANDELA_NO_UPDATE_CHECK` | Set to any non-empty value to silence the update check |
| `CI` | Silences the update check, so build machines never reach the network |

## The update check

`candela` tells you when a newer release is out. The check runs only where a
person is reading the output: the REPL, and `candela --help`. Running a program
never triggers it, so a script's output and exit status are never affected. The
notice goes to standard error.

The check is skipped entirely when any of these hold:

- Standard error is not a terminal.
- `CANDELA_NO_UPDATE_CHECK` or `CI` is set.
- The binary was built from source rather than installed, which the installer's
  receipt file beside the executable identifies.
- The install is pinned. `install.sh --version` writes a `pinned` line into that
  receipt, and an install that chose its release is left alone.

At most one network request is made a day; the answer is cached under
`XDG_CACHE_HOME` (or `LOCALAPPDATA` on Windows). Only the response headers of
the release redirect are read, never a page body, and every step fails quietly,
so a broken check never gets in the way.

On Windows, `candela --help` also offers to install the update, because there is
an installer to hand it to and the process is about to exit. Answering anything
other than yes declines. The REPL never offers, since it is holding the prompt
that the answer would have to be typed at. See
[install](../getting-started/install.md).
