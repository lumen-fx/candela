# Install

candela installs as two programs: `candela`, the compiler that also runs source
files and hosts the REPL, and `candela-vm`, the runtime that runs compiled
artifacts. Both land together, along with the standard library.

## Linux and macOS

Run the install script:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh
```

It works out your operating system and processor, downloads the matching
release archive, and checks it against the `sha256sums.txt` file published
with the release before unpacking; on a mismatch, or for a release without
that file, it installs nothing. It needs `sha256sum` or `shasum` alongside
`curl` or `wget`.

Everything lands under `~/.candela`, so the install needs no administrator
rights and never runs `sudo`. Once the archive is unpacked the script offers to
add that directory to your `PATH` by appending a line to your shell's rc file,
and leaves your shell configuration alone unless you say yes. Open a new shell
afterwards, or run the line it prints.

The standard library ships as `.cdl` sources in a `libs` directory beside the
binary, and `import "std/..."` resolves relative to the binary's own location.
Keep the two together; moving the binary on its own breaks those imports.

Check the install:

```sh
candela --version
```

### Options

| Option | What it does |
| --- | --- |
| `--prefix DIR` | Install root. Default `~/.candela`, also read from `CANDELA_PREFIX`. |
| `--version VERSION` | Install a pinned release instead of the current one. |
| `--no-confirm` | Run without prompting. |
| `--no-modify-path` | Never write a `PATH` line to a shell rc file. |
| `--force` | Reinstall even when that release is already installed. |
| `--uninstall` | Remove every file the installer put under the prefix. |
| `-h`, `--help` | Show the options. |

Options go after `--` when the script is piped into a shell:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --prefix ~/tools/candela
```

### Installing a specific release

Pass `--version` with a release tag. A leading `v` is optional, so both forms
work:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --version 0.0.3
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --version=v0.0.3
```

Releases published before `sha256sums.txt` existed cannot be verified, so
pinning to one fails with a message pointing at the releases page. Release
assets are named `candela-<os>-<arch>.tar.gz`; releases published before that
naming keep the names they went out with, and pinning to one fails with the
list of names that release does carry.

### Uninstalling

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --uninstall
```

It removes the files recorded in the receipt and leaves anything else under the
prefix alone. A `PATH` line the installer added stays in your shell rc file for
you to delete.

### The receipt

The installer writes a file called `receipt` next to the binary, recording the
version it installed, every file it wrote, and, when you asked for a specific
release, that the install is pinned. `candela` reads the receipt to decide
whether to look for newer releases:

- No receipt means nothing installed this binary, so it was built from source
  and is left alone.
- A pinned receipt means you chose this release, so newer ones are not
  announced.
- Installing again without `--version` rewrites the receipt without the pin,
  which lifts it.

## Windows

Windows installs from a package rather than the script. Download and run
<https://github.com/lumen-fx/candela/releases/latest/download/candela-windows-x86_64.msi>.

The package installs per user, under `%LOCALAPPDATA%\Programs\Candela`, so it
never asks for administrator rights. It adds that directory to your `PATH` and
ships `candela.exe`, `candela-vm.exe` and the standard library. Open a new
terminal afterwards so the updated `PATH` takes effect.

A portable `candela-windows-x86_64.zip` is published alongside the package for
anyone who would rather unpack the toolchain by hand. Extract it somewhere and
keep the `libs` directory beside the executables.

## Nightly builds

A build of `main` goes out every night at
<https://github.com/lumen-fx/candela/releases/tag/nightly>, for trying a change
before there is a release carrying it. It is a prerelease, so nothing that
looks up the newest release finds it: the install script with no arguments,
candela's update check, and the Windows link above all stay on the newest real
release.

Ask for the tag to install one:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --version nightly --force
```

That pins the install, so candela never offers you a release as an update.
`--force` is what lets a later run replace yesterday's build: the receipt
records the version as `nightly` either way, so without it the installer finds
nothing to do. Install again without `--version` to go back to releases.

One tag holds the newest build, which is why the address never changes and last
night's build is gone once tonight's is up. A nightly reports whatever version
`main` carries, which is usually a number no release was ever cut for.

The assets are the ones a release publishes, apart from the Windows installer.
On Windows, take `candela-windows-x86_64.zip` and unpack it. The installer
stamps the version from the manifest and Windows compares packages by that
number, so a nightly installer and a release installer of the same number would
replace one another.

## Staying up to date

`candela --help` and the REPL check at most once a day whether a newer release
exists and print a line to standard error when there is one. Running a program
never checks, so a script's output and exit status are never affected.

On Windows, `candela --help` goes further and asks whether to install the new
release. Answer `y` and the package downloads and installs once the command
exits; open a new terminal when it finishes. The REPL only prints the notice,
because it is already reading from your keyboard.

The check is skipped when standard error is not a terminal, when `CI` is set,
when the install is pinned, and when there is no receipt. To silence it
everywhere else, set `CANDELA_NO_UPDATE_CHECK`:

```sh
export CANDELA_NO_UPDATE_CHECK=1
```

## Editor support

The toolchain installs `candela-lsp`, a language server that runs candela's own
parser and type checker, so an editor reports the same errors the compiler
does. On top of diagnostics it gives hover, completion, go-to-definition, and a
document outline. Every editor below finds it as `candela-lsp` on your path.

Highlighting comes from one of two grammars. VS Code uses a TextMate grammar;
Neovim, Helix, and Zed use the tree-sitter grammar, which also drives
structural selection and bracket matching. Both live in the repository, under
`editors/`.

- **VS Code.** Install the extension from `editors/vscode`. Set
  `candela.languageServerPath` to use a server that is not on your path.
- **Neovim.** Copy `editors/tree-sitter/nvim/candela.lua` into your config,
  call `require('candela').setup()`, and install the parser with
  `:TSInstall candela`.
- **Helix.** Append `editors/tree-sitter/helix/languages.toml` to your
  `languages.toml`, copy the queries into the Helix runtime directory, then
  run `hx --grammar fetch` and `hx --grammar build`.
- **Zed.** Install `editors/zed` from the Extensions view with "Install Dev
  Extension".

Each directory has a README with the full setup, including how to point an
editor at a server you built yourself.

## In a browser

Every release also carries `candela-web.tar.gz`: candela compiled to
WebAssembly, as the pair of files `wasm-bindgen` writes.

```
candela.js          loads the runtime
candela_bg.wasm     the compiler and the virtual machine
```

Load the module, hand `run` a whole program, and read what the program printed
with `get_output`. A compile or run error arrives as a thrown value, and the
report itself goes to the output, so read the output after catching.

```js
import init, { run, get_output } from "./candela.js";

await init({ module_or_path: "./candela_bg.wasm" });
try {
  run('fn main() { print("hello"); }');
} catch {
  // the report is in the output read below
}
console.log(get_output());
```

This build has no file system, so a program it runs cannot `import` and the
standard library is out of reach. The language itself works, including the
[built-ins](../standard-library/builtins.md).

The prompt on [candela.lumenfx.dev](https://candela.lumenfx.dev/) is this
asset.

## Building from source

To build the toolchain yourself, see
[Building candela](../contributing/building.md). A binary you build is never
announced as out of date, because it has no receipt.

## Next

Write your first program in [Hello, world](hello-world.md).
