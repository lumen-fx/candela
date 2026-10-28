# A set is made with `set::Set<int>::new()`; the module function `new` is removed

std/set no longer declares a module-level `new<T>()`. The constructor is a
function of the `Set<T>` type, called by its path: `set::Set<int>::new()`, or
`Set<int>::new()` when the import names the type with
`import "std/set" { Set };`. A function
an `impl` block declares with no parameters is now called this way for every
type, so `Point::origin()` works the same. Calling `set::new` is a compile
error, `unknown_function_in_namespace`, whose help names
`set::Set<T>::new()`.

This affects you if a program makes a set.

Before:

```rust
import "std/set" as set;

fn main() {
    let s = set::new<int>();
    s.add(1);
    print(s.len());
}
```

After:

```rust
import "std/set";

fn main() {
    let s = set::Set<int>::new();
    s.add(1);
    print(s.len());
}
```
