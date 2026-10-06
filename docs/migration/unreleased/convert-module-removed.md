# `std/convert` and its `to_*` methods are removed

The `to_int`, `to_float`, `to_bool` and `to_string` methods spelled the
conversions a second way. The conversions are the type names called as
functions: `int(x)`, `float(x)`, `bool(x)` and `string(x)`. The module is gone
from the prelude and from the library directory, so `import "std/convert";` no
longer resolves, and calling one of the methods is a compile error,
`no_such_method`, whose help names the conversion.

This affects you if a program calls one of the four methods or imports
`std/convert`.

Before:

```rust
fn main() {
    print("42".to_int() + 1);
    print(3.9.to_int());
    print(2.5.to_string() + "!");
}
```

After:

```rust
fn main() {
    print(int("42") + 1);
    print(int(3.9));
    print(string(2.5) + "!");
}
```
