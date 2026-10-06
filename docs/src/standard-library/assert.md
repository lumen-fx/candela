# assert

Assertions for writing candela tests.

```rust
import "std/assert";
```

The functions are reached through the module's name, `assert::eq(a, b)`. A test
file that calls a few of them often can name them in the import instead,
`import "std/assert" { eq, ne };`, and call `eq(a, b)`.

Each assertion raises an error when its check fails, so a test file runs its
checks in sequence and the first failure stops the run and prints the message.
A thrown message doubles as the error's code, so a harness reading the run's
output can match on it. Wrapping a check in `try` catches the failure instead of
ending the run, which is how a harness reports the failing check and carries on.
See [error handling](../language/error-handling.md).

```rust
import "std/assert";

fn main() {
    try {
        assert::eq(2, 3);
    } catch e {
        print("check failed: " + e);
    }
}
```

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## that

```rust
assert::that(cond)
```

- `cond`: a bool.
- Returns: nothing.

Raises `assertion failed` when `cond` is false.

## msg

```rust
assert::msg(cond, message)
```

- `cond`: a bool.
- `message`: the string to raise.
- Returns: nothing.

Raises `message` when `cond` is false. Use it when the check alone does not say what
went wrong.

```rust
import "std/assert";

fn main() {
    let users = ["ada", "grace"];
    assert::msg(users.len() == 2, "expected two users");
    print("ok");
}
```

## is_true

```rust
assert::is_true(cond)
```

- `cond`: a bool.
- Returns: nothing.

Raises `expected true` when `cond` is false.

## is_false

```rust
assert::is_false(cond)
```

- `cond`: a bool.
- Returns: nothing.

Raises `expected false` when `cond` is true.

## eq

```rust
assert::eq(a, b)
```

- `a`, `b`: two values of the same type.
- Returns: nothing.

Raises `assert::eq failed: <a> != <b>` when the two differ, rendering both sides
with `string`. Comparison is `!=`, so it works on any type the operator accepts,
including lists and maps.

## ne

```rust
assert::ne(a, b)
```

- `a`, `b`: two values of the same type.
- Returns: nothing.

Raises `assert::ne failed: both sides are <a>` when the two are equal.
