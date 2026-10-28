# `print(null)` prints `null`

Printing `null`, the value of a call that returns nothing, used to write
nothing at all, not even a line break. It now writes `null` on its own line,
under `candela` and `candela-vm` alike.

This affects you if a program prints a value that can be `null` and relies on
the line being left out.

Before, this printed one line, `done`; it now prints `done` and then `null`:

```rust
fn log(message) {
    print(message);
}

fn main() {
    print(log("done"));
}
```

Print only the values you mean to show:

```rust
fn log(message) {
    print(message);
}

fn main() {
    log("done");
}
```
