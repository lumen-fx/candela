# Artifacts

A `.cdlb` file is a compiled candela program. You build one with the `candela`
toolchain and run it with `candela-vm`, the small runtime that carries no
parser, compiler or REPL.

## Building one

```sh
candela build game.cdl
```

This writes `game.cdlb` beside the source. Pass `-o` to choose the path. See
[the command line](cli.md).

The whole program goes into the one file. Every workspace `.cdl` module the
program imports, and every standard-library module it uses, is linked into the
artifact, so it runs with no source tree present.

## Running one

```sh
candela-vm game.cdlb
```

The artifact produces the same output as running the source through `candela`.
`candela-vm` loads it, binds anything it refers to by name, and runs `main`.

## What the artifact records

- The bytecode instructions and the register file they run against.
- The constant pools: strings, objects and maps built at compile time.
- The struct and enum type tables, with field and variant names.
- Per-function register layouts and the allocation sizes the VM needs up front.
- The source text of every file that went into the program, along with the span
  each instruction came from. This is what lets a runtime error from an artifact
  print the same underlined source report you get when running from source.
- A recipe for each dynamic-library binding: the library name exactly as written
  in the `dylib` block, the C symbol, and the marshalling signature.
- A recipe for each `host` function: its namespace, name, signature, and whether
  it is variadic.

The two recipe tables are references, not contents. A dynamic library's bytes
are never embedded; the runtime re-opens the library by name and re-resolves the
symbol when it loads the artifact, then rebuilds the calling interface from the
recorded signature. This keeps artifacts small and lets a system library be
upgraded underneath one, and it means the library has to be present wherever the
artifact runs. See [C libraries](../integration/c-libraries.md).

The same applies to the standard library. A program that imports only
pure-candela modules is self-contained. The `math`, `random` and `time` modules
bind a dynamic library, so an artifact using them needs that library at run
time. See [the standard library overview](../standard-library/overview.md).

## Version compatibility

The file starts with a four-byte marker and a one-byte format version. The
runtime accepts exactly the version it was built for and rejects anything else
rather than risk decoding it wrongly.

The version is raised whenever the shape of what is recorded changes: adding the
dynamic-library and host-function tables, adding the enum type table, and adding
the map, JSON and `any` operations each raised it. There is no forward or
backward compatibility across a change, and there is no conversion tool.

In practice this means: build the artifact with the toolchain whose `candela-vm`
will run it, and rebuild after a toolchain upgrade. The failure is loud, not
silent, so a stale artifact is reported rather than mis-run.

## What candela-vm refuses

- A file that does not carry the `.cdlb` marker, or is too short to hold a
  header.
- An artifact built for a different format version.
- An artifact whose body does not decode.
- An artifact whose dynamic library cannot be opened. The message names the
  library as written and the filename it resolved to.
- An artifact whose library opened but does not export a symbol it needs.
- An artifact that declares a `host` block. Host functions are supplied by an
  embedding program, and the standalone runtime has none to bind, so it names
  the function and refuses rather than fail at the call. Run such a program
  from a host instead; see [embedding](../integration/embedding.md).

All of these are reported before the program starts, so an artifact either runs
or tells you why it cannot.
