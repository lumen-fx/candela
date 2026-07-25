# candela-lsp

A language server for [candela](../README.md) (`.cdl` files), speaking LSP
over stdio. Built on `tower-lsp`, and built to *reuse* candela's own
lexer/parser/type-checker (`candela::compiler::compile`) rather than
reimplement any part of the language frontend -- see `src/analysis.rs` for
the boundary between "candela's compiler" and "this crate's presentation
logic".

This crate is a separate workspace member from the `candela` package (the
interpreter/CLI). That split is deliberate: `candela` produces the `candela`
binary, which must stay small and dependency-light, and must never link
`tower-lsp`/`tokio`/`lsp-types`. The dependency only goes one way,
candela-lsp -> candela; see the `[workspace]` comment in the repo root
`Cargo.toml` and the `BUILD NOTE` in this crate's `Cargo.toml`.

## Build

```sh
# From the repo root:
cargo build -p candela-lsp
```

**Do not build this crate with `--release` as-is.** The workspace's
`[profile.release]` (set by the `candela` package, to keep the interpreter
binary small) uses `panic = "abort"`. This crate's diagnostics pass depends
on `candela::collect_diagnostic`, which catches an internal unwind via
`std::panic::catch_unwind` -- that does not work under `panic = "abort"`, so
a `--release` build of `candela-lsp` would abort the whole language server
process on the first parse/type error in the user's buffer instead of
returning a `Diagnostic`. Build with the default `dev` profile, or the
repo's existing `embed` profile (`cargo build -p candela-lsp --profile
embed`), both of which use `panic = "unwind"`.

## Run

```sh
cargo test -p candela-lsp   # unit tests + the stdio smoke test, see below
```

The binary itself speaks LSP over stdin/stdout and expects to be launched by
an editor (see `../editors/vscode/src/extension.js` for the reference
client); it is not meant to be run interactively from a terminal.

## What's real vs. stubbed, and why

Every feature below is implemented by calling candela's own
`compiler::compile()` (parse + type-check + codegen, **without** running
`main` -- so re-analyzing on every keystroke has no script side effects) and
then reading the `CompileOutput` it returns (functions, structs, spans,
inferred-type caches). Nothing here reimplements parsing or type inference.

- **Diagnostics (REAL).** Parse and type errors surface as
  `textDocument/publishDiagnostics`. Caveat inherited from candela itself,
  not added here: candela's error funnel is **fatal-on-first-error** --
  `collect_diagnostic` returns at most one `Diagnostic` per compile, not a
  full error list. Fix the first reported error and re-save/re-type to see
  the next one, the same experience the `candela` CLI itself gives you.

- **Document sync (REAL).** Full-document sync (`didOpen`/`didChange`/
  `didSave`/`didClose`); a per-document cache also keeps the last
  *successful* compile's symbol table around, so hover/completion/outline/
  go-to-definition keep working while a buffer is mid-edit and currently
  failing to compile, instead of going blank on every syntax error.

- **Hover (REAL, with one honest gap).** Hovering a `struct` declaration or
  literal shows its fields with fully-resolved types (struct fields require
  explicit type annotations in candela, so this is never a guess). Hovering
  a function shows its declared parameters and the concrete
  `(arg types) -> return type` signature(s) it has actually been specialized
  for, sourced from `Function::return_type_cache` -- candela infers a
  function's return type per call site, so a function nothing calls yet
  genuinely has no inferred return type, and the hover says so rather than
  guessing. **Gap:** hovering a local variable use (as opposed to a function
  or struct) shows nothing. candela's compiler does not retain a per-AST-node
  inferred-type map after compilation -- variable types live only in an
  ephemeral `Vec<Variable>` during codegen -- so there is no durable type
  information to show for an arbitrary variable reference without
  instrumenting the compiler itself, which was out of scope here.

- **Completion (REAL for keywords/builtins/user symbols).** Keywords and
  built-in functions/methods come from a static table transcribed from
  `src/parser/lexer.rs` and `src/compiler/functions/builtin/*.rs` (see
  `src/builtins.rs` -- these are Rust match arms in candela, not a
  runtime-queryable table, so this table must be updated by hand if
  candela's built-ins change). User-defined function/struct names come live
  from the compiled program's symbol table. Typing `.` narrows completion to
  built-in methods only; there is no member-aware completion for struct
  fields (would need the receiver's static type at the cursor, which hits
  the same "no durable per-node type map" gap as hover on variables).

- **Document symbols / outline (REAL).** Top-level `fn`/`struct`
  declarations in the open buffer, from `Function`/`Struct` name spans.
  `Struct` (unlike `Function`) is not tagged with the source file that
  declared it by candela's compiler; this crate approximates "declared in
  this buffer" by checking that the struct's `name_span` byte range in the
  buffer text actually equals its own name (see `struct_is_in_buffer` in
  `src/analysis.rs`). That is a heuristic, not a compiler-guaranteed
  invariant, though a false positive would need a very unlikely span+name
  coincidence with an imported file.

- **Go-to-definition (REAL, simplified name resolution).** Jumps from a
  call site or struct literal to its declaration, including across an
  `import`ed file (candela records each imported file's absolute,
  canonicalized path, which this crate reads from disk to compute a
  precise range -- a synchronous read on the async task, fine for the small
  scripts candela targets, but a known rough edge for very large imported
  files). **Simplification:** resolution matches by bare (last-segment)
  name across the whole compiled program, not by fully resolving
  `namespace::name` qualification against the namespace tree. This is
  correct for the overwhelmingly common unqualified case and wrong only if
  two same-named functions/structs exist across different imported
  namespaces -- a real gap, not worth the added complexity for a first LSP.

## Testing

- `src/line_index.rs` has unit tests for the byte-offset <-> LSP `Position`
  conversion (including a multi-byte-character case, since LSP columns are
  UTF-16 code units, not bytes or chars).
- `tests/smoke.rs` is a headless integration test: it spawns the built
  `candela-lsp` binary as a real subprocess and drives it purely over the
  LSP JSON-RPC wire protocol (`initialize` -> `initialized` -> `didOpen` ->
  assert on `publishDiagnostics` -> `shutdown` -> `exit`), once with a
  deliberately broken `.cdl` snippet (expects a diagnostic) and once with a
  well-typed one (expects none). No editor or window is involved.
