# Candela for Visual Studio Code

Editor support for the Candela language (`.cdl` files): TextMate syntax
highlighting, language configuration (comments, brackets, auto-closing,
indentation), snippets, and a language server client (live diagnostics,
hover, completion, outline, go-to-definition) backed by `candela-lsp`.

Candela is the scripting language of the Lumen UI framework. It is a fork of
[keel](https://github.com/horacehoff/keel) by Horace Hoff, renamed and extended
for Lumen (source file extension `.cdl`). See the repository `NOTICE` for
attribution details.

## Features

- Syntax highlighting for:
  - Keywords: `if`, `else`, `match`, `while`, `for`, `in`, `loop`, `return`,
    `break`, `continue`, `try`, `catch`, `let`, `fn`, `struct`, `import`, `as`,
    `host`, `dylib`.
  - Built-in types: `int`, `float`, `bool`, `string`, plus user-defined structs.
  - Constants: `true`, `false`, `null`.
  - Strings with escape sequences (`\n`, `\t`, `\r`, `\\`, `\"`, `\0`).
  - Integer and float literals.
  - Operators, including the `...` variadic marker in `host` blocks and the
    `..` range operator.
  - `import`, `host`, and `dylib` constructs, struct declarations and literals,
    function definitions and calls, method calls, and `namespace::` access.
- Language configuration: line comments (`//`), bracket matching, auto-closing
  and surrounding pairs, and `{}` indentation rules.
- Snippets for `fn`, `main`, `struct`, `let`, `if` / `else`, `for`, `while`,
  `loop`, `match`, `host`, `dylib`, `import`, and `print`.
- A language server client that launches `candela-lsp` over stdio for live
  diagnostics, hover, completion, document symbols (outline), and
  go-to-definition. See `../../lsp/README.md` for exactly which of
  those are real vs. simplified, and why.

## Language facts

- Comments: line comments only (`// ...`). There is no block comment syntax.
- Strings: double-quoted, with the escapes listed above. No string
  interpolation.
- Numbers: integers are `i32` (`[0-9]+`); floats are `f64` and must be written
  with a decimal point (`[0-9]*.[0-9]+`).
- Structs: `struct Name { field: type, ... }`; instantiated with
  `Name { field: value }`; fields accessed with `x.field`.
- Foreign functions: `dylib "path" { rettype name(argtypes); ... }` for dynamic
  libraries and `host "namespace" { rettype name(argtypes); ... }` for
  host-registered closures. A trailing `...` argument marks a variadic host
  function.
- Modules: `import "file.cdl";` or `import "file.cdl" as alias;`, then call via
  `alias::func()`.

## Language server

This extension launches `candela-lsp` (a separate crate at `../../lsp/`
in this repo) as a subprocess over stdio, the same transport every LSP client
uses. It looks for a `candela-lsp` (or `candela-lsp.exe` on Windows) binary on
`PATH` by default; set `candela.languageServerPath` in your settings to point
at a specific binary instead (for example, one built locally with
`cargo build -p candela-lsp` from the repo root -- see that crate's README for
why NOT `--release` as-is).

If `candela-lsp` cannot be started, the extension shows an error message
instead of failing silently; the syntax highlighting/snippets/language
configuration above keep working either way, since those do not depend on the
server.

## Install / development

Unlike the 0.1.x grammar-only release, this extension now has one runtime
dependency (`vscode-languageclient`) that must be present in `node_modules`
when packaged, so an `npm install` step is required before packaging (there is
still no compile/build step -- `src/extension.js` is plain CommonJS, not
TypeScript).

- Install dependencies: `npm install` (from this directory).
- Run from source: open this folder (`editors/vscode`) in VS Code and press
  `F5` to launch an Extension Development Host, then open any `.cdl` file.
- Package a `.vsix`:

  ```sh
  npm install
  npx --yes @vscode/vsce package
  ```

  Install the produced `.vsix` with
  `code --install-extension candela-0.2.0.vsix`.

## License

Apache-2.0. See the repository `LICENSE` and `NOTICE`.
