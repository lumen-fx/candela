# option

`Option` is a value that is either present or absent. It is part of the
[prelude](overview.md#the-prelude), so it needs no import. A lookup that may
find nothing answers one: `m.get(k)`, `xs.first()`, `xs.max()`.

## The type

`Option` is an ordinary candela enum with a type parameter for what it holds:

```rust
enum Option<T> {
    Some(T),
    None,
}
```

You construct and match the variants directly:

```rust
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

`x?` reads the value out of a `Some` and hands a `None` back to the caller of
the function it is written in, which has to return an `Option` too:

```rust
fn total(prices: {string: int}) -> Option<int> {
    let a = prices.get("apple")?;
    let b = prices.get("pear")?;
    return Some(a + b);
}

fn main() {
    print(total({"apple": 2, "pear": 3}));
    print(total({"apple": 2}));
}
```

See [error handling](../language/error-handling.md#passing-a-failure-on) for
the rule.

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## Methods

The helpers are methods on the option value, defined in an `impl Option<T>`
block.

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

### and_then

```rust
o.and_then(f)
```

- `f`: takes the contained value and returns an option of its own.
- Returns: `f(v)` when the option holds `v`, and `None` when it is empty. `f` is
  not called on a `None`. Use it to chain steps that may each come back empty,
  where `map` would give you an option inside an option.

### or

```rust
o.or(other)
```

- `other`: the option to fall back on. It has the same payload type.
- Returns: this option when it holds a value, and `other` when it is empty.

### filter

```rust
o.filter(pred)
```

- `pred`: takes the contained value and returns a bool.
- Returns: this option when it holds a value `pred` answers true for, and `None`
  otherwise. `pred` is not called on a `None`.

```rust
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
