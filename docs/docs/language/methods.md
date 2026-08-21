# Methods

A method is a function called with a receiver in front of it: `value.name(args)`.
The built-in types come with methods, and an `impl` block adds methods to your
own types and to the built-in ones.

## impl blocks

An `impl` block names a type and holds functions whose first parameter is the
receiver. The type is a struct or enum you declare, or one of the built-in type
names: `string`, `list`, `map`, `int`, `float`, `bool`.

```rust
struct Rect {
    w: int,
    h: int,
}

impl Rect {
    fn area(self) {
        return self.w * self.h;
    }

    fn scaled(self, factor) {
        return Rect { w: self.w * factor, h: self.h * factor };
    }
}

fn main() {
    let r = Rect { w: 2, h: 3 };
    print(r.area());
    print(r.scaled(2).area());
}
```

`impl` blocks go at the top level, next to the type they belong to. A type may
have several. A block on a generic type names the type arguments it is written
against, and a method can take type parameters of its own
(`s.tagged<string>("hi")`); see [Generics](generics.md).

## What self is

`self` is an ordinary first parameter, not a keyword. It receives the value the
method was called on, and its type is the type the `impl` block names. Any name
works, but `self` is the convention and reads best.

Parameters after the first behave exactly like function parameters: name them,
pass them positionally, and either leave the type to be inferred from the call
or pin it with `name: type`. A method takes a `-> Type` return annotation on the
same terms as a free function, and it is checked against what the body returns.

A method call is compiled to a plain function call with the receiver passed as
the first argument, so `r.area()` and a free function taking a `Rect` cost the
same and behave the same.

## Mutating through a method

Structs, lists, and maps are passed by reference, so a method that writes to
`self` changes the value the caller holds.

```rust
struct Counter {
    n: int,
}

impl Counter {
    fn bump(self, by) {
        self.n = self.n + by;
    }
}

fn main() {
    let c = Counter { n: 0 };
    c.bump(5);
    print(c.n);
}
```

## Methods on enums

An enum takes methods the same way. Inside, `match self` to tell the variants
apart; see [Enums](enums.md).

```rust
enum Signal {
    Stop,
    Go,
}

impl Signal {
    fn label(self) {
        let out = "";
        match self {
            Stop => { out = "stop"; }
            Go => { out = "go"; }
        }
        return out;
    }
}

fn main() {
    print(Signal::Go.label());
}
```

## Resolution

The receiver's type decides which function a call reaches. Two types may both
define `len`, and neither collides with the other or with a free function of the
same name.

Calling a method a struct or enum does not define is a compile error naming the
type, so a misspelling is caught before the program runs.

## Methods on the built-in types

An `impl` block can also name a built-in type, which is how the standard
library's collection modules define their helpers:

```rust
impl string {
    fn shout(self) {
        return self.uppercase() + "!";
    }
}

fn main() {
    print("hey".shout());
}
```

The built-in methods listed below keep precedence: an `impl list` method named
`len` never resolves, `[1, 2].len()` stays the built-in length. The one
exception is `find` on a list: with a function argument it resolves to the
standard library's predicate search, because the built-in `find` is the index
search by value and the argument type picks between the two.

## The built-in methods

These come with the language and need no import.

- Strings: `len`, `uppercase`, `lowercase`, `trim`, `trim_left`, `trim_right`,
  `trim_sequence`, `trim_sequence_left`, `trim_sequence_right`, `starts_with`,
  `ends_with`, `contains`, `find`, `replace`, `split`, `repeat`, `reverse`,
  `is_int`, `is_float`.
- Lists: `len`, `push`, `remove`, `contains`, `find`, `sort`, `reverse`,
  `repeat`, `join`, `partition`.
- Maps: `len`, `get`, `insert`, `contains`, `keys`, `values`.
- Integers: `abs`.
- Floats: `abs`, `sqrt`, `round`, `floor`.

```rust
fn main() {
    print("  Candela  ".trim().lowercase());
    print([3, 1, 2].len());
    print((-4).abs(), 2.25.sqrt());
}
```

Lists carry a second set of methods from the standard library's `list` module,
available without an import: `map`, `filter`, `reduce`, `each`, `any`, `all`,
`find`, `sort_by`, `first`, `last`, `is_empty`, `sum`, `product`, `min`, `max`,
`index_of`, `count`, `unique`, `chunk`, `take`, and `drop`. See
[Collections](collections.md). The standard library's `string` and `map`
modules add methods to strings and maps the same way, behind an import.

The `Option` and `Result` types from the standard library are enums with `impl`
blocks, so their helpers are called as methods once the module is imported; see
[Error handling](error-handling.md).
