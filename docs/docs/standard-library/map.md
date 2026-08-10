# map

Free-function spellings of the map operations, plus a lookup with a default.

```rust
import "std/map" as map;
```

A map holds key-value pairs. The core operations are built-in methods on a map
value, and a map iterates its keys:

```rust
fn main() {
    let m = {"a": 1, "b": 2};
    for k in m {
        print(k + "=" + str(m.get(k)));
    }
}
```

This module wraps those methods so they read as calls, and adds `get_or`. Use
whichever spelling suits the surrounding code; they compile to the same work. The
built-in methods are listed in [built-in functions](builtins.md), and map
literals in [collections](../language/collections.md).

The helpers are polymorphic through compile-time monomorphisation: one definition
specialises to whatever key and value types the call site uses. The module is
pure candela, so it compiles into a `.cdlb` artifact and runs under `candela-vm`
with no dynamic library.

## len

```rust
map::len(m)
```

- Returns: the number of entries, as an int.

## is_empty

```rust
map::is_empty(m)
```

- Returns: a bool, true when the map has no entries.

## contains

```rust
map::contains(m, k)
```

- `k`: a key of the map's key type.
- Returns: a bool, true when `k` is a key in the map.

## get

```rust
map::get(m, k)
```

- `k`: a key of the map's key type.
- Returns: the value stored under `k`.
- Raises: `unknown_map_key`, naming the key, when it is absent.

## get_or

```rust
map::get_or(m, k, default)
```

- `k`: a key of the map's key type.
- `default`: the value to return when the key is absent; the same type as the
  map's values.
- Returns: the value stored under `k`, or `default`.
- Raises: nothing.

```rust
import "std/map" as map;

fn main() {
    let counts = {"a": 1};
    print(map::get_or(counts, "a", 0));
    print(map::get_or(counts, "z", 0));
}
```

## insert

```rust
map::insert(m, k, v)
```

- `k`: the key to store under.
- `v`: the value to store.
- Returns: nothing. Replaces any value already stored under `k`.

An empty map literal `{}` takes its key and value types from the first insert.

## keys

```rust
map::keys(m)
```

- Returns: a list of the keys.

## values

```rust
map::values(m)
```

- Returns: a list of the values.

A map is a hash map, so the order of `keys`, `values`, and `for k in m` is
unspecified and is not insertion order. `keys` and `values` walk the map the same
way, so their results line up entry for entry. Sort the result when you need a
fixed order.
