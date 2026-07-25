# Changelog

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
