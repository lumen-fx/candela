# candela-lsp

A language server for [candela](../README.md) (`.cdl` files), speaking LSP over
stdio. It reuses candela's own lexer, parser, and type checker
(`candela::compiler::compile`) instead of reimplementing the language frontend,
so its analysis always matches the compiler.

## Build

```sh
# From the repo root:
cargo build -p candela-lsp
```

Build with the default `dev` profile, or the repo's `embed` profile
(`cargo build -p candela-lsp --profile embed`). Do not build it with `--release`:
the workspace release profile uses `panic = "abort"`, under which the server's
diagnostics pass -- which catches an internal unwind to turn a compile error into
a `Diagnostic` -- would instead abort the whole server on the first error in the
user's buffer.

## Run

```sh
cargo test -p candela-lsp   # unit tests + the stdio smoke test, see below
```

The binary speaks LSP over stdin/stdout and expects to be launched by an editor
(see `../editors/vscode/` for the reference client); it is not meant to be run
interactively from a terminal.

## Features

Every feature re-analyzes the buffer by calling candela's compiler (parse +
type-check + codegen, without running `main`, so editing has no script side
effects) and reads the symbol tables it returns. A per-document cache keeps the
last successful compile's symbols around, so hover, completion, outline, and
go-to-definition keep working while a buffer is mid-edit and not yet compiling.

- **Diagnostics.** Parse and type errors are published as
  `textDocument/publishDiagnostics`. candela reports one error per compile, so
  fix the first and re-type to see the next -- the same flow as the `candela`
  CLI.

- **Document sync.** Full-document sync
  (`didOpen`/`didChange`/`didSave`/`didClose`).

- **Hover.** Shows a struct's fields with their declared types, and a function's
  parameters with the concrete `(arg types) -> return type` signatures it has
  been specialized for. candela infers a function's return type per call site,
  so a function nothing calls yet shows no return type. Hovering a local variable
  shows nothing: candela does not retain per-variable inferred types after
  compilation.

- **Completion.** Completes keywords, built-in functions and methods, and
  user-defined function and struct names from the compiled program. Typing `.`
  narrows to built-in methods; struct-field completion is not available.

- **Document symbols / outline.** Top-level `fn` and `struct` declarations in the
  open buffer.

- **Go-to-definition.** Jumps from a call site or struct literal to its
  declaration, including into an `import`ed file. Resolution matches by name
  across the compiled program; it does not distinguish two same-named symbols
  declared in different imported namespaces.

## Testing

- `src/line_index.rs` unit-tests the byte-offset <-> LSP `Position` conversion,
  including a multi-byte-character case (LSP columns are UTF-16 code units).
- `tests/smoke.rs` is a headless integration test: it spawns the built
  `candela-lsp` binary as a subprocess and drives it over the LSP JSON-RPC
  protocol (`initialize` -> `initialized` -> `didOpen` -> assert on
  `publishDiagnostics` -> `shutdown` -> `exit`), once with a broken `.cdl`
  snippet (expects a diagnostic) and once with a well-typed one (expects none).
  No editor or window is involved.
