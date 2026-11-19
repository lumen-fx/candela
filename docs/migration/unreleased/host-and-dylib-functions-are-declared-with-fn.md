# host and dylib blocks declare their functions with fn and named parameters

A `host` or `dylib` block used to list C-style signatures, a return type, a name
and the parameter types: `float noise(int, float);`. Now each entry is declared
the way a candela function is, with named parameters, an optional `-> Type`,
and a `;` for the body: `fn noise(seed: int, scale: float) -> float;`. The
parameter names are what a call passes arguments by, and a parameter may
declare a default. A variadic host function is `fn log(...);`.

This affects every `host` and `dylib` block, in your own code and in a binding
you maintain. The compiler refuses the old form and shows the entry rewritten
with placeholder parameter names; give them real names.

Before:

```rust
dylib "mylib" {
    int read_block(int[], int);
    reset(int);
}

host "app" {
    any log(...);
}
```

After:

```rust
dylib "mylib" {
    fn read_block(buf: int[], len: int) -> int;
    fn reset(level: int);
}

host "app" {
    fn log(...) -> any;
}
```
