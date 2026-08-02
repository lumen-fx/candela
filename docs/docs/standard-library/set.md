---
icon: lucide/shapes
---
# Set library

A set of unique values, built as a thin layer over a map. Import the library
with `import "std/set";` at the top-level.

*[at the top-level]: Outside of any function.

A set stores its members as the keys of a map, with every value set to `true`,
so it reuses the map's storage, hashing, and iteration rather than adding a
separate runtime type. Create one with `new`, add members with `add`, and test
membership with `contains`. A set has no dedicated literal syntax; build one
through this module.

The functions are polymorphic through Candela's compile-time
monomorphization: a single definition specializes to the member type at the
call site.

This module is written in Candela and uses no dynamic library, so a program
that imports it builds to a `.cdlb` artifact that runs under `candela-vm` with
the module bytecode inlined.

```rust
import "std/set" as set;

fn main() {
    let a = set::new();
    set::add(a, 1);
    set::add(a, 2);
    let b = set::new();
    set::add(b, 2);
    set::add(b, 3);

    print(set::len(set::union(a, b)));        // 3
    print(set::len(set::intersection(a, b))); // 1
}
```

### New
`new() -> {T: bool}`: a new empty set.
### Add
`add(s: {T: bool}, x: T)`: adds `x`. A member already present is unchanged.
### Contains
`contains(s: {T: bool}, x: T) -> bool`: true when `x` is a member.
### Len
`len(s: {T: bool}) -> int`: the number of members.
### Is empty
`is_empty(s: {T: bool}) -> bool`: true when the set has no members.
### Members
`members(s: {T: bool}) -> T[]`: a list of the members.
### Union
`union(a: {T: bool}, b: {T: bool}) -> {T: bool}`: a new set with every member
of either `a` or `b`.
### Intersection
`intersection(a: {T: bool}, b: {T: bool}) -> {T: bool}`: a new set with the
members present in both `a` and `b`.
### Difference
`difference(a: {T: bool}, b: {T: bool}) -> {T: bool}`: a new set with the
members of `a` that are not in `b`.
