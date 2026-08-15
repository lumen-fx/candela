# candela-vm

The standalone runtime for [Candela](../README.md). It loads a pre-compiled
`.cdlb` bytecode artifact and runs it; it links no parser, compiler, or REPL,
and it never checks for updates.

```sh
candela build program.cdl     # (via `candela`) emit program.cdlb
candela-vm program.cdlb       # load and run the bytecode
```

`candela build` writes alongside the source, turning `program.cdl` into
`program.cdlb`; pass `-o`/`--output` to choose the path. `candela-vm` takes the
artifact path and nothing else, plus `--version` and `--help`.

The full `candela` toolchain runs on this same runtime, so a program behaves the
same whether run from source or from a `.cdlb`. The goal is to keep the
standalone binary under 1 MiB.

## What a .cdlb holds

An artifact starts with a magic marker and a format version, and the runtime
rejects one whose version it does not know rather than guessing at the layout.

Compilation resolves imports, so an artifact carries the whole program: the
modules a program imports are already linked into it, and it needs none of the
`.cdl` sources at run time.

Two things are recorded as recipes rather than contents:

- **Dynamic C libraries.** A `dylib` block stores the library name, the symbol,
  and its signature, never the library itself. The runtime resolves the library
  through the OS loader when the artifact loads, so the shared library has to be
  present on the machine that runs it.
- **Host functions.** A `host` block stores the namespace, name, signature, and
  whether the function is variadic. The Rust closures behind them come from the
  program that embeds the runtime, which registers them before it loads the
  artifact. The standalone `candela-vm` command registers none, so it refuses an
  artifact that declares one.

An artifact also carries an export table: a call trampoline per host-callable
function, compiled at build time. That is what lets an embedding program invoke
a script function by name with the compiler absent.

The full contract for the format, including how versions are matched, is in the
[artifacts reference](https://candela.lumenfx.dev/reference/artifacts/).

## Embedding without the compiler

Link this crate alone to run precompiled artifacts: register host functions on a
`HostRegistry`, load the `.cdlb` against it, and call exported functions by name.
The [embedding guide](https://candela.lumenfx.dev/integration/embedding/) covers
both this and the compiler-resident path.

## Features

`embed` makes a fatal script error unwind instead of exiting the process, so an
embedding host survives a script that dies. It is off by default.
