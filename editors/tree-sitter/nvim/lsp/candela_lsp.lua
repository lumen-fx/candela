---@brief
---
--- https://github.com/lumen-fx/candela
---
--- Language server for candela (`.cdl`). It calls candela's own lexer,
--- parser, and type checker, so the editor and the compiler agree on what
--- counts as valid.
---
--- The toolchain installs it as `candela-lsp`. To build it from a checkout,
--- run `cargo build -p candela-lsp`; the release profile aborts on panic,
--- which turns the first error in a buffer into a crashed server, so use the
--- default profile or the repository's `embed` profile.

---@type vim.lsp.Config
return {
  cmd = { 'candela-lsp' },
  filetypes = { 'candela' },
  root_markers = { '.git' },
}
