# Enums

An enum is a type with a fixed set of variants. Use one when a value is exactly
one of several alternatives, and let the compiler check that you handled them.

## Declaring

Declare an enum at the top level. A variant may carry a payload, written as
types in parentheses.

```rust
enum Event {
    Click(int, int),
    Key(string),
    Quit,
}

fn main() {
    print("declared");
}
```

Payload types follow the usual type grammar from [Types](types.md), so a payload
can be a list, a map, a struct, or `any` for a slot whose type the value decides.

## Constructing

Build a value by naming the variant, qualified with the enum or on its own. A
variant with a payload is constructed like a call.

```rust
enum Event {
    Click(int, int),
    Key(string),
    Quit,
}

fn main() {
    let a = Event::Click(3, 4);
    let b = Key("escape");
    let c = Event::Quit;
    print(type(a), str(c));
}
```

`type` on an enum value gives the enum's name; `str` gives the variant's name.

## Matching

`match` on an enum matches the variant and binds its payload. Name the variant
alone or qualify it with the enum; both forms work in an arm.

```rust
enum Event {
    Click(int, int),
    Key(string),
    Quit,
}

fn main() {
    let e = Event::Click(3, 4);
    match e {
        Click(x, y) => { print(x + y); }
        Key(name) => { print(name); }
        Quit => { print("quit"); }
    }
}
```

A payload binding introduces a variable for the arm's block. Use `_` in a
payload position to ignore it, and `_` as a whole arm for everything not listed.
A wildcard arm comes last.

```rust
enum Event {
    Click(int, int),
    Key(string),
    Quit,
}

fn main() {
    let e = Key("enter");
    match e {
        Key(_) => { print("a key"); }
        _ => { print("something else"); }
    }
}
```

## Comparing

Enum values compare with `==` and `!=`, which is often shorter than a `match`
when you care about one variant.

```rust
enum Colour {
    Red,
    Green,
}

fn main() {
    let c = Colour::Red;
    if c == Colour::Red {
        print("red");
    }
}
```

## Methods

Give an enum behaviour with an `impl` block. Inside a method, `match self` to
tell the variants apart; see [Methods](methods.md).

```rust
enum Colour {
    Red,
    Green,
}

impl Colour {
    fn hex(self) {
        let out = "";
        match self {
            Red => { out = "#f00"; }
            Green => { out = "#0f0"; }
        }
        return out;
    }
}

fn main() {
    print(Colour::Red.hex());
}
```

## Enums in the standard library

`Option` and `Result` are enums declared in candela and shipped with the
toolchain. `Option` is `Some(any)` or `None`; `Result` is `Ok(any)` or
`Err(any)`. They are matched exactly like your own enums.

```rust
import "std/option" as option;

fn main() {
    let found = Some(5);
    match found {
        Some(v) => { print(v); }
        None => { print("nothing"); }
    }
    print(option::unwrap_or(None, 0));
}
```

See [Error handling](error-handling.md) for when to reach for them.
