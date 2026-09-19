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

A variant with no payload, written bare, names no type arguments of a
[generic](generics.md) enum, and is accepted wherever an instantiation that
names them is expected.

An enum from a module bound with `as` is reached through the alias and then the
enum, because a variant belongs to its enum rather than to the module:
`shapes::Shape::Circle(1)`. A [generic](generics.md) enum carries its type
arguments on the enum name in the middle of that path:
`shapes::Slot<int>::Filled(1)`. The alias on its own, `shapes::Circle(1)`, names
no function and does not compile. See [modules](modules.md).

## Matching

`match` on an enum matches the variant and binds its payload. Name the variant
alone, qualify it with the enum, or write the full path through a module alias;
all three forms work in an arm. A qualifier is resolved rather than skipped, so
it has to lead to the enum the matched value has: a pattern qualified with
another enum, with an alias that reaches one, or with another instantiation of a
generic enum is a compile error, even where both enums declare a variant of the
same name.

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

candela matches variant arms only when it knows which enum the value is. A
parameter takes the type its call site passes it, so matching a parameter works
when the caller hands it an enum value. An element read out of an empty array
literal has no type, and matching one against variants is a compile error rather
than an arm that never fits. A declaration says what a collection holds where
the literal does not: a parameter typed `Event[]` or `{string: Event}`, or the
same as a return type, gives the body `Event` values however empty the list or
map it is handed. See [the empty list](collections.md#the-empty-list) and [the
empty map](collections.md#the-empty-map).

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
toolchain. `Option<T>` is `Some(T)` or `None`; `Result<T, E>` is `Ok(T)` or
`Err(E)`. The constructors take no type argument, since the value handed to one
decides what the enum holds, and they are matched exactly like your own enums.

```rust
import "std/option";

fn main() {
    let found = Some(5);
    match found {
        Some(v) => { print(v); }
        None => { print("nothing"); }
    }
    print(None.unwrap_or(0));
}
```

See [Error handling](error-handling.md) for when to reach for them.
