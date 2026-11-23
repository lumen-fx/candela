# Option and Result lose unwrap_or; write a fallback with else

`unwrap_or` is gone from `Option` and `Result`. A fallback has one spelling,
`value else fallback`, which reads the value out of a `Some` or an `Ok` and
gives the fallback for a `None` or an `Err`. A call to `unwrap_or` is now the
compile error `no_such_method`, and its help names `x else fallback`.

`else` binds looser than every operator, so put a fallback inside an
arithmetic expression in parentheses.

```rust
// Before
let port = parse_port(text).unwrap_or(80);
print(m.get("a").unwrap_or(0) + 1);

// After
let port = parse_port(text) else 80;
print((m.get("a") else 0) + 1);
```
