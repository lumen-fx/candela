# A host block struct from an imported module is named once

A file that imported a module holding `host "ecs" { struct Entity { ... } }`
had to write the namespace twice to reach the struct, `ecs::ecs::Entity`: once
for the module and once for the block. It is now named like any other type the
module declares, `ecs::Entity`, or `Entity` with `import "ecs.cdl" { Entity };`.
The doubled spelling no longer compiles.

This affects you if a program names a struct from another file's `host` block.
The type name the host receives in `Value::Struct` is unchanged.

Before:

```rust
import "ecs.cdl";

fn main() {
    let e = ecs::ecs::Entity { id: 1 };
}
```

After:

```rust
import "ecs.cdl";

fn main() {
    let e = ecs::Entity { id: 1 };
}
```
