# A break or continue with no loop around it is a compile error

`break` and `continue` used to compile anywhere. Outside a loop they jumped to
a place nothing set: a program could hang, and a `break` in a function called
from a loop left the caller's loop. They now compile only inside a `for`,
`while` or `loop` body of their own function, and anywhere else raise
`outside_loop`.

This affects you if a function uses `break` or `continue` to stop the loop of
the code that called it.

Before:

```rust
fn check(n: int) {
    if n > 2 { break; }
    print(n);
}

fn main() {
    for n in 0..5 { check(n); }
}
```

After, telling the caller when to stop:

```rust
fn check(n: int) -> bool {
    if n > 2 { return false; }
    print(n);
    return true;
}

fn main() {
    for n in 0..5 {
        if !check(n) { break; }
    }
}
```
