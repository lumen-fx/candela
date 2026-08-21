# list

Reductions, slicing, and higher-order methods on arrays.

The module loads automatically, so every method here works on any array with no
import line: `[1, 2, 3].sum()`, `xs.map(f)`. The methods are defined in an
`impl list` block; see [methods](../language/methods.md) for how `impl` blocks
on the built-in types resolve.

The helpers are polymorphic through compile-time monomorphisation: one definition
specialises to whatever element type the call site uses. The reductions `sum`,
`product`, `min`, and `max` read their seed from the first element, so they need
a non-empty list.

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library. It builds on the built-in array methods
(`len`, `push`, `contains`, `sort`, and the rest), which are listed in
[built-in functions](builtins.md).

## Element access

### first

```rust
arr.first()
```

- Returns: the element at index 0.
- Raises: `index_out_of_bounds` on an empty list.

### last

```rust
arr.last()
```

- Returns: the element at index `arr.len() - 1`.
- Raises: `index_out_of_bounds` on an empty list.

### is_empty

```rust
arr.is_empty()
```

- Returns: a bool, true when `arr` has no elements.

### index_of

```rust
arr.index_of(value)
```

- `value`: an element to look for.
- Returns: the index of the first element equal to `value`, or -1 when there is
  none.

### count

```rust
arr.count(value)
```

- `value`: an element to look for.
- Returns: the number of elements equal to `value`, as an int.

## Reductions

### sum

```rust
arr.sum()
```

- Returns: the elements combined left to right with `+`.
- Raises: `index_out_of_bounds` on an empty list.

The element type only has to support `+`, so a list of strings sums to their
concatenation.

### product

```rust
arr.product()
```

- Returns: the elements combined left to right with `*`.
- Raises: `index_out_of_bounds` on an empty list.

### min

```rust
arr.min()
```

- Returns: the smallest element, compared with `<`.
- Raises: `index_out_of_bounds` on an empty list.

### max

```rust
arr.max()
```

- Returns: the largest element, compared with `>`.
- Raises: `index_out_of_bounds` on an empty list.

## Slicing and reshaping

### take

```rust
arr.take(n)
```

- `n`: an int.
- Returns: a new list of the first `n` elements, or a copy of the whole list when
  `n` exceeds the length. An empty list gives an empty list.

### drop

```rust
arr.drop(n)
```

- `n`: an int.
- Returns: a new list of every element after the first `n`, and an empty list
  when `n` reaches or exceeds the length.

### chunk

```rust
arr.chunk(size)
```

- `size`: an int.
- Returns: a new list of consecutive sub-lists of at most `size` elements. The
  final chunk is shorter when the length is not a multiple of `size`. An empty
  list gives an empty result.

```rust
fn main() {
    print([1, 2, 3, 4, 5].chunk(2));
}
```

### unique

```rust
arr.unique()
```

- Returns: a new list with duplicate elements removed, keeping first-seen order.
  An empty list returns unchanged.

## Higher-order helpers

Each of these takes a function value. Pass a named function or an anonymous one;
see [functions](../language/functions.md).

### map

```rust
arr.map(f)
```

- `f`: takes one element, returns the mapped value.
- Returns: a new list of the results, in order.

### filter

```rust
arr.filter(f)
```

- `f`: takes one element, returns a bool.
- Returns: a new list of the elements for which `f` is true, in order.

### reduce

```rust
arr.reduce(init, f)
```

- `init`: the starting accumulator.
- `f`: takes the accumulator and an element, returns the new accumulator.
- Returns: the accumulator after folding left to right. `init` on an empty list.

```rust
fn main() {
    let xs = [1, 2, 3, 4];
    print(xs.reduce(0, fn(acc, x) { return acc + x; }));
}
```

### each

```rust
arr.each(f)
```

- `f`: takes one element; its result is discarded.
- Returns: nothing. Call it for the side effect.

### find

```rust
arr.find(f)
```

- `f`: takes one element, returns a bool.
- Returns: the first element for which `f` is true, or null when none match.

`arr.find(x)` with a value rather than a function is the built-in index search,
which returns an int index or -1. The argument type picks between the two.

### any

```rust
arr.any(f)
```

- `f`: takes one element, returns a bool.
- Returns: true when `f` holds for at least one element. False on an empty list.

### all

```rust
arr.all(f)
```

- `f`: takes one element, returns a bool.
- Returns: true when `f` holds for every element. True on an empty list.

### sort_by

```rust
arr.sort_by(less)
```

- `less`: takes two elements, returns true when the first comes before the
  second.
- Returns: a new sorted list; `arr` is left unchanged.

A stable insertion sort. Use it when you need a custom order or an untouched
input; the built-in `arr.sort()` sorts ints, floats, and strings ascending in
place.

```rust
fn main() {
    let xs = [3, 1, 2];
    print(xs.sort_by(fn(a, b) { return a > b; }));
    print(xs);
}
```
