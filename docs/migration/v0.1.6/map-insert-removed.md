# Map entries are written with `m[k] = v`; `insert` is removed

`m.insert(k, v)` is gone. A map entry is written with brackets, `m[k] = v`,
which adds the entry or replaces its value, and `m[k] += 1` and the other
compound assignments work on an entry. Calling `insert` is a compile error,
`no_such_method`, whose help names `m[key] = value`.

This affects you if a program calls `insert` on a map.

Before:

```rust
fn main() {
    let counts = {};
    counts.insert("a", 1);
    counts.insert("a", counts.get("a") + 1);
    print(counts);
}
```

After:

```rust
fn main() {
    let counts = {};
    counts["a"] = 1;
    counts["a"] += 1;
    print(counts);
}
```
