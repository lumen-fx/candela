# candela.toml

The project manifest. Its presence makes a directory a project: `candela run`,
`candela check` and `candela build` take their entry point from it, and
`candela add`, `candela remove`, `candela fetch`, `candela update` and
`candela publish` work on the package it describes.

```toml
[package]
name = "geom"
version = "0.3.1"
description = "2D geometry"
entry = "src/main.cdl"

[dependencies]
shapes = "1.2"
```

## Where it is found

A command looks for `candela.toml` in the working directory and then in each
directory above it, stopping at the first one it finds. That directory is the
project root, and every relative path in the manifest is relative to it.

`candela.lock` sits beside it. It is written by the registry client, records
the exact version of every package resolved, and is meant to be committed.

## Unknown keys

A table or a key candela does not know is an error naming it, rather than
something quietly ignored. A manifest either says what candela reads or says
what is wrong with it.

## [package]

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `name` | string | yes | The package name. This is what another project writes in its `[dependencies]`, and what its imports resolve under. |
| `version` | string | yes | The package version. `candela publish` releases this version, and a workflow release checks it against the tag. |
| `description` | string | no | One line about the package, sent to the registry when it is published. |
| `entry` | string | no | The file `run`, `check` and `build` compile when no file is named, and the file another project reads when it imports this package by name. Defaults to `src/main.cdl`. |

The entry is the package's front door twice over: the verbs start there, and
`import "name"` in a project that depends on the package reads it. A library
keeps a `main` in it so `candela check` has something to compile; a `main` in
an imported module is ignored.

## [dependencies]

One entry per package, keyed by name. The value is a version requirement, in
either of two spellings:

```toml
[dependencies]
shapes = "1.2"
geom = { version = "0.3" }
```

The requirement is passed to the registry client as written; candela does not
interpret it. `candela add` writes the first spelling and edits the `version`
key of the second, leaving whatever else the table carries alone.

An empty table, or no table at all, means no dependencies, and no command
reaches for the registry client.

A dependency on a package published for the `lumen` platform is refused by
name. The two platforms ship different things, and a Lumen package holds
nothing a candela import can reach.

## Editing

`candela add` and `candela remove` rewrite the file in place and keep
everything that is not data: comments, key order, blank lines, and the table
form a dependency was written in.

## Package layout

Nothing is enforced beyond the manifest, but `candela new` writes, and the
reusable release workflow packs, this shape:

```
package/
  candela.toml
  candela.lock
  src/
    main.cdl
  dist/          # native libraries, when the package ships any
```

`src/` and `candela.toml` are what a release archive carries, with the package
root at the root of the archive. A package that ships native libraries carries
`dist/` too, built per target.

## Imports into a package

A library import whose first segment is the package name resolves against the
package's entry: the name alone reads the entry file, and a path reads from the
entry's directory. With the default entry:

| Import | Reads |
| --- | --- |
| `import "shapes";` | `<package root>/src/main.cdl` |
| `import "shapes/circle";` | `<package root>/src/circle.cdl` |
| `import "shapes/geo/arc";` | `<package root>/src/geo/arc.cdl` |

A manifest that sets `entry = "lib/shapes.cdl"` moves both: the name reads
`lib/shapes.cdl` and `shapes/circle` reads `lib/circle.cdl`.

The package root is also searched for native libraries, so a `dylib` import
with a plain name finds a library shipped at the root of the package. A library
under `dist/` is reached from `src/` with a path: `dylib "../dist/mylib"`. See
[C libraries](../integration/c-libraries.md).
