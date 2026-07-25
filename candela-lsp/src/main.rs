//! `candela-lsp`: a language server for candela (`.cdl`) built on top of
//! candela's own lexer/parser/type-checker (see `crate::analysis`).
//!
//! Speaks LSP over stdio, the same transport every editor's built-in LSP
//! client (including `vscode-languageclient`, wired up in
//! `editors/vscode`) expects by default.

mod analysis;
mod backend;
mod builtins;
mod line_index;

use backend::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
