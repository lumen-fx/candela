# `math::sqrt`, `floor`, `ceil`, `round` and `trunc` are number methods

A float rounds and takes its square root through its own methods, which every
program has with no import. `std/math` no longer declares `sqrt`, `floor`,
`ceil`, `round` or `trunc`; the methods `x.ceil()` and `x.trunc()` join
`x.sqrt()`, `x.round()`, `x.floor()` and `x.abs()`. Each answers a float, as
the math function did. `std/math` keeps trigonometry, logarithms, `pow`,
`cbrt`, `hypot`, `fmod`, `copysign`, `erf` and the constants.

This affects you if a program calls one of the five removed functions.

Before:

```rust
import "std/math";

fn main() {
    print(math::sqrt(2.0));
    print(math::ceil(7.0 / 2.0));
}
```

After:

```rust
fn main() {
    print(2.0.sqrt());
    print((7.0 / 2.0).ceil());
}
```
