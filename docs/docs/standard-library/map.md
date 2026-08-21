# map

Extra map methods on top of the built-in ones.

```rust
import "std/map";
```

A map holds key-value pairs. The core operations are built-in methods on a map
value (`m.len()`, `m.get(k)`, `m.insert(k, v)`, `m.contains(k)`, `m.keys()`,
`m.values()`; listed in [built-in functions](builtins.md)), and a map iterates
its keys:

```rust
fn main() {
    let m = {"a": 1, "b": 2};
    for k in m {
        print(k + "=" + str(m.get(k)));
    }
}
```

This module adds conveniences in an `impl map` block (see
[methods](../language/methods.md)); the import brings them in. Map literals are
described in [collections](../language/collections.md).

The methods are polymorphic through compile-time monomorphisation: one
definition specialises to whatever key and value types the call site uses. The
module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## is_empty

```rust
m.is_empty()
```

- Returns: a bool, true when the map has no entries.

## get_or

```rust
m.get_or(k, default)
```

- `k`: a key of the map's key type.
- `default`: the value to return when the key is absent; the same type as the
  map's values.
- Returns: the value stored under `k`, or `default`.
- Raises: nothing.

```rust
import "std/map";

fn main() {
    let counts = {"a": 1};
    print(counts.get_or("a", 0));
    print(counts.get_or("z", 0));
}
```

A map is a hash map, so the order of `keys`, `values`, and `for k in m` is
unspecified and is not insertion order. `keys` and `values` walk the map the same
way, so their results line up entry for entry. Sort the result when you need a
fixed order.
