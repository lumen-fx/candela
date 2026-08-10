# set

A set of unique values.

```rust
import "std/set" as set;
```

A set is a map whose keys are the members, so it reuses the map's storage,
hashing, and garbage collection rather than adding a runtime type of its own.
That has two consequences you can rely on: a set value accepts the built-in map
methods, and `for x in s` iterates the members.

Create one with `set::new()`, add with `set::add`, and test with
`set::contains`.

The helpers are polymorphic through compile-time monomorphisation: one definition
specialises to the member type at the call site. The module is pure candela, so
it compiles into a `.cdlb` artifact and runs under `candela-vm` with no dynamic
library.

## new

```rust
set::new()
```

- Returns: a new empty set.

## add

```rust
set::add(s, x)
```

- `x`: the member to add.
- Returns: nothing. A member already present leaves the set unchanged.

## contains

```rust
set::contains(s, x)
```

- Returns: a bool, true when `x` is a member.

## len

```rust
set::len(s)
```

- Returns: the number of members, as an int.

## is_empty

```rust
set::is_empty(s)
```

- Returns: a bool, true when the set has no members.

## members

```rust
set::members(s)
```

- Returns: a list of the members. A set is a map underneath, so the order is
  unspecified and is not insertion order. Sort the result when you need a fixed
  order.

## union

```rust
set::union(a, b)
```

- Returns: a new set with every member of either `a` or `b`. Neither input
  changes.

## intersection

```rust
set::intersection(a, b)
```

- Returns: a new set with the members present in both `a` and `b`. Neither input
  changes.

## difference

```rust
set::difference(a, b)
```

- Returns: a new set with the members of `a` that are not in `b`. Neither input
  changes.

```rust
import "std/set" as set;

fn main() {
    let a = set::new();
    set::add(a, 1);
    set::add(a, 2);
    let b = set::new();
    set::add(b, 2);
    set::add(b, 3);

    print(set::members(set::union(a, b)));
    print(set::members(set::intersection(a, b)));
    print(set::members(set::difference(a, b)));
}
```
