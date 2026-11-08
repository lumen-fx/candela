# A function in the attribute table carries its signature, and attributes go on enums

Two shapes in the attribute table changed for an embedding host that reads it.

`AttributeValue::Fn` was a tuple variant holding the function's name. It is
now a struct variant with the name, the parameters and the declared return
type, so a host can call a function named in an attribute:

Before:

```rust
if let AttributeValue::Fn(name) = value { run(name); }
```

After:

```rust
if let AttributeValue::Fn { name, params, ret } = value { run(name); }
```

Write `AttributeValue::Fn { name, .. }` where only the name matters.

`AttributeTarget` has a new variant, `Enum(u16)`, for an attribute written on
an enum. A `match` on `AttributeTarget` that lists every variant needs an arm
for it.

A parameter written `name: T?` in a host block's attribute declaration has no
entry in `Attribute::args` when the script leaves it out, so `arg(name)`
answers `None` for it. Parameters without `?` are recorded as before.
