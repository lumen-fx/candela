# tree-sitter-candela

Tree-sitter grammar for candela (`.cdl`).

Editors use it for syntax highlighting, structural selection, and bracket
matching, and it is what the Neovim, Helix, and Zed integrations are built on.
It describes the shape of a program: declarations, statements, expressions with
candela's precedence, and the type syntax that appears in annotations and in
foreign function signatures. Whether a name resolves or a type checks is the
compiler's business, and `candela-lsp` reports that from candela's own
frontend.

The generated parser is committed, so an editor builds it without the
tree-sitter CLI. Highlighting queries live in [`queries`](queries). The Zed
extension is separate, at [`../zed`](../zed), and carries its own copy of the
queries.

## Neovim

Needs Neovim 0.11 or newer, `nvim-treesitter`, and a C compiler. Copy
`nvim/candela.lua` to `~/.config/nvim/lua/candela.lua`, call
`require('candela').setup()` from your config, install the parser with
`:TSInstall candela`, and open a `.cdl` file.

`setup()` takes `grammar_path` (build the grammar from a local candela
checkout instead of fetching it), `grammar_url` and `grammar_revision` (where
and what to fetch, tracking the default branch by default), `server_path`
(absolute path to `candela-lsp`), and `treesitter`/`lsp` toggles to skip
either half. With `server_path` unset the server is found the way the VS Code
extension finds it: `candela-lsp` on `$PATH`, where the toolchain installs it.

On the `nvim-treesitter` main branch the queries install with the parser; on
master, copy them yourself:

```sh
mkdir -p ~/.config/nvim/queries/candela
cp editors/tree-sitter/queries/*.scm ~/.config/nvim/queries/candela/
```

`nvim/lsp/candela_lsp.lua` is the server definition on its own, in the layout
`nvim-lspconfig` uses; drop it in `~/.config/nvim/lsp/` and enable it with
`vim.lsp.enable('candela_lsp')` to configure the grammar some other way.

## Helix

Append `helix/languages.toml` to `~/.config/helix/languages.toml`, copy the
queries into `~/.config/helix/runtime/queries/candela/`, then run
`hx --grammar fetch` and `hx --grammar build`. `hx --health candela` reports
what Helix found. `candela-lsp` has to be on `$PATH`, or named by an absolute
path in the `[language-server.candela-lsp]` section.

## What it parses

Every declaration form: `import` with and without an alias, `fn` with typed or
bare parameters and an optional return annotation, `struct`, `enum` with
payload types, `impl` blocks, and the `dylib` and `host` signature blocks
including the variadic `...` parameter. Statements: `let`, assignment and its
compound forms, `if`/`else if`/`else`, `while`, `for` over a collection or a
range, `loop`, `match`, `try`/`catch`, and bare blocks. Expressions carry
candela's precedence, so `2 ^ 3 ^ 2` groups to the right and `-a ^ 2` negates
before it raises; calls, method calls, field access, indexing, and slicing bind
tighter than every operator. Literals cover integers, floats, strings with
their escapes, `true`, `false`, `null`, lists, maps, struct literals, and
anonymous functions. The type syntax covers arrays, map types, unions, and
namespaced names.

candela has one comment form, the line comment. A `///` comment is an ordinary
comment; the standard library writes them ahead of a declaration to document
it, and the queries colour them as documentation.

Source that the compiler rejects can still parse here when its shape is valid,
so an unknown function name or a type mismatch looks fine to the grammar and is
reported by the language server.

## Working on the grammar

```sh
npx tree-sitter generate --abi 14
npx tree-sitter test
npx tree-sitter parse ../../libs/std/string.cdl
```

Commit the regenerated `src/` along with `grammar.js`. Corpus cases in
`test/corpus` are taken from the standard library, the examples, and the
documentation, so a construct that appears in real candela source has a test.
Update the expectations with `npx tree-sitter test --update` and read the diff
before committing it.

Consumers name the revision they build the grammar from: the `[[grammar]]`
section for Helix, `grammar_revision` for Neovim, and `[grammars.candela] rev`
in the Zed extension. All three follow the default branch, so a grammar change
reaches them on the next fetch.
