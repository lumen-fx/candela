# `m.get_or(k, fallback)` is removed

The `get_or` method of `std/map` is gone, now that `m.get(k)` returns an
`Option`. Calling it is a compile error, `no_such_method`, whose help names the
replacement, `m.get(k).unwrap_or(fallback)`.

This affects you if a program calls `get_or` on a map. A program that imported
`std/map` only for `get_or` can drop the import too: the map methods are part of
the prelude.

Before:

```rust
import "std/map";

fn main() {
    let counts = {"a": 1};
    print(counts.get_or("z", 0));
}
```

After:

```rust
fn main() {
    let counts = {"a": 1};
    print(counts.get("z").unwrap_or(0));
}
```
