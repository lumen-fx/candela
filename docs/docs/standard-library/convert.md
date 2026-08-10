# convert

Type conversions under names that read as verbs. Each function wraps the
built-in conversion of the same effect; see
[built-in functions](builtins.md) for `int`, `float`, `str`, and `bool`.

```rust
import "std/convert" as convert;
```

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## to_int

```rust
convert::to_int(x)
```

- `x`: a string or a float.
- Returns: an int.

Parses a string, or truncates a float towards zero. Raises `Invalid integer`
when the string does not parse.

## to_float

```rust
convert::to_float(x)
```

- `x`: a string or an int.
- Returns: a float.

Parses a string, or widens an int. Raises `Invalid float` when the string does
not parse.

## to_string

```rust
convert::to_string(x)
```

- `x`: any value.
- Returns: a string.

Renders the value in its string form. Never raises.

## to_bool

```rust
convert::to_bool(x)
```

- `x`: a string.
- Returns: a bool.

Parses `"true"` and `"false"`. Any other string raises `The string could not be
parsed into a boolean`.

```rust
import "std/convert" as convert;

fn main() {
    print(convert::to_int("42") + 1);
    print(convert::to_int(3.9));
    print(convert::to_string(2.5) + "!");
    print(convert::to_bool("true"));
}
```
