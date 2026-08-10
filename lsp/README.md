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

Build with the default `dev` profile or the repo's `embed` profile
(`cargo build -p candela-lsp --profile embed`). Do not build it with `--release`
or `--profile debugrelease`. Both set `panic = "abort"`, which would turn the
first compile error in a user's buffer into a full server crash rather than a
published diagnostic: the server catches the compiler's panic to recover the
diagnostic, and `catch_unwind` does nothing when panics abort.

## Run

The binary speaks LSP over stdin and stdout and expects to be launched by an
editor (see `../editors/vscode/` for the reference client). It is not meant to
be run interactively from a terminal.

## Features

Every feature re-analyses the buffer by calling candela's compiler (parse,
type-check, and codegen, without running `main`, so editing has no script side
effects) and reads the symbol tables it returns. A per-document cache keeps the
last successful compile's symbols around, so hover, completion, outline, and
go-to-definition keep working while a buffer is mid-edit and not yet compiling.

- **Diagnostics.** Parse and type errors are pushed as
  `textDocument/publishDiagnostics`. candela reports one error per compile, so
  fix the first and re-type to see the next, the same flow as the `candela` CLI.

- **Document sync.** Full-document sync (`didOpen`/`didChange`/`didClose`); each
  change replaces the whole buffer.

- **Hover.** On a struct, shows its fields with their declared types. On a
  function, shows its parameters with the concrete `(arg types) -> return type`
  signatures it has been specialised for; candela infers a function's return
  type per call site, so a function nothing calls yet shows no return type. On a
  built-in function or method, shows its documentation. Hover works on a use
  site as well as a declaration. Hovering a local variable shows nothing:
  candela does not retain per-variable inferred types after compilation.

- **Completion.** Completes keywords, built-in functions, and user-defined
  function and struct names from the compiled program, including symbols pulled
  in by `import`. `.` is a registered trigger character, and typing it narrows
  the list to built-in methods. Struct-field completion is not available.

- **Document symbols / outline.** `fn` and `struct` declarations from the open
  buffer, as a flat list.

- **Go-to-definition.** Jumps from a call site to the function's declaration,
  including into an `import`ed file. Struct literals resolve only within the
  open buffer.

## Known simplifications

These are deliberate, and the source refers here for them.

- Resolution matches by bare name across the compiled program. It does not
  distinguish two same-named symbols declared in different imported namespaces,
  and returns every match.
- A struct carries no source-file index, so anything that needs a struct's
  origin file (go-to-definition, and hover on a declaration) works only for
  structs declared in the open buffer.
- Jumping into an imported file reads that file from disk synchronously, on the
  request.
- The outline and completion list the functions the compiler produces, so a
  `fn` nested inside another function body appears alongside top-level ones, and
  a closure appears under the synthetic name the type checker hoists it to.
- Enums are compiled but do not appear in the outline.
- The server handles `didSave` but does not advertise it, so a client that sends
  only what is advertised never delivers one.

## Testing

- `src/line_index.rs` unit-tests the byte-offset to LSP `Position` conversion,
  including a multi-byte character and an out-of-range clamp. LSP columns are
  UTF-16 code units.
- `tests/smoke.rs` is a headless integration test: it spawns the built
  `candela-lsp` binary as a subprocess and drives it over the LSP JSON-RPC
  protocol (`initialize` -> `initialized` -> `didOpen` -> assert on
  `publishDiagnostics` -> `shutdown` -> `exit`), once with a broken `.cdl`
  snippet (expects a diagnostic) and once with a well-typed one (expects none).
  No editor or window is involved.

Run both with `cargo test -p candela-lsp`.
