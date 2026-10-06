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
    fn area(self) -> int => self.w * self.h;

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

A method's body is a block, or `=> expr;` for one that returns a single
expression, the same as a free function's; see
[Functions](functions.md#returning).

`impl` blocks go at the top level, next to the type they belong to. A type may
have several. A block on a generic type names the type arguments it is written
against, and a method can take type parameters of its own
(`s.tagged<string>("hi")`); see [Generics](generics.md).

## What self is

`self` is an ordinary first parameter, not a keyword. It receives the value the
method was called on, and its type is the type the `impl` block names. Any name
works, but `self` is the convention and reads best.

A function in an `impl` block that declares no parameters has no receiver. It
belongs to the type rather than to a value, so it is called by the type's path,
and calling it with a dot is a compile error. This is how a type gets a
constructor:

```rust
struct Point {
    x: int,
    y: int,
}

impl Point {
    fn origin() -> Point => Point { x: 0, y: 0 };
}

fn main() {
    let p = Point::origin();
    print(p.x, p.y);
}
```

A generic type names its instantiation in front of the function,
`Set<int>::new()`, and a type from a module bound with `as` keeps the alias in
front, `shapes::Point::origin()`. A function that does declare a receiver is a
method, so calling it by its path is a compile error that points at the dot
call. One receiverless function has a special role: a struct's `default`
replaces the struct's built-in default and is what `Options::default()` calls;
see [Default values](types.md#default-values).

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
    fn label(self) => match self {
        Stop => "stop",
        Go => "go",
    };
}

fn main() {
    print(Signal::Go.label());
}
```

## Resolution

The receiver's type decides which function a call reaches. Two types may both
define `len`, and neither collides with the other or with a free function of the
same name.

A field that holds a function is called through the same dot: `obj.cb(x)` calls
what the field `cb` holds, when the receiver's type has no method of that name.
A method wins, so adding one never changes which call a program was already
making. See [Functions](functions.md).

Calling a name the struct or enum has no method and no function-holding field
for is a compile error at the call, naming the type, so a misspelling is caught
before the program runs.

## Operators on your own types

A method named by an operator symbol is what that operator reaches on a value
of the type. `a + b` calls the `+` method the type of `a` defines, with `b` as
the other operand.

```rust
struct Vec2 {
    x: float,
    y: float,
}

impl Vec2 {
    fn +(self, other: Vec2) -> Vec2 {
        return Vec2 { x: self.x + other.x, y: self.y + other.y };
    }

    fn -(self) -> Vec2 {
        return Vec2 { x: -self.x, y: -self.y };
    }

    fn ==(self, other: Vec2) -> bool {
        return self.x == other.x && self.y == other.y;
    }

    fn <(self, other: Vec2) -> bool {
        return self.x < other.x;
    }
}

fn main() {
    let a = Vec2 { x: 1.0, y: 2.0 };
    let b = Vec2 { x: 3.0, y: 4.0 };
    print((a + b).x);   // 4
    print(a != b);      // true
    print(b > a);       // true
    a += b;             // a = a + b
}
```

The operators a type defines are `+`, `-`, `*`, `/`, `%`, `^`, `&`, `|`, `^^`,
`<<`, `>>`, `==`, `<` and `<=` between two values, and `-` and `~` in front of
one. `-` is the one symbol with both forms: the parameter count says which is
being declared, and a type may define both.

`!=`, `>` and `>=` need no method. `!=` is the type's `==` with the answer
flipped, `>` is its `<` with the operands swapped, and `>=` is its `<=` the same
way. The swap is in the operands, so a derived comparison evaluates the right
operand first. `==`, `<` and `<=` answer a question about two values, so each
returns a `bool`.

The other operand is of the receiver's type unless the method declares
otherwise, so `fn *(self, k: float) -> Vec2` is what a vector scales by a
number with. Only the left operand decides which method an operator reaches:
`2.0 * v` is a type error however `Vec2` defines `*`, because there is one
spelling of an operator method and it belongs to the type on the left.

A compound assignment applies the matching operator, so `a += b` reaches `+`
and needs nothing of its own. There is no method-call spelling of an operator:
`a + b` is how the method is called.

Built-in types are closed. What `+` means on an `int` or a `string` is part of
the language, so an operator method inside `impl int` or `impl list` is a
compile error; wrap the value in a type of your own instead. An operator a type
does not define is the ordinary operator error, and the report names the method
that would make the expression work. See [errors](../reference/errors.md) and
[operators](../reference/operators.md).

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
`len` never resolves, `[1, 2].len()` stays the built-in length.

## The built-in methods

These come with the language and need no import.

- Strings: `len`, `uppercase`, `lowercase`, `trim`, `trim_start`, `trim_end`,
  `trim_chars`, `trim_start_chars`, `trim_end_chars`, `starts_with`,
  `ends_with`, `contains`, `index_of`, `replace`, `split`, `repeat`, `reverse`,
  `parse`.
- Lists: `len`, `push`, `remove`, `contains`, `index_of`, `sort`, `reverse`,
  `repeat`, `join`, `split`.
- Maps: `len`, `remove`, `contains`, `keys`, `values`. An entry is read and
  written with brackets, `m[k]` and `m[k] = v`.
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
`count`, `unique`, `chunk`, `take`, and `drop`. See
[Collections](collections.md). The standard library's `string` and `map`
modules add methods to strings and maps the same way, also with no import: they are all part of the
[prelude](../standard-library/overview.md#the-prelude).

The `Option` and `Result` types from the standard library are enums with `impl`
blocks in the prelude, so their helpers are called as methods with no import;
see [Error handling](error-handling.md).
