# Changelog

## 0.5.0

- With a `candela-lsp` from the same release: errors show while you type,
  every one the compiler finds rather than the first, on the file each is in.
  A call with arguments of the wrong type shows each one, naming its
  parameter and linking to its declaration. Files that import a changed file
  are checked again, against unsaved text too.
- `candela.diagnosticsDelay` sets how long after typing the check runs.
- A package's `check` command (`candela.toml`) runs after each save, and what
  it reports shows beside the compiler's errors. `candela.packageChecks`
  turns it off; it does not run in an untrusted workspace.
- The extension watches `.toml` files as well as `.cdl` files, so a changed
  settings file is checked again.

## 0.4.0

- `///` and `//!` comments color as documentation.
- With a `candela-lsp` from the same release: hover renders an item's `///`
  doc comment as Markdown, lists a struct's fields with their defaults and an
  enum's variants with the summary of each, and ends in an "Open docs" link
  when the item's package names a `docs` URL in its `candela.toml`.
  Completion items show the summary line of the doc comment. A plain `//`
  comment above a declaration is no longer shown.

## 0.3.0

- The `candela.importRoots` setting names the packages `import "name";` reads,
  so a script that imports a host's module (`import "engine" { * };`)
  compiles in the editor without a manifest. `candela.libPath` points the
  server at a standard library other than the one installed beside it. Both
  apply as soon as they change.
- With a `candela-lsp` from the same release: completion follows the cursor
  (fields and methods after `.`, variants after `Enum::`, the missing fields
  of a struct literal, attributes after `@` and their parameters and values),
  hover shows the `//` comment above a declaration, signature help shows a
  call's or an attribute's parameters, and go-to-definition reaches fields,
  methods, variants, attributes and locals.

## 0.2.3

- `throw` colours as a keyword: it is a statement, `throw "kind";`.
- `is` and `as` colour as operators: the type test `v is int` and the checked
  downcast `v as int`. The `ifis` snippet writes an `if` on a test.
- The `import` and `importlib` snippets write a plain import, which binds the
  module under its name; `importitems` writes `import "std/assert" { eq };`.

## 0.2.2

- The `fn` that opens a function type, `fn(int, int) -> int`, or an anonymous
  function, `fn(x) { ... }`, carries the same scope as the `fn` of a
  declaration, so a theme colours all three alike.

## 0.2.1

- The grammar reads generics instead of arithmetic: `Cell<int>` in a type,
  `first<int>(xs)` at a call, `Cell<int>{ .. }`, `Slot<int>::Filled(9)`,
  `impl Cell<T>` and `struct Cell<T>` all colour their angle brackets as
  brackets and their contents as types, while `a < b` and `a < b > c` stay
  comparisons. A name that carries a type argument list before its call
  parentheses or its literal brace colours as the function or struct it names.
- `->` and `=>` colour as arrows, rather than as a minus or an assignment
  followed by a greater-than.

## 0.2.0

- Added a language client (`vscode-languageclient`) that launches `candela-lsp`
  (see `../../lsp/`) over stdio for `.cdl` files: live diagnostics
  (parse/type errors), hover, completion, document symbols (outline), and
  go-to-definition. The extension now has a runtime dependency, so run
  `npm install` before packaging (see README.md).
- Added the `candela.languageServerPath` setting to point at a specific
  `candela-lsp` binary instead of relying on `PATH`.
- Raised the minimum VS Code version to `^1.91.0` (required by
  `vscode-languageclient@10`).

## 0.1.0

- Initial release.
- TextMate grammar (`source.candela`) for `.cdl` files: keywords, built-in
  types, structs, constants, strings with escapes, integer and float literals,
  operators, the `...` variadic marker, `import` / `host` / `dylib` constructs,
  function definitions and calls, method and `namespace::` access.
- Language configuration: line comments, brackets, auto-closing and surrounding
  pairs, and `{}` indentation rules.
- Snippets for common Candela constructs.
