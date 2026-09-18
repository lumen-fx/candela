# Changelog

## 0.2.2

- The `fn` that opens a function type, `fn(int, int) -> int`, colours as a type
  rather than as a call.

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
