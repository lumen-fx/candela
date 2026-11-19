# An impl function is a method only when its first parameter is `self`

An `impl` function used to be a method whenever it declared any parameter,
whatever the parameter was called, and only a function with no parameters at
all was called by its path. Now the first parameter decides: a function whose
first parameter is `self` is a method, called with a dot, and every other
`impl` function is called by its path, with or without arguments. That gives a
type constructors that take arguments, `Point::new(1, 2)`. `self` is accepted
only as the first parameter of an `impl` function.

This affects you if a method names its receiver something other than `self`,
or if a free function or a later parameter is named `self`.

Before:

```rust
impl Point {
    fn len(this) => this.x + this.y;
}
fn point(x: int, y: int) -> Point => Point { x: x, y: y };
```

After:

```rust
impl Point {
    fn len(self) => self.x + self.y;
    fn new(x: int, y: int) -> Point => Point { x: x, y: y };
}
```

Call the constructor as `Point::new(1, 2)`.
