# Command line

The toolchain installs two commands. `candela` compiles and runs source, and
`candela-vm` runs a compiled artifact.

## candela

### candela &lt;file.cdl&gt; [arguments]

Compiles the file and everything it imports, then runs `main`. The first
argument is the path; one file needs no verb in front of it.

```sh
candela hello.cdl
```

Arguments after the path are passed to the program, which reads them with
`argv()`.

This form reads no manifest and resolves no dependencies. A file that imports a
package the project depends on runs under `candela run`.

### candela

With no arguments, starts the REPL. Each line you enter is appended to the
session and the whole session is re-run, so earlier definitions and output stay
in place; only new output is printed. A line takes anything a file takes: a
statement, or a top-level declaration (`fn`, `struct`, `enum`, `impl`,
`import`, `dylib`, `host`), which goes above the synthesised `main` while
everything else goes inside it. A line has to be complete on its own, and one
opening with `{` reads as a block rather than a map literal. A line that is an
expression prints its value. A line that does not already end in `;`, `}` or
`{` gets a semicolon added, and a line that fails to compile is dropped so the
session stays usable. A line that compiles and then fails at run time is
dropped too, having run once. Leave by typing `exit`, which takes a status the
way the built-in of that name does (`exit(2)`, or `exit(code)` for one the
session works out), or with Ctrl+D, Ctrl+Z then enter on Windows, or Ctrl+C.
The REPL also exits when its input ends, so a file of statements can be piped
in.

### candela new &lt;name&gt;

Creates `name/candela.toml` and `name/src/main.cdl`, a project that prints
`Hello, world!`. A directory that is already there is an error; nothing is
overwritten.

### candela run [file.cdl] [arguments]

Compiles and runs the project. With no file, the entry point comes from the
manifest ([`[package] entry`](candela-toml.md), `src/main.cdl` by default);
with one, that file runs instead. Arguments after either reach the program
through `argv()`.

```sh
candela run
candela run Ada
candela run tools/report.cdl --verbose
```

An argument ending in `.cdl` is the file; anything else is the program's. To
pass an argument that looks like a source path, name the entry point yourself.

Dependencies are resolved first, so the packages the manifest lists are
downloaded and their import roots are handed to the compiler.

### candela check [file.cdl]

Compiles the program and stops before running it, reporting the path when it
holds and the compile error when it does not. The compile is the one `build`
does, so an error in the body of a function `main` never calls fails the check
too, as long as that function is in the file being checked and annotates every
parameter. Dependencies are resolved first, so this also proves the manifest is
satisfiable. A `main` is not required here, the way it is for a run or a build:
a library whose entry only declares functions for other projects to import
checks like any other file.

### candela build [file.cdl] [-o out.cdlb] [--debug]

Compiles to a `.cdlb` bytecode artifact and writes it out, reporting the path
and the size. `compile` is accepted as the same command.

```sh
candela build
candela build game.cdl
candela build game.cdl -o dist/game.cdlb
candela build game.cdl --debug
```

A build compiles in the release profile, which runs passes that make the
program faster: a small function that calls no other candela function is
copied into the places that call it. The release program prints the same
output and raises the same errors, with the same messages and spans, as the
program compiled in the debug profile that `candela <file.cdl>`, `candela run`
and an embedding host use. `--debug` builds in the debug profile instead, with
no passes.

With no file, the entry point comes from the manifest. Without `-o` (or its
long form `--output`), the output name is the input with `.cdl` replaced by
`.cdlb`; a path that does not end in `.cdl` gets `.cdlb` appended.

`-o` is the only way to name the output. Any other argument is an error, so
`candela build game.cdl dist/game.cdlb` says what it wants instead of writing
`dist/game.cdlb`.

The program is compiled as it is for a normal run, so every compile error
listed in [errors](errors.md) can come out of this command. The build checks
more than a run does: every function in the file being built whose parameters
are all annotated is compiled at those declared types, so an error in the body
of a function `main` never calls fails the build here. A package a project depends on is compiled
into the artifact like any other import, so the `.cdlb` runs with neither the
source tree nor the package cache present. See [artifacts](artifacts.md) for
what the file contains and how to run it.

### candela add &lt;name&gt;[@requirement]

Resolves the package, downloads it, records it in `[dependencies]`, and updates
the lock file. Without a requirement, what is recorded is the version that was
resolved, at its major and minor (`1.2.3` becomes `^1.2`). With one, the
requirement is recorded as written.

```sh
candela add shapes
candela add shapes@^2
candela add shapes@1.2.3
```

The manifest is edited in place: comments, key order, and the table form of an
existing entry all survive.

### candela remove &lt;name&gt;

Drops the entry from `[dependencies]` and resolves what is left, so the lock
file follows. A name the manifest does not list is an error.

### candela fetch [--locked] [--offline]

Resolves everything the manifest lists and reports each package with its
version, target, and where it unpacked to.

`--locked` refuses to change `candela.lock`: a lock file that disagrees with
the manifest is an error rather than a rewrite. This is the form for a build
machine.

### candela update [name...]

Moves dependencies to newer versions the manifest still allows and rewrites the
lock file. With names, only those move; with none, all of them do.

### candela publish --url &lt;url&gt;

Publishes the manifest's version to the registry, pointing it at the archive at
`url`. The package name is registered first if the registry has not seen it,
and the release carries the manifest's dependencies. It carries the toolchain
requirement `[package] candela` names too, or the version of the candela doing
the publishing when the manifest names none, and prints which one it recorded.

A name the registry already holds is the one refusal the command carries on
past, since it means the name is there to release against; who holds it makes
no difference to the answer. Any other refusal stops it, and the release is not
published; the registry's own message is what you see.

A release that fails after the name went in leaves the name registered, so
running the command again carries past the registration and retries the
release.

Without `--url` the command prints the reusable workflow that builds and
uploads the archive for you, and exits 2. See
[projects and packages](../getting-started/packages.md).

### candela --help

Prints the usage summary. Also the point at which an available update is
reported, if one is due. `-h` is the same.

### candela --version

Prints the version. `-v` is the same.

Neither flag takes anything else. An argument after either one is an error
rather than an argument the command quietly drops.

### Development flags

A `candela` built with debug assertions accepts two extra flags after the file
name. `--debug` dumps the compiled program: the array pool, every register with
the value it starts on, and the instruction stream. A register that an array,
struct, map or enum construction fills at run time starts on that
construction's own template, so it dumps the shape and the constant parts of
the value it is about to hold. It also reports what parsing, compiling and
running each took. `--debug-parser` compiles the file and stops without running
it. Neither flag is present in a released build.

### Exit status

`candela` exits with zero when the program finishes and non-zero when anything
fails: a file it cannot read, a compile error, an uncaught runtime error. A
command line that does not say what to do exits two, as does `candela publish`
with no `--url`. A program can choose its own status with `exit()`.

## Resolving dependencies

`run`, `check`, `build`, `add`, `remove`, `fetch` and `update` resolve the
packages `candela.toml` lists. The work is done by `lpm`, the registry client:
candela hands it the requirements and reads back where each package unpacked.
It does not resolve versions, write the lock file, or manage the cache itself.

A project with no `[dependencies]` never reaches for the client at all.

### Finding the client

`LPM_BIN` names the client to run. It is an override: candela runs that binary
and looks nowhere else, and one that cannot answer for `lpm` is an error naming
the path and what it answered instead. A client too old to use is reported the
same way, rather than replaced with a download. That is what makes a stand-in
safe to point candela at: a mistake in it stops the command instead of handing
the work to the real client the machine also has.

An empty `LPM_BIN` counts as unset, and a value with no directory separator in
it is a command name looked up on `PATH`, so write `./lpm` for a file in the
working directory.

With `LPM_BIN` unset, candela looks in two places, in order:

1. `PATH`.
2. `~/.local/bin/lpm`, or `%LOCALAPPDATA%\Programs\lpm\lpm.exe` on Windows.

The second is the shared path every client of the registry installs to, which is
why it is a fixed location rather than something under the candela prefix.

When neither holds a client, or the one found is older than candela needs,
candela downloads the newest release from the registry's own repository,
checks it against the checksums published beside it, unpacks it to the shared
path, and says on standard error where it went. A download that does not match
its checksum installs nothing. The download needs `curl` or `wget` to fetch
with, `tar` to unpack with, and one of `sha256sum`, `shasum` or `certutil` to
hash with; a machine with none of one group gets an error naming what is
missing rather than an unverified client.

The oldest client candela works with is fixed at build time. `candela --version`
does not report it; an older one found on `PATH` or at the shared path is
replaced without being asked about.

### Offline and locked

`--offline` on `run`, `check`, `build` or `fetch` resolves from the packages
already downloaded and never reaches the network. A package that is not there
is an error naming it.

`--locked` on `fetch` refuses to change `candela.lock`. A lock file that
disagrees with the manifest is an error rather than a rewrite.

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
| `CANDELA_LIB_PATH` | Names the directory holding the shipped `std/` and `std_src/` library directories, overriding the default location beside the executable. `candela` reads it to resolve a library import, `candela-vm` to re-open a dynamic library the standard library owns |
| `LPM_BIN` | Names the registry client to resolve dependencies with. An override: nothing else is looked at, and one that cannot answer is an error |
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
