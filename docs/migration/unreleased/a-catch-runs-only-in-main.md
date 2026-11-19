# A `try` and `catch` run only in `main`

A failure a function expects now goes back to its caller as a value, a
`Result` (written `T!E`), and what is thrown ends the program unless `main`
catches it. A `try` written in any other function is reported as
`try_outside_main`. `value else fallback` and `value else |e| { ... }` handle
an `Option` or a `Result` in place, and `?` passes one on; in `main`, `?` ends
the program with the error. A `return`, `break` or `continue` that leaves a
`try` now also takes its catch down, so a later error is no longer caught by a
`try` the program already left.

This affects you if a function other than `main` writes `try`.

Before:

```rust
fn parse_or(text: string, fallback: int) -> int {
    let value = fallback;
    try {
        value = int(text);
    } catch e {
        value = fallback;
    }
    return value;
}
```

After:

```rust
fn parse_or(text: string, fallback: int) -> int {
    return text.parse<int>() else fallback;
}
```

A function that raised its own error to a caller's `catch` returns a `Result`
instead:

```rust
fn withdraw(balance: int, amount: int) -> int!string {
    if amount > balance {
        return Err("insufficient_funds");
    }
    return Ok(balance - amount);
}
```
