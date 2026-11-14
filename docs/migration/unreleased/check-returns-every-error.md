# Engine::check returns every error, and a Diagnostic carries related places

`Engine::check` goes on past an error in a function body to the next
statement and the next function, so its error is now a list of every error
found, in the order found.

Before:

```rust
match engine.check(src, "game.cdl") {
    Ok(warnings) => show(&warnings),
    Err(error) => show(&[error]),
}
```

After:

```rust
match engine.check(src, "game.cdl") {
    Ok(warnings) => show(&warnings),
    Err(errors) => show(&errors),
}
```

`Diagnostic` has a new field, `related: Vec<Related>`: the other places an
error is about, each with a `filename`, a byte `span` and a `message`. An
argument that does not fit its parameter points at the parameter's
declaration this way. Code that builds a `Diagnostic` with a struct literal
adds `related: Vec::new()`.

The message of an argument that does not fit a declared parameter names the
parameter: `Parameter b of add takes int, but this argument is string`, where
it read `Function add expects this argument's type to be int, but this
expression's type is string`. A host that matched the old text matches the
`argument_type_mismatch` code instead.
