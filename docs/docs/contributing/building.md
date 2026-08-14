# Building candela

How to get the repository building, what is in it, and which gates to run before
you open a pull request.

## What you need

- A stable Rust toolchain. The crates are on edition 2024, so Rust 1.85 or
  newer. Nothing is pinned; CI builds on current stable.
- A C toolchain. The FFI dependencies build C as part of their own build, and
  the standard library's native modules are C.
- `clang`, if you want to build those native modules yourself.

Two more are needed only for specific jobs: `cargo-pgo` and the LLVM tools for a
profile-guided release build, and WiX for the Windows installer.

## Layout

The repository is a Cargo workspace of three crates:

- The root package, `candela-lang`. The lexer, parser, type checker, code
  generator, REPL, the `Engine`/`Program` embedding API, and the `candela`
  binary. The package name carries the `-lang` suffix only because crates.io
  gave `candela` to an unrelated project years ago; the library it builds is
  `candela`, so every `use candela::...` reads the same as it always did.
- `vm/`, the `candela-vm` crate. The self-contained runtime: the executor, the
  bytecode and value representation, the garbage collector, host and C value
  marshalling, and the `.cdlb` artifact format. It builds both a library and the
  `candela-vm` binary.
- `lsp/`, the `candela-lsp` crate. The language server, which reuses this
  repository's lexer, parser and type checker rather than reimplementing them.

Dependencies run one way only: `candela` depends on `candela-vm`, and
`candela-lsp` depends on `candela`. Nothing goes the other way, which is what
keeps the runtime free of the compiler. Both binaries link the same VM, so the
executor exists once. A reverse edge would be a dependency cycle and would fail
to resolve, so the rule enforces itself; keep it in mind when deciding where new
code belongs.

The rest of the tree:

- `libs/std/` holds the standard library, written in candela. `libs/std/tests/`
  holds one `.cdl` test program per module.
- `libs/std_src/` holds the C sources behind the `math`, `random` and `time`
  modules.
- `examples/` holds demo and benchmark programs, most of them alongside Python
  and Lua versions of the same thing. They double as the training corpus for
  profile-guided builds.
- `tests/` holds the Rust integration suites.
- `pgo/` holds the workloads and the small C library used to train a
  profile-guided release build.
- `msi/` holds the WiX package definition and the script that builds the Windows
  installer.
- `editors/vscode/` holds the VS Code extension: the language server client,
  the grammar, and snippets.
- `tools/jetbrains-candela/` holds the plugin for the IntelliJ-based IDEs. It is
  a Gradle build rather than a Cargo one, and it reads its grammar out of
  `editors/vscode/` so there is one grammar to fix.
- `docs/` holds this documentation site.

## Building

`cargo build` at the root builds the `candela-lang` package only. That is deliberate;
the workspace sets its default member to the root so the common case stays
quick. Name the others explicitly:

```sh
cargo build
cargo build -p candela-vm --bin candela-vm
cargo build -p candela-lsp
```

Two feature flags matter:

- `compiler` is on by default and gates everything the front end needs. It
  exists so the language server and the binary opt into their allocator and
  lexer dependencies rather than every consumer getting them.
- `embed` makes a fatal error unwind instead of ending the process, for building
  candela into a host program. See [embedding](../integration/embedding.md).

There is also a WebAssembly target, which drops the FFI dependencies:

```sh
cargo build --target wasm32-unknown-unknown
```

## Profiles

- `dev` is a lightly optimised debug profile with debug information off. Debug
  builds here are not stock Cargo debug builds.
- `release` is the shipping profile: full optimisation, fat link-time
  optimisation, one code generation unit, stripped, and aborting on panic.
- `debugrelease` is `release` with debug information kept and nothing stripped.
  Use it to profile or debug optimised code.
- `embed` is `release` with unwinding on panic. It is the profile to pair with
  the `embed` feature, and it is also the one to use for an optimised
  `candela-lsp`: the language server catches unwinds to turn errors into
  diagnostics, so it cannot run under a profile that aborts.

## Tests

```sh
cargo test
```

runs the root package's suites:

- The in-crate tests, which compile a snippet and run it through the VM, then
  assert on the resulting state or on what it printed. Run them alone with
  `cargo test --lib`.
- `tests/cdlb_roundtrip.rs`, covering the artifact format: the header, the
  instruction and constant tables, the type tables, the dynamic-library recipe
  round trip, the host-block refusal, and a full comparison of `candela` output
  against `candela-vm` running the artifact.
- `tests/embedding.rs`, covering the `Engine`/`Program` API: registering host
  functions, calling script functions, marshalling values both ways, state
  persistence, and the diagnostics returned on failure.
- `tests/imports.rs`, covering module binding rules, by writing multi-file
  programs to a scratch directory and running the built binary against them.
- `tests/std_library.rs`, which runs each `.cdl` program in `libs/std/tests/`
  through the binary and checks it exits cleanly. This is the standard library's
  test suite.

The other two crates are not reached by a root `cargo test`:

```sh
cargo test -p candela-vm
cargo test -p candela-lsp
```

The language server's suite spawns the built binary and drives it over its
protocol, so build it first.

Two tests skip rather than fail when their prerequisite is absent: the dynamic
library round trip, which needs a system zlib it can open, and the artifact
comparison, which needs the `candela-vm` binary built. Build
`-p candela-vm --bin candela-vm` before `cargo test` if you want that one to
run, and pass `-- --nocapture` to see whether anything skipped.

`CANDELA_LIB_PATH` points the compiler at a standard library directory. The
suites set it to this checkout's `libs`, because a test binary is not laid out
like an install, and deliberately clear it in the tests that prove resolution
works with no configuration.

Coverage comes from the same suites, measured with `cargo llvm-cov --workspace`
on every push to `main` and every pull request and reported to Codecov. It is a
report rather than a gate: what decides a pull request is the suite itself.
Doctests are outside the measurement, since collecting coverage from them needs
a nightly toolchain.

There is no fuzzing setup. Benchmarking is manual; `BENCHMARKS.md` describes it.

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test
cargo test --features embed
cargo build --target wasm32-unknown-unknown
```

Clippy's pedantic and nursery groups are on as warnings for the `candela`
package. Do not add new ones. If a lint is wrong for your case, allow it at the
narrowest scope with a comment saying why.

Continuous integration builds and tests on Linux with and without the `embed`
feature, builds for WebAssembly, type-checks both feature combinations, and
gates on `cargo fmt --all --check` and on clippy with warnings denied. A
separate job exercises the Windows installer end to end when the installer or
the update logic changes.

The crate documentation is built from the sources with

```sh
cargo doc --workspace --no-deps
```

and the same build is published on every push to `main`, serving the API
documentation for the current tree at
[api.candela.lumenfx.dev](https://api.candela.lumenfx.dev). That is the Rust
API, separate from this site.

Two policies from `CONTRIBUTING.md` are worth repeating. A change to the
language, the standard library or the command line updates the matching
documentation page in the same pull request. A change in behaviour comes with a
test: in `tests/` for the compiler, artifacts, imports or embedding, and in
`libs/std/tests/` for the standard library. Run a program through both `candela`
and `candela-vm` and confirm they agree.
