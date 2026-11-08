# Rebuild your .cdlb artifacts: the artifact format is now version 18

The `.cdlb` format moved from version 17 to version 18. An attribute argument
that names a function now records that function's parameters and return type,
and an attribute can go on an enum.

The new `candela-vm`, and `load_program` in an embedding host, refuse a
version 17 artifact:

```text
candela-vm: unsupported .cdlb format version 17 (this runtime understands version 18)
```

Rebuild each artifact from its source with the new toolchain:

```sh
candela build game.cdl
```
