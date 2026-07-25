---
icon: lucide/check-check
---
# Assert library

Assertions for writing Candela tests. Import the library with
`import std::assert;` at the top-level.

*[at the top-level]: Outside of any function.

Each assertion raises a catchable error (via `throw`) when it fails, so a test
file runs its checks in sequence and the first failure stops the run with a
message naming the check. A test file that reaches its end and prints a success
line has passed.

This module is written in Candela and uses no dynamic library, so a program that
imports it builds to a `.cdlb` artifact that runs under `candela-vm` with the
module bytecode inlined.

```rust
import std::assert;

fn main() {
    assert::assert_eq(1 + 1, 2);
    assert::assert_true("hello".starts_with("he"));
    print("ok");
}
```

### Assert
`assert(cond: bool)` -- raises "assertion failed" when `cond` is false.
### Assert msg
`assert_msg(cond: bool, msg: string)` -- raises `msg` when `cond` is false.
### Assert true
`assert_true(cond: bool)` -- raises when `cond` is false.
### Assert false
`assert_false(cond: bool)` -- raises when `cond` is true.
### Assert eq
`assert_eq(a, b)` -- raises a message showing both sides when `a` and `b` differ.
### Assert ne
`assert_ne(a, b)` -- raises a message showing the value when `a` and `b` are
equal.
