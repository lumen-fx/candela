# The string trim methods are renamed to `trim_start`, `trim_end` and `trim_chars`

The trims name the end of the string they work on as start and end, which holds
for right-to-left text, and the ones that strip a set of characters say so:

| Before | After |
| --- | --- |
| `s.trim_left()` | `s.trim_start()` |
| `s.trim_right()` | `s.trim_end()` |
| `s.trim_sequence(chars)` | `s.trim_chars(chars)` |
| `s.trim_sequence_left(chars)` | `s.trim_start_chars(chars)` |
| `s.trim_sequence_right(chars)` | `s.trim_end_chars(chars)` |

`s.trim()` is unchanged. The behaviour of each is unchanged. Calling an old
name is a compile error, `no_such_method`, whose help names the new one.

This affects you if a program calls one of the old names.

Before:

```rust
fn main() {
    print("  hi  ".trim_left() + "|");
    print("--hi--".trim_sequence("-"));
}
```

After:

```rust
fn main() {
    print("  hi  ".trim_start() + "|");
    print("--hi--".trim_chars("-"));
}
```
