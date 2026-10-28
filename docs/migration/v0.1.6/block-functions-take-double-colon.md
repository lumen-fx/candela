# A `host` or `dylib` block's functions are called with `::` only

A function a `host` or `dylib` block declares used to be reachable two ways,
`app::rows(id)` and `app.rows(id)`. The dot form is now a compile error,
`block_dot_call`, whose help names the `::` call. A block is a namespace, and
`::` is how every namespace is reached; `.` is for the fields and methods of a
value.

This affects you if a script calls a block's function with a dot. A variable
that shares the block's name is unaffected: a dot on it still calls the value's
own method.

Before:

```rust
host "app" {
    int rows(string);
}

fn count(id: string) -> int {
    return app.rows(id);
}
```

After:

```rust
host "app" {
    int rows(string);
}

fn count(id: string) -> int {
    return app::rows(id);
}
```
