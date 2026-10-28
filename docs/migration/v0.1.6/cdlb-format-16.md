# Rebuild your .cdlb artifacts again: the artifact format is now version 16

The `.cdlb` format moved to version 16. It records the entry the runtime runs
event handlers through for a program that imports `std/event`, and the
instructions `std/event` and the new `std/fs` functions compile to.

This affects you if you ship or keep `.cdlb` files built by an earlier
toolchain, including one that already wrote version 15. The new `candela-vm`,
and `load_program` in an embedding host, refuse them:

```text
candela-vm: unsupported .cdlb format version 15 (this runtime understands version 16)
```

Rebuild each artifact from its source with the new toolchain:

```sh
candela build game.cdl
```

An embedding host that builds artifacts with `build_bytecode` needs no code
change. One that fills in a `ProgramImage` field by field sets the new
`event_entry` field, `None` for a program with no event handlers.
