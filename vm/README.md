# candela-vm

The standalone runtime for [Candela](../README.md). It loads a pre-compiled
`.cdlb` bytecode artifact and runs it; it links no parser, compiler, or REPL.

```sh
candela build program.cdl     # (via `candela`) emit program.cdlb
candela-vm program.cdlb       # load and run the bytecode
```

The full `candela` toolchain runs on this same runtime, so a program behaves the
same whether run from source or from a `.cdlb`. The goal is to keep the
standalone binary under 1 MiB.

See the [workspace README](../README.md) for the `.cdlb` model, including how
whole-program capture, dynamic libraries, and `host` blocks are handled.
