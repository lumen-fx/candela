# Candela for Visual Studio Code

Editor support for the Candela language (`.cdl` files): TextMate syntax
highlighting, language configuration (comments, brackets, auto-closing,
indentation), snippets, and a language server client (live diagnostics,
hover, completion, signature help, outline, go-to-definition) backed by
`candela-lsp`.

Candela is the scripting language of the Lumen UI framework. It is a fork of
[keel](https://github.com/horacehoff/keel) by Horace Hoff, renamed and extended
for Lumen (source file extension `.cdl`). See the repository `NOTICE` for
attribution details.

## Features

- Syntax highlighting for:
  - Keywords: `if`, `else`, `match`, `while`, `for`, `in`, `loop`, `return`,
    `break`, `continue`, `try`, `catch`, `throw`, `let`, `fn`, `struct`,
    `enum`, `impl`, `import`, `as`, `is`, `host`, `dylib`.
  - Built-in types: `int`, `float`, `bool`, `string`, plus user-defined structs
    and enums.
  - Function types, `fn(int, int) -> int`, wherever a type goes.
  - Constants: `true`, `false`, `null`.
  - Strings with escape sequences (`\n`, `\t`, `\r`, `\\`, `\"`, `\0`).
  - Integer and float literals.
  - Operators, including the `...` variadic marker in `host` blocks and the
    `..` range operator.
  - `import`, `host`, and `dylib` constructs, struct declarations and literals,
    `enum` and `impl` declarations, function definitions and calls, method
    calls, and `namespace::` access.
  - Generics: a declaration's type parameters (`struct Cell<T>`, `fn first<T>`,
    `impl Signal<int>`) and the type arguments a use names (`Cell<int>` in a
    type, `first<int>(xs)`, `Cell<int>{ .. }`, `Slot<int>::Filled(9)`) color as
    types inside angle brackets. `a < b` and `a < b > c` stay comparisons: a
    list is read only where the closing `>` is followed by something no operand
    can start with. A list written across two lines is not recognized.
  - Macro invocations, `name!( ... )`: the name colors as a macro rather than a
    function, and the region between the parentheses stays raw, so markup
    written inside one is not read as candela strings and operators.
- Language configuration: line comments (`//`), bracket matching, auto-closing
  and surrounding pairs, and `{}` indentation rules.
- Snippets, by prefix: `main`, `fn`, `struct`, `let`, `if`, `ifelse`, `ifis`,
  `for`, `forin`, `while`, `loop`, `match`, `host`, `dylib`, `import`,
  `importlib`, and `print`.
- A language server client that launches `candela-lsp` over stdio for live
  diagnostics, hover, completion, signature help, document symbols (outline),
  and go-to-definition. See `../../lsp/README.md` for the exact scope of each.

## Language facts

- Comments: line comments only (`// ...`). There is no block comment syntax.
  `///` documents the item below it and `//!` the file it is in; both color as
  documentation.
- Strings: double-quoted, with the escapes listed above. No string
  interpolation.
- Numbers: integers are `i64` (`[0-9]+`); floats are `f64` and must be written
  with a decimal point (`[0-9]*.[0-9]+`).
- Structs: `struct Name { field: type, ... }`; instantiated with
  `Name { field: value }`; fields accessed with `x.field`.
- Foreign functions: `dylib "path" { rettype name(argtypes); ... }` for dynamic
  libraries and `host "namespace" { rettype name(argtypes); ... }` for
  host-registered closures. A trailing `...` argument marks a variadic host
  function.
- Modules: `import "file.cdl";` for a file, `import "std/json";` for a shipped
  library. An import binds the module under its name (`json::parse`);
  `import "..." as alias;` picks the name, and `import "..." { a, b };` brings
  the named items into the file's scope.
- Macros: `name!( ... )` hands the raw region between the parentheses to the
  program embedding candela, which returns candela source to parse in its
  place. The region ends at the parenthesis that balances the one that opened
  it, skipping parentheses inside a string or after `//`. candela defines no
  macros of its own and never reads a region.

## Language server

This extension launches `candela-lsp` (a separate crate at `../../lsp/`
in this repo) as a subprocess over stdio, the same transport every LSP client
uses. It looks for a `candela-lsp` (or `candela-lsp.exe` on Windows) binary on
`PATH` by default; set `candela.languageServerPath` in your settings to point
at a specific binary instead (for example, one built locally with
`cargo build -p candela-lsp` from the repo root; see that crate's README for why
to avoid `--release`).

A script that imports a package no manifest lists, such as the module a host
program ships (`import "engine" { * };`), needs the server to know where it
is. Name it in `candela.importRoots`, a map from the name the script imports
to the package's directory or its `.cdl` file:

```json
"candela.importRoots": { "engine": "~/engine/module" }
```

`~` and `${workspaceFolder}` expand, and a relative path is read from the first
workspace folder. `candela.libPath` points the server at a standard library
other than the one installed beside it, the directory `CANDELA_LIB_PATH` would
name. Changing either re-checks the open files.

### Diagnostics

Errors show while you type, a moment after you stop: every type and name
error the compiler finds, each where it is, in the file it is in. A call whose
arguments are out of order shows each argument that does not fit, naming the
parameter it lands on, with a link to where that parameter is declared. A
change to a file re-checks the open files that import it, using the text in
the editor even before it is saved. `candela.diagnosticsDelay` sets the wait,
in milliseconds (250 by default).

A package can report what only it knows: an engine that reads attributes,
settings files and asset paths when it loads a program. It names a command
as `check` in its `candela.toml`, and after each save the server runs it for
the programs that import the package and shows what it prints beside the
compiler's errors, on any file it names, including settings files. Set
`candela.packageChecks` to `false` to turn this off; it is off in a workspace
you have not trusted.

If `candela-lsp` cannot be started, the extension shows an error message
instead of failing silently; the syntax highlighting/snippets/language
configuration above keep working either way, since those do not depend on the
server.

## Install / development

This extension has one runtime dependency (`vscode-languageclient`) that must be
present in `node_modules` when packaged, so run `npm install` before packaging.
There is no compile step; `src/extension.js` is plain CommonJS, not TypeScript.

- Install dependencies: `npm install` (from this directory).
- Run from source: open this folder (`editors/vscode`) in VS Code and press
  `F5` to launch an Extension Development Host, then open any `.cdl` file.
- Package a `.vsix`:

  ```sh
  npm install
  npx --yes @vscode/vsce package
  ```

  Install the produced `.vsix` with
  `code --install-extension candela-<version>.vsix`.

The extension carries its own version, independent of the candela toolchain
release it ships alongside. A change here bumps `version` in `package.json` and
adds a `CHANGELOG.md` entry; without that the next release publishes nothing new,
because the registries already hold the version in the manifest.

## License

Apache-2.0. See the repository `LICENSE` and `NOTICE`.
