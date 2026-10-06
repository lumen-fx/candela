# The file functions are the `std/fs` module

`fs::read`, `fs::write` and the other file functions used to be a built-in
namespace that needed no import. They are now the `std/fs` module, so a program
that uses them imports it. The functions, their arguments and the errors they
raise are unchanged. A call with no import is a compile error,
`unknown_namespace`, whose help names the import.

This affects you if a program calls any `fs::` function.

Before:

```rust
fn main() {
    fs::write("notes.txt", "hello");
    print(fs::read("notes.txt"));
}
```

After:

```rust
import "std/fs";

fn main() {
    fs::write("notes.txt", "hello");
    print(fs::read("notes.txt"));
}
```
