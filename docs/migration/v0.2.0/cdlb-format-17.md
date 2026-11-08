# Rebuild your .cdlb artifacts: the artifact format is now version 17

The `.cdlb` format moved from version 16 to version 17. It records the
attribute table: every attribute a `host` block declares that the program's
scripts wrote, with its arguments.

This affects you if you ship or keep `.cdlb` files built by an earlier
release. The new `candela-vm`, and `load_program` in an embedding host, refuse
them:

```text
candela-vm: unsupported .cdlb format version 16 (this runtime understands version 17)
```

Rebuild each artifact from its source with the new toolchain:

```sh
candela build game.cdl
```

An embedding host that builds artifacts with `build_bytecode` needs no code
change. One that fills in a `ProgramImage` field by field sets the new
`attributes` field, an empty list for a program that writes none.
