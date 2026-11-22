# Rebuild your .cdlb artifacts: the artifact format is now version 19

The `.cdlb` format moved from version 17 to version 19. An attribute argument
that names a function now records that function's parameters and return type,
and an attribute can go on an enum, a method, a parameter, a constant, a field,
a variant and an `impl` block.

The new `candela-vm`, and `load_program` in an embedding host, refuse a
version 17 artifact:

```text
candela-vm: unsupported .cdlb format version 17 (this runtime understands version 19)
```

Rebuild each artifact from its source with the new toolchain:

```sh
candela build game.cdl
```
