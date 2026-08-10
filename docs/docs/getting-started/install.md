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
`curl` or `wget`. The verified archive unpacks into `/usr/local/lib/candela`
on Linux or `/Library/Candela` on macOS. It then links `candela` and `candela-vm` into
`/usr/local/bin` so both are on your path. Where it cannot write to those
locations directly it uses `sudo`.

The standard library ships as `.cdl` sources in a `libs` directory beside the
binary, and `import "std/..."` resolves relative to the binary's own location.
Keep the two together; moving the binary on its own breaks those imports.

Check the install:

```sh
candela --version
```

### Installing a specific release

Pass `--version` with a release tag. A leading `v` is optional, so both forms
work:

```sh
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --version 0.0.3
curl -fsSL https://candela.lumenfx.dev/install.sh | sh -s -- --version=v0.0.3
```

Run the script with `--help` to see its options.

Releases published before `sha256sums.txt` existed cannot be verified, so
pinning to one fails with a message pointing at the releases page.

### The receipt

The installer writes a file called `receipt` next to the binary, recording the
version it installed and, when you asked for a specific release, that the
install is pinned. `candela` reads the receipt to decide whether to look for
newer releases:

- No receipt means nothing installed this binary, so it was built from source
  and is left alone.
- A pinned receipt means you chose this release, so newer ones are not
  announced.
- Installing again without `--version` rewrites the receipt without the pin,
  which lifts it.

## Windows

Windows installs from a package rather than the script. Download and run
<https://github.com/lumen-fx/candela/releases/latest/download/candela-x86_64-windows.msi>.

The package installs per user, under `%LOCALAPPDATA%\Programs\Candela`, so it
never asks for administrator rights. It adds that directory to your `PATH` and
ships `candela.exe`, `candela-vm.exe` and the standard library. Open a new
terminal afterwards so the updated `PATH` takes effect.

A portable `candela-x86_64-windows.zip` is published alongside the package for
anyone who would rather unpack the toolchain by hand. Extract it somewhere and
keep the `libs` directory beside the executables.

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

## Building from source

To build the toolchain yourself, see
[Building candela](../contributing/building.md). A binary you build is never
announced as out of date, because it has no receipt.

## Next

Write your first program in [Hello, world](hello-world.md).
