# Leaving a try with return, break or continue ends its catch

A `return`, `break` or `continue` that leaves a `try` now takes its `catch`
down with it. Before, the catch stayed in place after the code left the
`try`, so an error raised later, outside it, could still run that catch.

This affects a program that leaves a `try` early and relied on its `catch`
running for an error raised after that.

```rust
fn first_even(xs: int[]) -> int {
    for x in xs {
        try {
            if x % 2 == 0 {
                return x;
            }
        } catch e {
            print("never for an error after the return");
        }
    }
    return -1;
}
```

To catch an error raised after the early exit, put a `try` around the code
that raises it.
