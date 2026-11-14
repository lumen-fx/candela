'use strict';

// Activates the candela-lsp language client. This extension has no build
// step (see README.md "Install / development"), so this file is plain
// CommonJS, `require`-ing the prebuilt `vscode-languageclient` package
// directly instead of importing a TypeScript source tree.

const os = require('os');
const path = require('path');
const { workspace } = require('vscode');
const {
  DidChangeConfigurationNotification,
  LanguageClient,
  TransportKind,
} = require('vscode-languageclient/node');

/** @type {import('vscode-languageclient/node').LanguageClient | undefined} */
let client;

/**
 * Resolves the `candela-lsp` binary to launch.
 *
 * Defaults to `candela-lsp` on PATH (installed the same way the `candela`
 * CLI itself is), but honors the `candela.languageServerPath` setting so a
 * locally-built server (e.g. from `lsp/`, built with `cargo build -p
 * candela-lsp`; see that crate's README for why not `--release`) can be
 * pointed at during development.
 */
function resolveServerCommand() {
  const configured = workspace.getConfiguration('candela').get('languageServerPath');
  if (typeof configured === 'string' && configured.trim().length > 0) {
    return configured;
  }
  return process.platform === 'win32' ? 'candela-lsp.exe' : 'candela-lsp';
}

/**
 * Expands `~` and `${workspaceFolder}` in a configured path, and reads a
 * relative one from the first workspace folder.
 */
function expandPath(value) {
  const folder =
    workspace.workspaceFolders && workspace.workspaceFolders.length > 0
      ? workspace.workspaceFolders[0].uri.fsPath
      : undefined;
  let p = value.trim();
  if (folder) {
    p = p.split('${workspaceFolder}').join(folder);
  }
  if (p === '~' || p.startsWith('~/') || p.startsWith('~\\')) {
    p = path.join(os.homedir(), p.slice(1));
  }
  if (!path.isAbsolute(p) && folder) {
    p = path.join(folder, p);
  }
  return p;
}

/**
 * The settings the server reads: `candela.importRoots` and `candela.libPath`,
 * with their paths expanded, `candela.diagnosticsDelay`, and
 * `candela.packageChecks`, which an untrusted workspace turns off. Sent as
 * the initialization options and again whenever one changes.
 */
function serverSettings() {
  const config = workspace.getConfiguration('candela');
  const importRoots = {};
  const roots = config.get('importRoots') || {};
  for (const [name, dir] of Object.entries(roots)) {
    if (typeof dir === 'string' && dir.trim().length > 0) {
      importRoots[name] = expandPath(dir);
    }
  }
  const lib = config.get('libPath');
  const libPath = typeof lib === 'string' && lib.trim().length > 0 ? expandPath(lib) : '';
  const delay = config.get('diagnosticsDelay');
  const diagnosticsDelay = typeof delay === 'number' && delay >= 0 ? Math.round(delay) : 250;
  const packageChecks = config.get('packageChecks') !== false && workspace.isTrusted !== false;
  return { importRoots, libPath, diagnosticsDelay, packageChecks };
}

/** The settings `serverSettings` reads. */
const SETTINGS = [
  'candela.importRoots',
  'candela.libPath',
  'candela.diagnosticsDelay',
  'candela.packageChecks',
];

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
    // A change to a script or a settings file on disk checks the programs
    // that read it again, and runs the package checks.
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher('**/*.{cdl,toml}'),
    },
    initializationOptions: serverSettings(),
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
  const sendSettings = () => {
    if (client) {
      client.sendNotification(DidChangeConfigurationNotification.type, {
        settings: serverSettings(),
      });
    }
  };
  context.subscriptions.push(
    workspace.onDidChangeConfiguration((event) => {
      if (SETTINGS.some((name) => event.affectsConfiguration(name))) {
        sendSettings();
      }
    }),
    workspace.onDidGrantWorkspaceTrust(sendSettings),
  );

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
