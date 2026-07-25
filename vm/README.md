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
marshalling, and the `.cdlb` artifact format. The full `candela` toolchain runs
on this same runtime, so a program behaves identically whether run from source or
from a `.cdlb`. Our goal is to keep the standalone binary under 1 MiB. See the
[workspace README](../README.md) for the `.cdlb` artifact format, including how
whole-program capture, dynamic libraries, and `host` blocks are handled.
