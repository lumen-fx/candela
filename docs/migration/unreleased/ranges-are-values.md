# `range()` is removed; `a..b` is a range value

`a..b` and the new `a..=b` are values of the prelude type `Range`, which can be
stored, passed and looped over, with `len`, `contains`, `rev`, `step` and
`list` methods. The `range(n)` and `range(start, end)` built-ins, which
returned a list, are removed, and a call to one is reported with the range to
write instead. A `for` over a range written in place counts in a register as
it did.

This affects you if your program calls `range`.

Before:

```rust
for i in range(n) {
    print(i);
}
let squares = range(1, 6).map(fn(x) => x * x);
```

After:

```rust
for i in 0..n {
    print(i);
}
let squares = (1..6).list().map(fn(x) => x * x);
```
