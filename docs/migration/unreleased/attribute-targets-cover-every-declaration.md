# AttributeTarget has a variant for every kind of declaration

Attributes now go on methods, parameters, constants, fields, variants and
`impl` blocks as well as on functions, structs and enums. An embedding host
that reads the attribute table finds the new kinds as new `AttributeTarget`
variants: `Method`, `Param`, `Const`, `Field`, `Variant` and `Impl`.

A host that matches every variant no longer compiles. Add the kinds it reads,
or a wildcard arm for the rest:

```rust
// Before
match &attr.target {
    AttributeTarget::Fn { name, .. } => systems.push(name.clone()),
    AttributeTarget::Struct(_) | AttributeTarget::Enum(_) => {}
}

// After
match &attr.target {
    AttributeTarget::Fn { name, .. } => systems.push(name.clone()),
    _ => {}
}
```
