# Rebuild your .cdlb artifacts: the artifact format is now version 14

The `.cdlb` format moved from version 12 to version 14. Version 13 added
instructions, which renumbers the ones after them, and version 14 added the
machine code sections and the function table a release build now writes.

This affects you if you ship or keep `.cdlb` files built by candela 0.1.4 or
earlier. The new `candela-vm`, and `load_program` in an embedding host, refuse
them:

```text
candela-vm: unsupported .cdlb format version 12 (this runtime understands version 14)
```

Rebuild each artifact from its source with the new toolchain:

```sh
candela build game.cdl
```

An embedding host that builds artifacts with `build_bytecode` needs no code
change; rebuild the artifacts it wrote before the upgrade.
