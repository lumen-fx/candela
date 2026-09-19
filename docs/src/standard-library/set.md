# set

A set of unique values.

```rust
import "std/set" as set;
```

`Set<T>` is a struct holding a map from member to a unit value, so it reuses the
map's storage, hashing, and garbage collection rather than adding a runtime type
of its own. Because it is a type of its own, the operations are methods on it:
`s.add(x)`, `s.contains(x)`, and the set algebra through `|`, `&`, `-`, and
`^^`.

Create one with `set::new<int>()`, naming the member type. Nothing in a bare
`set::new()` pins the type, so that hands back a `Set<any>`, which takes a
member of any type. `Set<int>` and `Set<any>` are separate types, like any other
two instantiations of a generic; see [generics](../language/generics.md).

A set is a struct, so it is not iterable and the built-in map methods do not
apply to it. Iterate the list `members` hands back, `for x in s.members()`, and
reach the rest through the methods below.

The type is generic through compile-time monomorphisation: one definition
specialises to the member type at the call site. The module is pure candela, so
it compiles into a `.cdlb` artifact and runs under `candela-vm` with no dynamic
library.

## new

```rust
set::new<int>()
```

- Returns: a new empty `Set<int>`. Written bare, `set::new()`, it returns a
  `Set<any>`.

## add

```rust
s.add(x)
```

- `x`: the member to add.
- Returns: nothing. A member already present leaves the set unchanged.

## remove

```rust
s.remove(x)
```

- `x`: the member to take out.
- Returns: nothing. A member the set does not hold leaves it unchanged.

## contains

```rust
s.contains(x)
```

- Returns: a bool, true when `x` is a member.

## len

```rust
s.len()
```

- Returns: the number of members, as an int.

## is_empty

```rust
s.is_empty()
```

- Returns: a bool, true when the set has no members.

## members

```rust
s.members()
```

- Returns: a list of the members, in the order they were added. The map
  underneath keeps insertion order: a member added twice keeps the place it
  first took, and one removed and added again goes to the end.

## union

```rust
a.union(b)
a | b
```

- Returns: a new set with every member of either `a` or `b`. Neither input
  changes.

## intersection

```rust
a.intersection(b)
a & b
```

- Returns: a new set with the members present in both `a` and `b`. Neither input
  changes.

## difference

```rust
a.difference(b)
a - b
```

- Returns: a new set with the members of `a` that are not in `b`. Neither input
  changes.

## symmetric_difference

```rust
a.symmetric_difference(b)
a ^^ b
```

- Returns: a new set with the members of exactly one of `a` and `b`: those in
  `a` but not `b` first, then those in `b` but not `a`. Neither input changes.

## Comparing two sets

`a == b` is set equality and needs no method of its own. Struct equality
compares field by field, and map equality ignores the order the entries went in,
so two sets holding the same members are equal whatever order they were built
in. `a != b` is the same answer flipped. The two sets have to be the same
`Set<T>`, so a `Set<int>` never equals a `Set<any>`.

## Operators

```rust
import "std/set" as set;

fn main() {
    let a = set::new<int>();
    a.add(1);
    a.add(2);
    let b = set::new<int>();
    b.add(2);
    b.add(3);

    print((a | b).members());
    print((a & b).members());
    print((a - b).members());
    print((a ^^ b).members());

    a |= b;
    print(a.members());
}
```

The four operators come from methods named by the symbol, the same way any type
defines an operator; see
[operators on your own types](../language/methods.md#operators-on-your-own-types).
A compound assignment applies the matching operator, so `a |= b` is
`a = a | b`.
