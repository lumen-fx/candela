'use strict';

// Activates the candela-lsp language client. This extension has no build
// step (see README.md "Install / development"), so this file is plain
// CommonJS, `require`-ing the prebuilt `vscode-languageclient` package
// directly instead of importing a TypeScript source tree.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { commands, window, workspace } = require('vscode');
const {
  DidChangeConfigurationNotification,
  LanguageClient,
  TransportKind,
} = require('vscode-languageclient/node');

/** @type {import('vscode-languageclient/node').LanguageClient | undefined} */
let client;

/** The server's file name on this platform. */
const SERVER = process.platform === 'win32' ? 'candela-lsp.exe' : 'candela-lsp';

/**
 * The directories the candela installers put the toolchain in, which hold
 * `candela-lsp` beside `candela`: `CANDELA_PREFIX` or `~/.candela` for
 * install.sh, and the per-user folder the Windows installer uses.
 */
function installDirs() {
  const dirs = [];
  if (process.env.CANDELA_PREFIX) {
    dirs.push(process.env.CANDELA_PREFIX);
  }
  dirs.push(path.join(os.homedir(), '.candela'));
  if (process.platform === 'win32' && process.env.LOCALAPPDATA) {
    dirs.push(path.join(process.env.LOCALAPPDATA, 'Programs', 'Candela'));
  }
  return dirs;
}

/** Whether `file` exists and can be run. */
function runnable(file) {
  try {
    fs.accessSync(file, fs.constants.X_OK);
    return true;
  } catch (_) {
    return false;
  }
}

/**
 * Resolves the `candela-lsp` binary to launch: the `candela.languageServerPath`
 * setting when it is set, else `candela-lsp` on PATH, else the one the
 * toolchain installer put beside `candela`. An editor started from a desktop
 * launcher often has a PATH without the installer's directory, which the
 * last step covers.
 */
function resolveServerCommand() {
  const configured = workspace.getConfiguration('candela').get('languageServerPath');
  if (typeof configured === 'string' && configured.trim().length > 0) {
    return configured;
  }
  if (whichCommand(SERVER) !== SERVER) {
    return SERVER;
  }
  for (const dir of installDirs()) {
    const candidate = path.join(dir, SERVER);
    if (runnable(candidate)) {
      return candidate;
    }
  }
  return SERVER;
}

/** This extension's version: the `candela-lsp` it needs reports the same. */
const VERSION = require('../package.json').version;

/** `a` compared with `b`, as dotted version numbers: negative when older. */
function compareVersions(a, b) {
  const pa = String(a).split(/[.-]/).map((n) => parseInt(n, 10) || 0);
  const pb = String(b).split(/[.-]/).map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] || 0) !== (pb[i] || 0)) {
      return (pa[i] || 0) - (pb[i] || 0);
    }
  }
  return 0;
}

/** The file a command runs: itself when it names a path, else its PATH entry. */
function whichCommand(command) {
  if (command.includes('/') || command.includes('\\')) {
    return command;
  }
  for (const dir of (process.env.PATH || '').split(path.delimiter)) {
    const candidate = path.join(dir, command);
    if (dir && runnable(candidate)) {
      return candidate;
    }
  }
  return command;
}

/**
 * Warns when the server that started is older than this extension: an old
 * server can report only the first error of a file and misread newer
 * syntax, so what the editor shows would be wrong without saying so.
 */
function checkServerVersion(command) {
  const info = client && client.initializeResult && client.initializeResult.serverInfo;
  const found = info && info.version ? info.version : '(no version reported)';
  const where = whichCommand(command);
  outputChannel.appendLine(`candela-lsp ${found} at ${where}`);
  if (info && info.version && compareVersions(info.version, VERSION) >= 0) {
    return;
  }
  window.showWarningMessage(
    `Candela: ${where} is candela-lsp ${found}, older than this extension (${VERSION}), ` +
      'so errors and completions may be missing or wrong. Update the candela toolchain, ' +
      'which installs the matching server, or point "candela.languageServerPath" at one.',
  );
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

/** Starts the language client, reading the settings afresh. */
function startClient() {
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
    // One channel for the life of the extension, so a restart keeps what
    // the previous server wrote.
    outputChannel,
  };

  client = new LanguageClient(
    'candela-lsp',
    'Candela Language Server',
    serverOptions,
    clientOptions,
  );

  // If `candela-lsp` is not installed, `client.start()` rejects; say so
  // instead of leaving the extension silently inert.
  return client.start().then(
    () => checkServerVersion(command),
    (err) => {
      window
        .showErrorMessage(
          `Candela: failed to start candela-lsp ("${command}"). Install the candela ` +
            'toolchain, which includes it, or set "candela.languageServerPath". ' +
            `(${err && err.message ? err.message : err})`,
          'Open Settings',
          'Show Output',
        )
        .then((choice) => {
          if (choice === 'Open Settings') {
            commands.executeCommand('workbench.action.openSettings', 'candela.languageServerPath');
          } else if (choice === 'Show Output') {
            outputChannel.show(true);
          }
        });
    },
  );
}

/** Stops the running client, if any, and starts a new one. */
async function restartClient() {
  if (client) {
    const old = client;
    client = undefined;
    try {
      await old.stop();
    } catch (_) {
      // A server that never started has nothing to stop.
    }
  }
  await startClient();
}

/** @type {import('vscode').LogOutputChannel | undefined} */
let outputChannel;

function activate(context) {
  // A log channel: the language client writes leveled messages to it.
  outputChannel = window.createOutputChannel('Candela Language Server', { log: true });

  // `client` itself is stopped in `deactivate()`, not registered as a
  // disposable here, since `LanguageClient` does not implement
  // `vscode.Disposable`.
  const sendSettings = () => {
    if (client) {
      client.sendNotification(DidChangeConfigurationNotification.type, {
        settings: serverSettings(),
      });
    }
  };
  context.subscriptions.push(
    outputChannel,
    commands.registerCommand('candela.restartServer', restartClient),
    commands.registerCommand('candela.showOutput', () => outputChannel.show(true)),
    workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('candela.languageServerPath')) {
        restartClient();
      } else if (SETTINGS.some((name) => event.affectsConfiguration(name))) {
        sendSettings();
      }
    }),
    workspace.onDidGrantWorkspaceTrust(sendSettings),
  );

  startClient();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
