# option

`Option` is a value that is either present or absent.

```rust
import "std/option";
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
import "std/option";

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

## Methods

The helpers are methods on the option value, defined in an `impl Option` block;
importing the module brings them in.

### is_some

```rust
o.is_some()
```

- Returns: a bool, true when the option holds a value.

### is_none

```rust
o.is_none()
```

- Returns: a bool, true when the option is empty.

### unwrap

```rust
o.unwrap()
```

- Returns: the contained value.
- Raises: `called unwrap on a None option` when the option is `None`.

### unwrap_or

```rust
o.unwrap_or(default)
```

- `default`: the value to return when the option is `None`.
- Returns: the contained value, or `default`.
- Raises: nothing.

### map

```rust
o.map(f)
```

- `f`: takes the contained value, returns the mapped value.
- Returns: `Some(f(v))` when the option holds `v`, and `None` when it is empty.
  `f` is not called on a `None`.

```rust
import "std/option";

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
