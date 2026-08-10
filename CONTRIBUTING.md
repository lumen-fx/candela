# Contributing to Candela

Contributions and issues are welcome.

If you found a bug, open an issue. If you have an idea for a new feature, a
design change, or anything that changes the core logic, open an issue to discuss
it first. For smaller changes that leave the core logic alone (typos, docs,
performance work), open a pull request directly.

I have the final say on what gets merged.

**AI use**: using AI is fine as long as you have read and understood the part of
the codebase you are changing.

## Where things live

- `src/` is the `candela` package: lexer, parser, type checker, code generator,
  REPL, the `Engine`/`Program` embedding API, and the CLI.
- `vm/` is the `candela-vm` crate: the runtime core, plus the standalone
  `candela-vm` binary that runs a compiled `.cdlb` and links no compiler.
  `candela` depends on `candela-vm`, never the reverse.
- `lsp/` is the `candela-lsp` crate: the language server. It depends on
  `candela` as a library to reuse the frontend, so its analysis matches the
  compiler.
- `libs/std` is the standard library, written in Candela as `.cdl` files;
  `libs/std_src` holds the native sources behind it.
- `editors/vscode` is the VS Code extension and its language client.

The workspace root builds only the `candela` package, so build and test the
other crates by name: `cargo build -p candela-vm`, `cargo test -p candela-lsp`.

## Before opening a pull request

Run what CI runs:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test
cargo test --features embed
cargo build --target wasm32-unknown-unknown
```

If you touched `vm/` or `lsp/`, add `cargo test -p candela-vm` and
`cargo test -p candela-lsp`; a plain `cargo test` at the root does not reach
them.

The workspace enables clippy's `pedantic` and `nursery` groups as warnings. Do
not add new ones; if a lint is wrong for your case, allow it locally with a
comment saying why.

A change to the language, the standard library, or the CLI updates the matching
page under `docs/` in the same pull request. A change to how a program behaves
should come with a test: `tests/` covers the compiler, artifacts, imports and
embedding, and `libs/std/tests` covers the standard library.

Keep the code fast, then simplify it as far as it goes without giving that back.
