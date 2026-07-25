# candela-vm

A lean, VM-only runtime for [Candela](../README.md). It loads a pre-compiled
`.cdlb` bytecode artifact and runs it -- it links no parser, compiler, or REPL.

```sh
candela build program.cdl     # (via `candela`) emit program.cdlb
candela-vm program.cdlb       # load + run the bytecode
```

`candela-vm` depends on the `candela` crate with its default `compiler` feature
turned off (`default-features = false`), so the build contains only the runtime
core (VM + instruction set + values + GC + value marshalling + bytecode loader).
The release binary is under 1 MiB; see the [workspace README](../README.md) for
the exact measured sizes and the `.cdlb` artifact format.
