# `download::Event::Progress` gives the total as an `int?`

`T?` is now the type `Option<T>` wherever a type is written, and the packages
answer an absent value with `None` rather than a sentinel. The second value of
the `download` package's `Progress` event, the size the server declared, was
`-1` when the server declared none; it is now an `int?`.

This affects you if a program reads that total.

Before:

```rust
Progress(received, total) => {
    if total >= 0 {
        print(string(received) + " of " + string(total));
    }
}
```

After:

```rust
Progress(received, Some(total)) => print(string(received) + " of " + string(total)),
Progress(received, None) => print(string(received)),
```
