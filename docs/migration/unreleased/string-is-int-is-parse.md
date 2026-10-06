# `s.is_int()` and `s.is_float()` are replaced by `s.parse<T>()`

The two string methods answered whether the text held a number, and the number
was then read with a second call. `s.parse<T>()` does both: it answers
`Some(value)` when the text holds an `int`, a `float` or a `bool`, and `None`
otherwise. Calling `is_int` or `is_float` on a string is a compile error,
`no_such_method`, whose help names `parse`.

`"42".parse<float>()` is `Some(42.0)`, where `"42".is_float()` was `false`: a
float reads from integer text too.

This affects you if a program calls `is_int` or `is_float` on a string.

Before:

```rust
fn port(text: string) -> int {
    if text.is_int() {
        return int(text);
    }
    return 8080;
}

fn main() {
    print(port("80"), port("x"));
}
```

After:

```rust
fn port(text: string) -> int {
    if text.parse<int>() is Some(n) {
        return n;
    }
    return 8080;
}

fn main() {
    print(port("80"), port("x"));
}
```
