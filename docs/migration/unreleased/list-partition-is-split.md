# `list.partition(sep)` is renamed `list.split(sep)`

Cutting a list at each separator element is now `split`, the name a string
uses for the same operation; the behaviour is unchanged. Calling `partition` is
a compile error, `no_such_method`, whose help names `xs.split(separator)`.

This affects you if a program calls `partition` on a list.

Before:

```rust
fn main() {
    print([1, 0, 2, 0, 3].partition(0));
}
```

After:

```rust
fn main() {
    print([1, 0, 2, 0, 3].split(0));
}
```
