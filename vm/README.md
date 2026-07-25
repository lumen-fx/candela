# candela-vm

The self-contained, VM-only runtime for [Candela](../README.md). It loads a
pre-compiled `.cdlb` bytecode artifact and runs it -- it links no parser,
compiler, or REPL.

```sh
candela build program.cdl     # (via `candela`) emit program.cdlb
candela-vm program.cdlb       # load + run the bytecode
```

This crate is the runtime core: the VM executor, the bytecode instruction set,
the NaN-boxed values and shared runtime types, the GC, the host/script value
marshalling, and the `.cdlb` artifact format. It depends on NOTHING from the
compiler (`candela`) crate -- the dependency direction is strictly `candela ->
candela-vm`, and it exposes both this `candela-vm` binary and a `candela_vm`
library that the full `candela` toolchain links, so the VM is never duplicated.
The release binary is under 1 MiB; see the [workspace README](../README.md) for
the exact measured sizes and the `.cdlb` artifact format.
