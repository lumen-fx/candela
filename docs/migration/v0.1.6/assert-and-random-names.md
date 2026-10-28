# std/assert and std/random drop the module name from their functions

Now that an import binds the module's name, the functions that repeated it are
renamed:

| Before | After |
| --- | --- |
| `assert(cond)` | `assert::that(cond)` |
| `assert_msg(cond, message)` | `assert::msg(cond, message)` |
| `assert_true(cond)` | `assert::is_true(cond)` |
| `assert_false(cond)` | `assert::is_false(cond)` |
| `assert_eq(a, b)` | `assert::eq(a, b)` |
| `assert_ne(a, b)` | `assert::ne(a, b)` |
| `random::random_int()` | `random::int()` |
| `random::random_int_range(min, max)` | `random::int_range(min, max)` |
| `random::random()` | `random::float()` |
| `random::random_range(min, max)` | `random::float_range(min, max)` |

The failure messages of `eq` and `ne` start `assert::eq failed:` and
`assert::ne failed:` instead of `assert_eq failed:` and `assert_ne failed:`.
Nothing else changes. Calling an old name is a compile error,
`unknown_function` or `unknown_function_in_namespace`, whose help names the new
one.

This affects you if a program or a test calls one of the old names, or matches
on the old failure message.

Before:

```rust
import "std/assert";

fn main() {
    assert_true(1 < 2);
    assert_eq(2 + 2, 4);
}
```

After:

```rust
import "std/assert";

fn main() {
    assert::is_true(1 < 2);
    assert::eq(2 + 2, 4);
}
```
