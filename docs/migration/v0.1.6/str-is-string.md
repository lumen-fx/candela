# `str(x)` is renamed `string(x)`

The conversion to text takes the name of the type it produces, like `int(x)`,
`float(x)` and `bool(x)`. Calling `str` is a compile error, `unknown_function`,
whose help names `string(x)`.

This affects you if a program calls `str`. A program that declared its own
function named `string` now shadows the conversion with it.

Before:

```rust
fn main() {
    print("count: " + str(3));
}
```

After:

```rust
fn main() {
    print("count: " + string(3));
}
```
