---
icon: lucide/list
---
# List library

Helpers that build on Candela's array operators and built-in methods. Import the
library with `import std::list;` at the top-level.

*[at the top-level]: Outside of any function.

The functions are polymorphic through Candela's compile-time monomorphization: a
single definition specializes to whatever element type the call site uses. The
reductions (`sum`, `product`, `min`, `max`, `first`, `last`) read their seed or
result from an element, so they require a non-empty list.

This module is written in Candela and uses no dynamic library, so a program that
imports it builds to a `.cdlb` artifact that runs under `candela-vm` with the
module bytecode inlined.

The built-in array methods (`push`, `remove`, `sort`, `reverse`, `contains`,
`find`, `join`, `len`) remain available on any array; see
[Built-in functions](built-in-functions.md). This module adds helpers those
methods do not cover.

### First
`first(arr: T[]) -> T` -- the first element. Requires a non-empty list.
### Last
`last(arr: T[]) -> T` -- the last element. Requires a non-empty list.
### Is empty
`is_empty(arr: T[]) -> bool` -- true when `arr` has no elements.
### Sum
`sum(arr: T[]) -> T` -- the sum of the elements. Requires a non-empty list whose
element type supports `+`.
### Product
`product(arr: T[]) -> T` -- the product of the elements. Requires a non-empty
list whose element type supports `*`.
### Min
`min(arr: T[]) -> T` -- the smallest element. Requires a non-empty list.
### Max
`max(arr: T[]) -> T` -- the largest element. Requires a non-empty list.
### Index of
`index_of(arr: T[], value: T) -> int` -- the index of the first element equal to
`value`, or -1 if there is none.
### Count
`count(arr: T[], value: T) -> int` -- the number of elements equal to `value`.
### Unique
`unique(arr: T[]) -> T[]` -- a new list with duplicate elements removed,
preserving first-seen order.
### Chunk
`chunk(arr: T[], size: int) -> T[][]` -- `arr` broken into consecutive sub-lists
of at most `size` elements.
### Take
`take(arr: T[], n: int) -> T[]` -- the first `n` elements (all of them when `n`
exceeds the length).
### Drop
`drop(arr: T[], n: int) -> T[]` -- every element except the first `n` (an empty
list when `n` exceeds the length).
