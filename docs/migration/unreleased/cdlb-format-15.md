# Rebuild your .cdlb artifacts: the artifact format is now version 15

The `.cdlb` format moved from version 14 to version 15. Version 15 added the
instruction the `is` type test compiles to for structs, enums, unions and
collections, and the nested type codes that check what a list or a map holds.

This affects you if you ship or keep `.cdlb` files built by an earlier
release. The new `candela-vm`, and `load_program` in an embedding host, refuse
them:

```text
candela-vm: unsupported .cdlb format version 14 (this runtime understands version 15)
```

Rebuild each artifact from its source with the new toolchain:

```sh
candela build game.cdl
```

An embedding host that builds artifacts with `build_bytecode` needs no code
change; rebuild the artifacts it wrote before the upgrade.
