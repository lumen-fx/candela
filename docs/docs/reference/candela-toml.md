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
| `entry` | string | no | The file `run`, `check` and `build` compile when no file is named. Defaults to `src/main.cdl`. |

A library with nothing to run needs no entry: nothing reads it until a verb is
used without a file, and that verb says the entry is missing.

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

A package root is where a library import resolves from when its first segment
is the package name:

| Import | Reads |
| --- | --- |
| `import "shapes";` | `<package root>/shapes.cdl` |
| `import "shapes/circle";` | `<package root>/circle.cdl` |
| `import "shapes/geo/arc";` | `<package root>/geo/arc.cdl` |

The package root is also searched for native libraries, so a `dylib` import in
a package finds a library shipped beside its sources. See
[C libraries](../integration/c-libraries.md).
