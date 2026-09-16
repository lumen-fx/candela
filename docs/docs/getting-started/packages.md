# Projects and packages

A single file is enough for a long time. When a program grows past it, or when
you want code somebody else wrote, turn it into a project: a directory with a
`candela.toml` in it that names the package, says which file to run, and lists
what it depends on.

## Start a project

```sh
candela new hello
cd hello
candela run
```

```
Hello, world!
```

`candela new` writes two files:

```
hello/
  candela.toml
  src/
    main.cdl
```

`candela.toml` is the manifest. `src/main.cdl` is the entry point, the file
`candela run` compiles and runs when you do not name one.

```toml
[package]
name = "hello"
version = "0.1.0"
description = ""

[dependencies]
```

Every key is listed in the [manifest reference](../reference/candela-toml.md).

## Run, check, build

Inside a project, three verbs take the entry point from the manifest:

```sh
candela run      # compile and run
candela check    # compile and stop, reporting any error
candela build    # compile to a .cdlb artifact
```

Each also takes a file, so `candela run tools/report.cdl` runs that file in the
project instead. Arguments after either reach the program through `argv()`:

```sh
candela run Ada
candela run src/main.cdl Ada
```

They work from anywhere inside the project. candela looks for `candela.toml` in
the current directory and then in each directory above it, so a command typed
in `src/` finds the project it belongs to.

A library needs no `main`. `run` and `build` want one, because both start the
program there, and `check` compiles a file that has none, so a package whose
entry only declares functions for other projects to import checks cleanly.

## Add a dependency

```sh
candela add shapes
```

That asks the registry for the newest `shapes`, downloads it, records it in the
manifest, and writes the lock file:

```toml
[dependencies]
shapes = "^1.2"
```

The requirement is the version that was resolved, at its major and minor, so a
later fetch takes patch releases and nothing wider. Ask for a different one by
naming it:

```sh
candela add shapes@^2
candela add shapes@1.2.3
```

`candela remove shapes` takes it out again.

## Import from a package

A package is another place a library import reads from. Its own name reaches
its entry point, the file its manifest names (`src/main.cdl` unless it says
otherwise), and a path inside it reaches the files beside that entry:

```rust
import "shapes" as shapes;        // the package's src/main.cdl
import "shapes/circle" as circle; // src/circle.cdl inside the package

fn main() {
    print(shapes::area(3, 4));
}
```

Nothing else about importing changes: the same one form, the same `as`, the
same bare-import merge. See [modules](../language/modules.md).

## The lock file

`candela.lock` sits beside the manifest and records the exact version of every
package the project resolved to, direct and indirect. Commit it. It is what
makes two machines, and your machine tomorrow, compile the same code.

```sh
candela fetch            # resolve and download what the manifest lists
candela fetch --locked   # fail rather than change candela.lock
candela update           # move to newer versions the manifest still allows
candela update shapes    # move just that one
```

`--locked` is the one to use in CI: it turns a lock file that disagrees with
the manifest into an error instead of a quiet rewrite.

Add `--offline` to `run`, `check`, `build` or `fetch` to resolve from packages
already downloaded and never reach the network.

## The registry client

Resolution is done by `lpm`, a separate program that talks to the registry.
candela runs it and reads the answer; it does not resolve versions itself.

The toolchain installer puts `lpm` at `~/.local/bin/lpm`, and candela downloads
it there itself the first time a project needs it and finds none. You do not
have to run it by hand. To point candela at one of your own, set `LPM_BIN`.

## Publish a package

A package is a project with something worth importing in it. Give it a
description, and put the functions you want reachable in the entry point: that
file is what `import "name"` reads, and the files beside it are what
`import "name/file"` reads.

The archive a release serves has to live somewhere. The reusable workflow builds
it, uploads it to a GitHub release, and tells the registry where it is, so
pushing a tag is the whole of publishing:

```yaml
# .github/workflows/release.yml
name: release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  release:
    uses: lumen-fx/candela/.github/workflows/build-package.yml@main
    with:
      candela-version: latest
    secrets:
      lpm-token: ${{ secrets.LPM_TOKEN }}
```

The workflow installs candela, runs `candela check` on the package, packs
`candela.toml` and `src/` into an archive, publishes the release, and calls the
registry with the manifest's version and dependencies. The `contents: write`
grant is what lets it create the release; a workflow that only wants the check
and the pack, on a pull request, grants `contents: read` and stops there.

A package that ships a native library builds one archive per desktop target.
Name the command that builds them, leaving the results in `dist/`:

```yaml
    with:
      candela-version: latest
      native-build: ./build-native.sh
```

To publish an archive you uploaded yourself, name it:

```sh
candela publish --url https://example.com/shapes-1.2.3.tar.gz
```

A candela project cannot depend on a Lumen package. They are different
platforms, and a dependency on one is refused by name.

## Next

The [manifest reference](../reference/candela-toml.md) lists every key, and the
[CLI reference](../reference/cli.md) lists every verb and flag.
