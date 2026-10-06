# `throw` is a statement: `throw "kind";`

`throw` used to be written as a call, `throw("kind")`. It is now a statement
and a keyword, written like `return`: `throw "kind";`. The kind is any string
expression, as before. The call spelling is a parse error, `throw_call`, that
names the statement form.

This affects you if a program raises its own errors with `throw`, or uses
`throw` as the name of a variable or a function.

Before:

```rust
fn check(n: int) {
    if n > 2 {
        throw("too_big");
    }
}
```

After:

```rust
fn check(n: int) {
    if n > 2 {
        throw "too_big";
    }
}
```
