# candela for Zed

Zed extension for candela: `.cdl` files get syntax highlighting from the
candela tree-sitter grammar, and `candela-lsp` supplies diagnostics, hover,
completion, go-to-definition, and the document outline.

## Install

The extension is not in Zed's registry yet, so install it from this directory:
open the Extensions view, choose "Install Dev Extension", and pick
`editors/zed`. Zed clones the grammar, compiles it, and builds the extension
itself, so a Rust toolchain and a C compiler have to be available.

`candela-lsp` has to be on `$PATH`. The toolchain installs it there; to use a
server you built yourself, point Zed at it in settings:

```json
{
  "lsp": {
    "candela-lsp": {
      "binary": {
        "path": "/absolute/path/to/candela-lsp"
      }
    }
  }
}
```

Build that server with `cargo build -p candela-lsp`. The release profile
aborts on panic, which turns the first error in a buffer into a crashed
server, so use the default profile or the repository's `embed` profile.

## Grammar

The grammar lives at [`../tree-sitter`](../tree-sitter) in this repository, and
`extension.toml` names the revision it is built from. That revision follows the
default branch; set it to a commit to pin the parser instead.

Zed reads its queries from the extension rather than from the grammar
directory, and its theme keys differ from the Neovim and Helix vocabulary, so
`languages/candela/highlights.scm` is a separate file from the grammar's own.
Keep the two in step when the grammar changes.
