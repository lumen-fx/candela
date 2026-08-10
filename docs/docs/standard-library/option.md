# option

`Option` is a value that is either present or absent.

```rust
import "std/option" as option;
```

## The type

`Option` is an ordinary candela enum:

```rust
enum Option {
    Some(any),
    None,
}
```

Importing the module brings the variants into scope, so you construct and match
them directly:

```rust
import "std/option" as option;

fn main() {
    let o = Some(5);
    match o {
        Some(v) => { print(v); }
        None => { print("nothing"); }
    }
}
```

The payload is typed `any`, so read it back with a downcast (`as_int`, `as_str`)
or work on it through type-agnostic operations such as `str`. See
[enums](../language/enums.md) for the enum and match syntax, and
[built-in functions](builtins.md) for the downcasts.

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## Helpers

Every helper has two spellings that do the same work: a free function taking the
option as its first argument, and a method on the value. Under a bare
`import "std/option";` the free functions are unqualified (`unwrap(o)`); under
`as option` they are qualified (`option::unwrap(o)`). The method form
(`o.unwrap()`) works either way.

### is_some

```rust
option::is_some(o)
o.is_some()
```

- Returns: a bool, true when the option holds a value.

### is_none

```rust
option::is_none(o)
o.is_none()
```

- Returns: a bool, true when the option is empty.

### unwrap

```rust
option::unwrap(o)
o.unwrap()
```

- Returns: the contained value.
- Raises: `called unwrap on a None option` when the option is `None`.

### unwrap_or

```rust
option::unwrap_or(o, default)
o.unwrap_or(default)
```

- `default`: the value to return when the option is `None`.
- Returns: the contained value, or `default`.
- Raises: nothing.

### map

```rust
option::map(o, f)
o.map(f)
```

- `f`: takes the contained value, returns the mapped value.
- Returns: `Some(f(v))` when the option holds `v`, and `None` when it is empty.
  `f` is not called on a `None`.

```rust
import "std/option" as option;

fn describe(x) { return "value " + str(x); }

fn main() {
    let s = Some(5);
    let n = None;
    print(s.map(describe).unwrap());
    print(n.unwrap_or(0));
}
```

For a value that carries a reason for being absent, use
[result](result.md) instead.
