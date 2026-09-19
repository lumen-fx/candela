# convert

Type conversions under names that read as verbs, as methods on the value. Each
method wraps the built-in conversion of the same effect; see
[built-in functions](builtins.md) for `int`, `float`, `str`, and `bool`.

```rust
import "std/convert";
```

The import brings the methods in; each conversion is defined on the receiver
types it makes sense for. The module is pure candela, so it compiles into a
`.cdlb` artifact and runs under `candela-vm` with no dynamic library.

## to_int

```rust
s.to_int()
f.to_int()
```

- Receiver: a string or a float.
- Returns: an int.

Parses a string, or truncates a float towards zero. Raises `Invalid integer`
when the string does not parse.

## to_float

```rust
s.to_float()
n.to_float()
```

- Receiver: a string or an int.
- Returns: a float.

Parses a string, or widens an int. Raises `Invalid float` when the string does
not parse.

## to_string

```rust
n.to_string()
f.to_string()
b.to_string()
```

- Receiver: an int, a float, or a bool.
- Returns: a string.

Renders the value in its string form. Never raises. For other types, the
built-in `str(x)` renders any value.

## to_bool

```rust
s.to_bool()
```

- Receiver: a string.
- Returns: a bool.

Parses `"true"` and `"false"`. Any other string raises `The string could not be
parsed into a boolean`.

```rust
import "std/convert";

fn main() {
    print("42".to_int() + 1);
    print(3.9.to_int());
    print(2.5.to_string() + "!");
    print("true".to_bool());
}
```
