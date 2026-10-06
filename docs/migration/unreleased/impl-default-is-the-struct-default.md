# A `default` function in a struct's `impl` block is now its default

A `default` declared in a struct's `impl` block used to have no effect:
`S::default()` and every other default built the struct from its field
values. It now replaces that default, and a `default` that does not fit the
new role is a compile error.

This affects you if an `impl` block declares a function named `default`.

A `fn default() -> S` with no parameters now runs wherever a default of the
struct is built: `S::default()`, a literal ending in a bare `..`, and a field
whose type is the struct. If it builds something other than what
the field values give, those call sites now see its value.

A `default` with parameters, including a method `fn default(self)`, or one that
returns another type, no longer compiles:

```rust
impl Config {
    fn default(self) -> Config { // struct_default_signature
        return self;
    }
}
```

Rename the method, or make it the struct's default:

```rust
impl Config {
    fn reset(self) -> Config {
        return self;
    }
}
```
