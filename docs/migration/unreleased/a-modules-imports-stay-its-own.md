# What a module imports no longer reaches the files that import it

A module's own bare imports used to come along with it: importing `a.cdl`
also brought in the names `a.cdl` merged from its imports, and a module bound
with `as` exposed the modules it bound as nested namespaces (`a::json::parse`).
Now a file sees what a module declares and nothing it imports. A name that
arrived this way is a compile error, `unknown_function` or
`unknown_namespace`.

This affects you if a program calls a function or names a type that one of its
modules imports rather than declares.

Before, with `shapes.cdl` holding `import "std/math";` and its own functions:

```rust
import "./shapes.cdl" as shapes;

fn main() {
    print(shapes::math::sqrt(16.0));
}
```

After, importing the module it needs itself:

```rust
import "std/math";
import "./shapes.cdl";

fn main() {
    print(math::sqrt(16.0));
}
```
