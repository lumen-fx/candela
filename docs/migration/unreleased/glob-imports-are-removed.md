# `import "m" { * };` is removed

An import names the items it brings in, or binds the module and reaches its
items through that name. `{ * }` is a compile error whose help shows both.
`export import` stays and names its items too, so a module that gathers
several files lists what it passes on.

This affects you if a program or a package writes `{ * }` in an import or an
`export import`.

Before:

```rust
import "./geometry.cdl" { * };
export import "./circle.cdl" { * };

fn main() {
    print(area(2, 3));
}
```

After:

```rust
import "./geometry.cdl" { area };
export import "./circle.cdl" { Circle, circle_area };

fn main() {
    print(area(2, 3));
}
```

or, reaching the items through the module's name:

```rust
import "./geometry.cdl";

fn main() {
    print(geometry::area(2, 3));
}
```
