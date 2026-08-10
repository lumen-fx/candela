# assert

Assertions for writing candela tests.

```rust
import "std/assert" as assert;
```

Each assertion raises an error when its check fails, so a test file runs its
checks in sequence and the first failure stops the run and prints the message.
A thrown message doubles as the error's code, so a harness reading the run's
output can match on it. An assertion is written in candela, so calling one
inside a `try` block does not return; a failed check ends the run rather than
reaching a `catch`. See [error handling](../language/error-handling.md).

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## assert

```rust
assert::assert(cond)
```

- `cond`: a bool.
- Returns: nothing.

Raises `assertion failed` when `cond` is false.

## assert_msg

```rust
assert::assert_msg(cond, msg)
```

- `cond`: a bool.
- `msg`: the string to raise.
- Returns: nothing.

Raises `msg` when `cond` is false. Use it when the check alone does not say what
went wrong.

```rust
import "std/assert" as assert;

fn main() {
    let users = ["ada", "grace"];
    assert::assert_msg(users.len() == 2, "expected two users");
    print("ok");
}
```

## assert_true

```rust
assert::assert_true(cond)
```

- `cond`: a bool.
- Returns: nothing.

Raises `expected true` when `cond` is false.

## assert_false

```rust
assert::assert_false(cond)
```

- `cond`: a bool.
- Returns: nothing.

Raises `expected false` when `cond` is true.

## assert_eq

```rust
assert::assert_eq(a, b)
```

- `a`, `b`: two values of the same type.
- Returns: nothing.

Raises `assert_eq failed: <a> != <b>` when the two differ, rendering both sides
with `str`. Comparison is `!=`, so it works on any type the operator accepts,
including lists and maps.

## assert_ne

```rust
assert::assert_ne(a, b)
```

- `a`, `b`: two values of the same type.
- Returns: nothing.

Raises `assert_ne failed: both sides are <a>` when the two are equal.
