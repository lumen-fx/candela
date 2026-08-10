# Summary

What this changes, and why.

# Verification

How you checked it: the commands you ran, plus any program you ran through both
`candela` and `candela-vm` to confirm they agree.

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] `cargo test` and `cargo test --features embed`
- [ ] `cargo build --target wasm32-unknown-unknown`
- [ ] `cargo test -p candela-vm` / `cargo test -p candela-lsp`, if you touched
      `vm/` or `lsp/`; a plain `cargo test` at the root builds only the
      `candela` package
- [ ] docs under `docs/` updated, if this changes the language or the CLI
