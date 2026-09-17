# option

`Option` is a value that is either present or absent.

```rust
import "std/option";
```

## The type

`Option` is an ordinary candela enum with a type parameter for what it holds:

```rust
enum Option<T> {
    Some(T),
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

`Some(x)` needs no type argument: the value it is given decides the option's
type, so `Some(5)` is an `Option<int>` and the value bound in a `Some` arm comes
back with the type that went in.

```rust
import "std/option";

struct Point {
    x: int,
    y: int,
}

fn nearest(points: Point[]) -> Option<Point> {
    if points.len() == 0 {
        return None;
    }
    return Some(points[0]);
}

fn main() {
    match nearest([Point { x: 1, y: 2 }]) {
        Some(p) => { print(p.x, p.y); }
        None => { print("no points"); }
    }
}
```

`None` names no payload, so on its own it is an `Option<any>`, which goes
wherever an `Option<T>` is expected. Name the argument (`Option<Point>`) where
you want the check. See [enums](../language/enums.md) for the enum and match
syntax, and [generics](../language/generics.md) for type parameters.

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## Methods

The helpers are methods on the option value, defined in an `impl Option<T>`
block; importing the module brings them in.

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

- `default`: the value to return when the option is `None`. It has the option's
  own payload type.
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
