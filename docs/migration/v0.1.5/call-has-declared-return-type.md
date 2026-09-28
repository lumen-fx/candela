# A call has the type its function declares, not the type its body returns

A call used to take the type of the value the function's body returns, even
when the function declared something wider. It now takes the declared return
type. A function declared `-> any` therefore gives its callers an `any`, even
when every `return` in it is an `int`.

This affects you if a function declares a wider return type than it returns and
a caller uses the result as the narrower type. That code no longer compiles:

```rust
fn limit() -> any {
    return 10;
}

fn main() {
    print(limit() + 1); // Cannot perform operation any + int
}
```

Declare the type the function returns:

```rust
fn limit() -> int {
    return 10;
}
```

Or, where the function has to stay `-> any`, downcast at the call:

```rust
fn main() {
    print(as_int(limit()) + 1);
}
```

A generic function declared to return a type parameter the call leaves unbound
still takes the type its body returns.
