---
icon: lucide/table-properties
---
# Map library

Helpers built on Candela's map methods. Import the library with
`import "std/map";` at the top-level.

*[at the top-level]: Outside of any function.

A map holds key-value pairs. The core operations already exist as built-in
methods on any map value (`m.get(k)`, `m.insert(k, v)`, `m.len()`,
`m.contains(k)`, `m.keys()`, `m.values()`), and a map iterates its keys
(`for k in m { ... }`); see [Data types](../language-tour/data-types.md#map-k-v).
This module wraps those methods as free functions and adds a couple of
conveniences.

The functions are polymorphic through Candela's compile-time
monomorphization: a single definition specializes to whatever key and value
types the call site uses.

This module is written in Candela and uses no dynamic library, so a program
that imports it builds to a `.cdlb` artifact that runs under `candela-vm` with
the module bytecode inlined.

### Len
`len(m: {K: V}) -> int`: the number of entries.
### Is empty
`is_empty(m: {K: V}) -> bool`: true when the map has no entries.
### Contains
`contains(m: {K: V}, k: K) -> bool`: true when `k` is a key in the map.
### Get
`get(m: {K: V}, k: K) -> V`: the value stored under `k`. Raises when the key
is absent.
### Get or
`get_or(m: {K: V}, k: K, default: V) -> V`: the value stored under `k`, or
`default` when the key is absent.
```rust
import "std/map" as map;

fn main() {
    let m = {"a": 1, "b": 2};
    print(map::get_or(m, "a", 0));   // 1
    print(map::get_or(m, "z", 0));   // 0
}
```
### Insert
`insert(m: {K: V}, k: K, v: V)`: stores `v` under `k`, replacing any existing
value.
### Keys
`keys(m: {K: V}) -> K[]`: a list of the keys.
### Values
`values(m: {K: V}) -> V[]`: a list of the values.
