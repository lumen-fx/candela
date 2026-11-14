# candela-lsp

A language server for [candela](../README.md) (`.cdl` files), speaking LSP over
stdio. It reuses candela's own lexer, parser, and type checker
(`candela::check_only`) instead of reimplementing the language frontend, so its
analysis always matches the compiler. Like `candela check`, it opens no library
a `dylib` block names.

## Build

```sh
# From the repo root:
cargo build -p candela-lsp
```

Any profile in the repo builds a working server, `--release` included. The
server recovers a compile error in a user's buffer by catching the compiler's
unwind, so the one thing a profile must not do is set `panic = "abort"`: that
turns the first error a user types into a dead server rather than a published
diagnostic.

## Run

The binary speaks LSP over stdin and stdout and expects to be launched by an
editor (see `../editors/vscode/` for the reference client). It is not meant to
be run interactively from a terminal.

## Settings

The server reads where imports resolve from in the `initializationOptions`
the client sends, and again in every `workspace/didChangeConfiguration`. The
object may sit under a `candela` key:

```json
{ "importRoots": { "engine": "/opt/engine/module" }, "libPath": "/opt/candela/libs" }
```

- `importRoots` names the packages `import "name";` reads. A directory is a
  package root, entered through the file its `candela.toml` names, or
  `src/main.cdl`; a `.cdl` file is the module itself. A program that imports a
  host's module, which no manifest lists, needs this to compile in the editor.
- `libPath` is the directory holding `std/`, what `CANDELA_LIB_PATH` names.
  Without it the server reads `CANDELA_LIB_PATH`, then the `libs/` beside its
  own executable.

The VS Code extension sends both from its `candela.importRoots` and
`candela.libPath` settings.

## Features

Every feature re-analyses the buffer by calling candela's compiler (parse,
type-check, and codegen, without running `main`, so editing has no script side
effects) and reads the symbol tables it returns. A per-document cache keeps the
last successful compile's tables, so the features below keep working while a
buffer is mid-edit and not yet compiling. What the cursor is on is read from
the current text, so `bird.` completes while the line around it is unfinished.

- **Diagnostics.** Parse and type errors are pushed as
  `textDocument/publishDiagnostics`. candela reports one error per compile, so
  fix the first and re-type to see the next, the same flow as the `candela` CLI.
  The compile is the one `candela check` runs, so a buffer that fails on the
  command line fails in the editor: an error in the body of a function `main`
  never calls is reported here too, as long as that function annotates every
  parameter. A `main` is not required, so a library buffer analyses like any
  other. The warnings that command prints, such as a bare parameter on a
  function nothing calls, are pushed alongside at warning severity. Two more
  warnings cover attributes the compiler accepts but a host is unlikely to
  mean: the same attribute written twice on one item, and a variant passed for
  an `enum` parameter from an enum the declaring host marks with none of its
  attributes while it marks others.

- **Document sync.** Full-document sync (`didOpen`/`didChange`/`didClose`); each
  change replaces the whole buffer.

- **Completion.** What is offered depends on where the cursor is:
  - after `value.`, the fields and methods of the value's type. The type is
    followed from a parameter's annotation, a `let`, a `for` pattern
    (including each element of `for (a, b) in q` over a list of tuples), a
    field, a call's return type or a struct literal;
  - after `Enum::`, its variants; after `module::`, the module's names;
  - inside `Name { ... }`, the fields not written yet, and in a field's value
    the variants of its enum first;
  - after `@`, the attributes the program's `host` blocks declare, only those
    that go on the item below; inside `@name(...)`, the parameters not
    written yet, and in a parameter's value what its declaration accepts: the
    variants of a declared enum, the variants of the enums the declaring host
    marks for an `enum` parameter, the program's functions for a `fn` one, and
    its structs for a `type` one. Functions and structs that carry the
    attribute being written rank by how the program fills that parameter
    already, so ordering arguments list systems and predicates list the rest;
  - after `:` in a parameter list, a `let` or a struct declaration, and after
    `->`, the types in scope;
  - anywhere else, the bindings in scope with their types, the program's
    names (its own, the items its imports named or a glob import brought in,
    and `module::name`), builtins and keywords. At the top level, keywords and a
    skeleton for each declared attribute: `@system fn`, `@component struct`.
  `.`, `:` and `@` are trigger characters.

- **Hover.** The declaration under the cursor, written the way it reads in the
  source, with its `///` doc comment rendered as Markdown: a function's or
  method's signature, a struct's fields with their defaults, an enum's
  variants, a field's type and default, an attribute's parameters, and a
  local's or parameter's type. A struct's hover lists each field with the
  summary line of its doc comment, and an enum's each variant. Types are
  written the way a compiler error writes them. A function with an
  unannotated parameter also lists the signatures its call sites gave it.
  When the package that declares the item names a `docs` URL in its
  `candela.toml`, the hover ends in an "Open docs" link to the item.

- **Completion documentation.** Each item carries the summary line of its doc
  comment: the first paragraph.

- **Signature help.** Inside a call's parentheses, the function's or method's
  parameters, with the one being written highlighted; inside `@name(...)`, the
  attribute's parameters.

- **Document symbols / outline.** The functions, structs and enums the buffer
  declares.

- **Go-to-definition.** From a use of a function, method, struct, enum,
  variant, field, attribute or local to where it is declared, including into an
  imported module and a package from `importRoots`. A call written
  `geo::area(...)` goes to the `area` of the module bound as `geo`.

## Known simplifications

These are deliberate, and the source refers here for them.

- The type of a value is followed through the text, not the compiler: an
  expression it cannot follow (a closure's parameter, a `match` arm's binding,
  a generic function's return) completes with the builtin methods only.
- A struct or an enum carries no source-file index, so its declaring file is
  found by matching its name at its recorded position in each file.
- A method of a generic struct is known once the program uses an
  instantiation of the struct (`State<Phase>`).
- The server handles `didSave` but does not advertise it, so a client that sends
  only what is advertised never delivers one.

## Testing

- `src/analysis.rs` unit-tests `analyze` on the buffers whose handling is easy
  to get wrong: one using a macro the server has no expander for, one with an
  error beside such a macro, one whose only error is in the body of a function
  nothing calls, one with no `main`, one that compiles with a warning, and one
  whose `dylib` library does not exist. It also covers how a type is written
  out: a parameter and a struct field of a user enum, and the `any` slot.
- `src/ide.rs` unit-tests completion, hover, signature help, go-to-definition
  and the attribute warnings against a small game written for a host that
  declares its attributes in a `host` block, the way an engine module does,
  and the type of each expression form a local can be declared with.
- `src/line_index.rs` unit-tests the byte-offset to LSP `Position` conversion,
  including a multi-byte character and an out-of-range clamp. LSP columns are
  UTF-16 code units.
- `tests/smoke.rs` is a headless integration test: it spawns the built
  `candela-lsp` binary as a subprocess and drives it over the LSP JSON-RPC
  protocol, once with a broken `.cdl` snippet (expects a diagnostic), once with
  a well-typed one (expects none), once for hover on a function whose
  parameter is a user enum, and once with a package given in
  `initializationOptions`: completion on a buffer that does not compile,
  hover with the package's docs link, and go-to-definition into the package. A last one changes the
  settings after start-up and checks that the open buffer is checked again,
  then asks for signature help and the outline, saves with an error in the
  imported file, and closes the buffer.

Run both with `cargo test -p candela-lsp`.
