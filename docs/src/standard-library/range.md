# range

A run of whole numbers: `a..b` holds `a` up to but not including `b`, and
`a..=b` holds `b` too.

A range is a value of the prelude type `Range`, so it needs no import. Store
it, pass it, return it, and loop over it with `for`. A `for` over a range
written in place, `for i in 0..n`, counts in a register and builds no range.

```rust
fn main() {
    let r = 2..5;
    print(r.len(), r.contains(4));
    for i in r.rev() {
        print(i);
    }
}
```

`..` binds looser than every operator, so `0..n + 1` runs to `n + 1`. Between
the brackets of an index, a range slices: `xs[1..3]`, `xs[1..=3]`.

`Range` is a struct with the fields `start`, `end` and `step`: the numbers from
`start`, moving by `step`, up to but not including `end`. A negative `step`
counts down. The module is pure candela, so it compiles into a `.cdlb` artifact
and runs under `candela-vm` with no dynamic library.

## len

```rust
r.len()
```

- Returns: how many numbers `r` holds, an `int`. `(5..0).len()` is 0.

## contains

```rust
r.contains(x)
```

- Returns: whether `r` holds the `int` `x`, a `bool`.

## rev

```rust
r.rev()
```

- Returns: a `Range` of the same numbers, last first. `(0..3).rev()` holds 2,
  1 and 0.

## step

```rust
r.step(n)
```

- Returns: a `Range` of every `n`th number of `r`, starting from its first.
  `(0..10).step(3)` holds 0, 3, 6 and 9.
- Raises: `range_step` when `n` is less than 1.

## list

```rust
r.list()
```

- Returns: the numbers of `r` in an `int[]`, first to last. `(1..4).list()` is
  `[1,2,3]`.
