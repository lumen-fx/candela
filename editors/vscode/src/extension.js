'use strict';

// Activates the candela-lsp language client. This extension has no build
// step (see README.md "Install / development"), so this file is plain
// CommonJS, `require`-ing the prebuilt `vscode-languageclient` package
// directly instead of importing a TypeScript source tree.

const { workspace } = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

/** @type {import('vscode-languageclient/node').LanguageClient | undefined} */
let client;

/**
 * Resolves the `candela-lsp` binary to launch.
 *
 * Defaults to `candela-lsp` on PATH (installed the same way the `candela`
 * CLI itself is), but honors the `candela.languageServerPath` setting so a
 * locally-built server (e.g. from `candela-lsp/`, built with `cargo build
 * -p candela-lsp` -- see that crate's README for why NOT `--release`) can
 * be pointed at during development.
 */
function resolveServerCommand() {
  const configured = workspace.getConfiguration('candela').get('languageServerPath');
  if (typeof configured === 'string' && configured.trim().length > 0) {
    return configured;
  }
  return process.platform === 'win32' ? 'candela-lsp.exe' : 'candela-lsp';
}

function activate(context) {
  const command = resolveServerCommand();

  /** @type {import('vscode-languageclient/node').ServerOptions} */
  const serverOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  /** @type {import('vscode-languageclient/node').LanguageClientOptions} */
  const clientOptions = {
    documentSelector: [{ scheme: 'file', language: 'candela' }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher('**/*.cdl'),
    },
  };

  client = new LanguageClient(
    'candela-lsp',
    'Candela Language Server',
    serverOptions,
    clientOptions,
  );

  // If `candela-lsp` isn't installed/on PATH, `client.start()` rejects;
  // surface that instead of leaving the extension silently inert. `client`
  // itself is stopped in `deactivate()`, not registered as a disposable
  // here, since `LanguageClient` does not implement `vscode.Disposable`.
  client.start().then(undefined, (err) => {
    const vscode = require('vscode');
    vscode.window.showErrorMessage(
      `Candela: failed to start candela-lsp ("${command}"). Install it or set ` +
        `"candela.languageServerPath" in your settings. (${err && err.message ? err.message : err})`,
    );
  });
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
