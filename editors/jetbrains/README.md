# Candela - JetBrains IDE plugin

Editor support for the [candela](https://candela.lumenfx.dev) language in
IntelliJ IDEA, CLion, PyCharm, WebStorm, and the other IntelliJ-based IDEs:
syntax highlighting for `.cdl` files, and everything else from `candela-lsp`.

Use it if you write candela in a JetBrains IDE. The extension in
`editors/vscode` covers VS Code.

## Quick start

1. Build the language server:

   ```sh
   cargo build -p candela-lsp
   ```

   Do not build it with `--release`. That profile aborts on panic, and the
   server turns a compiler panic into a diagnostic, so a release build stops at
   the first error you type.

2. Build the plugin:

   ```sh
   ./gradlew buildPlugin
   ```

   The installable zip lands in `build/distributions/`.

3. In the IDE, install [LSP4IJ](https://plugins.jetbrains.com/plugin/23257-lsp4ij)
   from the Marketplace, then install the zip with Settings | Plugins |
   gear icon | Install Plugin from Disk, and restart.

4. Put `candela-lsp` on your `PATH`, or set its path in Settings |
   Languages & Frameworks | Candela. With auto-discovery on, the plugin also
   finds a binary under `$CARGO_TARGET_DIR` or the project's `target/`
   directory, in the `debug` and `embed` profiles.

Open a `.cdl` file. The status of the server is in the LSP console
(View | Tool Windows | Language Servers).

## What you get

Syntax highlighting from the same TextMate grammar the VS Code extension uses,
so both editors color candela the same way and a grammar fix reaches both. A
TextMate grammar matches patterns rather than parsing, so highlighting follows
the shape of the text and not the compiler's view of it.

Everything else comes from `candela-lsp`, which runs the real compiler over
your buffer:

- Diagnostics, pushed as you type. candela reports one error per compile, so
  fix the first and the next appears.
- Hover on a struct, a function, or a built-in.
- Completion of keywords, built-ins, and the functions and structs in the
  compiled program, with `.` as a trigger character.
- Outline of the `fn` and `struct` declarations in the file.
- Go to definition, including into an imported file.

See [`lsp/README.md`](../../lsp/README.md) for what each of those covers and
where the server simplifies.

## Limitations

- LSP4IJ has to be installed first. It is the LSP client, and it is what makes
  the plugin work in Community-edition IDEs.
- The plugin does not ship `candela-lsp`. Build it from this repository.
- The server offers no formatting, rename, find usages, or signature help, so
  the plugin does not either.
- There are no `candela` run or build actions.
- The plugin is not on the JetBrains Marketplace. Install the zip from disk.
- Enabling or disabling it takes an IDE restart, because it registers the
  TextMate bundle at startup.

## Development

Requires JDK 21. `./gradlew buildPlugin` produces the zip,
`./gradlew verifyPlugin` runs the JetBrains Plugin Verifier, and
`./gradlew runIde` starts a sandbox IDE with the plugin loaded.

The plugin compiles against the oldest IDE it supports, set in
`gradle.properties`, so it cannot reach for an API that the floor lacks. There
is no upper bound on the IDE version.
