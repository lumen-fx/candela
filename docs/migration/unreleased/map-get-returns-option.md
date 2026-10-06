# `m.get(k)` returns an `Option` instead of the value

`m.get(k)` used to hand back the value stored under `k` and raise
`unknown_map_key` when the key was absent. It now returns `Some` of the value,
or `None` when the map holds no such key, and never raises. Reading a value
that has to be there is `m[k]`, which raises `unknown_map_key` the way `get`
did.

This affects you if a program calls `get` on a map and uses the result as the
value.

Before:

```rust
fn main() {
    let ages = {"ada": 36};
    print(ages.get("ada") + 1);
}
```

After, when the key has to be there:

```rust
fn main() {
    let ages = {"ada": 36};
    print(ages["ada"] + 1);
}
```

Or, when it may be absent:

```rust
fn main() {
    let ages = {"ada": 36};
    print(ages.get("alan").unwrap_or(0) + 1);
}
```
