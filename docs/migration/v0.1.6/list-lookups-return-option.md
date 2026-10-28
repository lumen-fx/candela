# `first`, `last`, `min` and `max` on a list return an `Option`

`xs.first()`, `xs.last()`, `xs.min()` and `xs.max()` used to hand back the
element and raise `index_out_of_bounds` on an empty list. They now return
`Some` of the element, or `None` for an empty list, and never raise.

This affects you if a program uses one of them as the element itself.

Before:

```rust
fn main() {
    let xs = [3, 1, 2];
    print(xs.max() + xs.first());
}
```

After:

```rust
fn main() {
    let xs = [3, 1, 2];
    print(xs.max().unwrap_or(0) + xs.first().unwrap_or(0));
}
```

In a function that returns an `Option` itself, `xs.first()?` reads the element
and hands a `None` back to the caller.
