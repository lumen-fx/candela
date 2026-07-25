# Candela for Visual Studio Code

Editor support for the Candela language (`.cdl` files): TextMate syntax
highlighting, language configuration (comments, brackets, auto-closing,
indentation), and snippets.

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

## Install / development

This is a grammar-only extension; it has no runtime dependencies and needs no
build step.

- Run from source: open this folder (`editors/vscode`) in VS Code and press
  `F5` to launch an Extension Development Host, then open any `.cdl` file.
- Package a `.vsix`:

  ```sh
  npx --yes @vscode/vsce package
  ```

  Install the produced `.vsix` with
  `code --install-extension candela-0.1.0.vsix`.

## Roadmap: LSP

Candela does not yet ship a language server. Diagnostics, go-to-definition,
hover, and completion are planned once a Candela language server lands; this
extension will then contribute an LSP client alongside the grammar.

## License

Apache-2.0. See the repository `LICENSE` and `NOTICE`.
