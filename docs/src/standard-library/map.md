# map

The map methods the standard library defines on top of the built-in ones. The
module is part of the prelude, so they need no import.

A map holds key-value pairs. Brackets read and write an entry: `m[k]` is the
value stored under `k` and raises `unknown_map_key` when there is none, and
`m[k] = v` adds the entry or replaces its value. The built-in methods
`m.len()`, `m.contains(k)`, `m.remove(k)`, `m.keys()` and `m.values()` are
listed in [built-in functions](builtins.md), and a map iterates its keys:

```rust
fn main() {
    let m = {"a": 1, "b": 2};
    m["c"] = 3;
    m["a"] += 10;
    for k in m {
        print(k + "=" + string(m[k]));
    }
}
```

This module adds the methods below in an `impl map` block (see
[methods](../language/methods.md)). Map literals are described in
[collections](../language/collections.md).

The methods are polymorphic through compile-time monomorphisation: one
definition specialises to whatever key and value types the call site uses. The
module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## get

```rust
m.get(k)
```

- `k`: a key of the map's key type.
- Returns: `Some` of the value stored under `k`, or `None` when the map holds no
  such key.
- Raises: nothing.

`get` is the lookup that may find nothing; `m[k]` is the one that expects the
key to be there. `m.get(k).unwrap_or(fallback)` reads with a fallback, and
`m.get(k)?` hands the `None` back to the caller; see [option](option.md).

```rust
fn main() {
    let counts = {"a": 1};
    print(counts.get("a"));
    print(counts.get("z").unwrap_or(0));
}
```

## is_empty

```rust
m.is_empty()
```

- Returns: a bool, true when the map has no entries.

A map keeps its entries in the order they went in, and every walk over one
follows that order. The rules for where an entry lands are in
[built-in functions](builtins.md).
