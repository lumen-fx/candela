# Types

Candela is statically typed with inference. Most types go unwritten, but every
expression has one, and the compiler rejects a program whose types do not line
up before it runs.

## The built-in types

| Type | Written as | Example values |
| --- | --- | --- |
| Integer | `int` | `0`, `42`, `-7` |
| Floating point | `float` | `1.5`, `0.0`, `-2.75` |
| String | `string` | `"hello"`, `""` |
| Boolean | `bool` | `true`, `false` |
| Absent value | `null` | `null` |
| List | `T[]` | `[1, 2, 3]` |
| Map | `{K: V}` | `{"a": 1}` |
| Function | `fn(A) -> R` | `fn(x) => x` |

`int` is a signed 64-bit integer and `float` is double precision. A numeric
literal with a decimal point is a `float`; without one it is an `int`. An
exponent makes a `float` too, with a point or without: `1e3`, `2.5e-1`,
`6.02e23`. An `int` literal outside -9223372036854775808 to 9223372036854775807
is a compile error at the literal; give it a decimal point or an exponent to
write it as a `float` instead. A `float` literal past what double precision
holds, roughly 1.8e308, is a compile error at the literal too, rather than an
infinity the program then runs on.

Arithmetic that leaves the range wraps around it rather than raising, so a sum
past the top comes back at the bottom.

Lists and maps are covered in [Collections](collections.md), functions in
[Functions](functions.md), and your own types in [Enums](enums.md) and in
[Structs](#structs) below. A function's type is inferred wherever the value
says it; write `fn(A) -> R` where a declaration has to say what it accepts,
such as a struct field or a parameter.

## Inspecting a type

`type(value)` returns the type as a string, which is the quickest way to check
what the compiler inferred.

```rust
fn main() {
    print(type("hi"), type(1), type(1.5), type(true), type(null));
    print(type([1, 2]), type({"a": 1}));
}
```

Every answer is written the way the same type is written in a program: a struct
or an enum by its declared name, a dynamic value as `any`, and a function as
`fn(A) -> R`. It is the type the compiler inferred, so a value it could not pin
reads `any` however the program later fills it.

### Testing a type with `is`

`value is T` answers whether the value is a `T`, as a `bool`. `T` is any type
an annotation can name: `int`, a struct, an enum, `int[]`, `{string: any}`, a
union, or a generic type with its arguments, such as `Option<int>`. A list or a
map is checked down to what it holds, so a list of `any` passes `is int[]` only
when every element is an `int`.

```rust
import "std/json" as json;

fn main() {
    let v = json::parse("[1, 2, 3]");
    print(v is int[], v is string[], v is any[]);
}
```

Where the static type already answers the test, it costs nothing to run. A
test for a type the value can never have answers `false`: a function compiles
once for each type its callers pass, and the test is how one body serves them
all. `null` is a value rather than a type, so test for it with `value == null`.

Inside the body of an `if` or a `while` whose condition is `v is T`, the
variable `v` has the type `T`, and so does every use of it to the right of an
`&&` after the test. An `else if` with a test of its own does the same for its
branch. An `else` branch is not narrowed; write `else if v is U` to name the
other type.

```rust
fn describe(v) -> string {
    if v is int {
        return "the number " + string(v + 1);
    } else if v is string {
        return "the word " + v.uppercase();
    }
    return "something else";
}

fn main() {
    print(describe(41), describe("hi"), describe(true));
}
```

On an enum, `is` tests the variant with the pattern a `match` arm would write,
and the names in the pattern hold the payload in the same places a narrowed
variable has its type.

```rust
fn main() {
    let found = {"port": 80}.get("port");
    if found is Some(p) && p > 0 {
        print("port " + string(p));
    }
    print(found is None);
}
```

`is` binds as tightly as `<` and `>`: looser than arithmetic, tighter than
`==` and `&&`. See [operators](../reference/operators.md).

### Downcasting with `as`

`value as T` hands the value on typed as `T`, checking it while the program
runs. A value of another type raises `bad_downcast`, which a `catch` can take;
see [Error handling](error-handling.md). `T` is any type `is` accepts, so `as`
also turns the `{any: any}` a JSON document gives into the `{string: any}` a
function declares, checking every key.

```rust
import "std/json" as json;

fn port(cfg: {string: any}) -> int {
    if cfg.get("port") is Some(p) && p is int {
        return p;
    }
    return 8080;
}

fn main() {
    let cfg = json::parse("{\"port\": 80}") as {string: any};
    print(port(cfg));
}
```

Where the static type already is a `T`, `as` does nothing. Where it can never
be one, such as `3 as string`, the downcast is a compile error: that is a
conversion, `string(3)`.

## No implicit conversion

Candela never converts a value behind your back. Mixing an `int` and a `float`
in the same arithmetic, or adding a number to a string, is a compile error.
Convert first by calling the type you want: `int`, `float`, `string`, or
`bool`.

```rust
fn main() {
    let count = 3;
    let rate = 1.5;
    print(float(count) * rate);
    print("count: " + string(count));
}
```

`int` accepts a `string` or a `float`, `float` accepts a `string` or an `int`,
`string` accepts any value, and `bool` accepts the strings `"true"` and
`"false"`. A conversion that cannot succeed raises at runtime; see
[Error handling](error-handling.md).

Text that may not hold a number is read with `s.parse<T>()`, which answers an
`Option`: `Some` with the value, or `None` where the text is not one. `T` is
`int`, `float` or `bool`.

```rust
fn main() {
    print("42".parse<int>(), "4.5".parse<float>(), "nope".parse<int>());
    if "7".parse<int>() is Some(n) {
        print(n + 1);
    }
}
```

## Truthiness

There is none, and none is added for you. `0`, `""` and `null` are not tests,
and a condition of any type but `bool` is a compile error at the condition. Write
the comparison out.

```rust
fn main() {
    let items = [];
    if items.len() == 0 {
        print("empty");
    }
}
```

## null

`null` is the value of an expression that has nothing to return, such as a
function that returns without a value. It is its own type: no other type accepts
it, so a variable is never implicitly empty. Test for it with `value == null`.

`null` is a value you write in code, never a type you write in an annotation:
`x: null`, `-> null` and `int|null` are all rejected. A function that returns
nothing leaves its return annotation off.

`print(null)` prints `null`. `null` stands for "no value was returned" and
nothing else: a value that may be absent is an `Option`, which the lookups that
may find nothing (`m.get(k)`, `xs.first()`) answer, and which needs no import.

## Where you write a type

Type annotations appear on struct fields, enum variant payloads, the signature
blocks that declare foreign functions, and, where you want them, function
parameters, return types and `let` declarations. The type grammar is the same in
all of them.

- `int`, `float`, `bool`, `string`: the built-in types. `null` is not among
  them; see [null](#null).
- `T[]`: a list of `T`, for example `string[]` or `int[][]`.
- `{K: V}`: a map from `K` to `V`, for example `{string: int}`.
- A struct or enum name: that type.
- `A|B`: a union, a value that is either an `A` or a `B`.
- `fn(A, B) -> R`: a function taking an `A` and a `B` and returning an `R`.
  Leave the arrow off for one that returns nothing.
- `(T)`: parentheses, which group a type the way they group an expression.
- `any`: a slot whose type is decided by the value, written on a struct
  field, a parameter, a return type, a `let`, or an enum payload.

`[]` after a function type belongs to its return type, so `fn(int) -> int[]` is
a function returning a list of ints. Parenthesise to put the function itself in
a list: `(fn(int) -> int)[]`. The same rule reads a map value type,
`{string: fn(int) -> int}`, as a map of functions.

An annotation is not only checked against the value. Where a collection carries
no element type of its own the annotation supplies one: an empty list has no
element type and an empty map has neither a key nor a value type, so a parameter
declared `Value[]` decides that the body sees `Value` elements, a parameter
declared `{string: Value}` decides that it sees `string` keys and `Value`
values, a `-> Value[]` or `-> {string: Value}` return type decides what the
caller gets back from `return []` or `return {}`, and `let xs: Value[] = [];`
decides what the local holds. An annotation never overrides a type the value
does have, so passing `[1, 2]` to a `Value[]` parameter is still an error. An
empty collection in a `let` without an annotation takes its element types from
the first value put into it instead.

## Structs

A struct groups named fields into one type. Declare it at the top level, with a
type for every field.

```rust
struct Point {
    x: int,
    y: int,
}

fn main() {
    let p = Point { x: 3, y: 4 };
    print(p.x, p.y);
}
```

Build a value with `Name { field: value, ... }`, giving every field. Read and
write a field with a dot.

```rust
struct Point {
    x: int,
    y: int,
}

fn main() {
    let p = Point { x: 0, y: 0 };
    p.x = 10;
    print(p.x + p.y);
}
```

A field can hold any type, including a list, a map, or a union.

```rust
struct Item {
    name: string,
    tags: string[],
    counts: {string: int},
    id: int|string,
}

fn main() {
    let it = Item { name: "widget", tags: ["new"], counts: {"sold": 2}, id: "w-1" };
    print(it.name, it.tags, it.counts, it.id);
}
```

A field declared `any` takes a value of any type, and reading it gives an `any`
back; name the type again with `as`, or test it with `is`.

```rust
struct Slot {
    v: any,
}

fn main() {
    let s = Slot { v: 5 };
    print(s.v as int + 1);
}
```

### Building from another value

End a literal with `..base` to take every field it does not write from `base`,
another value of the same struct. The literal is a new value; `base` is left as
it was.

```rust
struct Style {
    color: string,
    size: int,
    bold: bool,
}

fn main() {
    let body = Style { color: "black", size: 12, bold: false };
    let heading = Style { size: 18, bold: true, ..body };
    print(heading.color, heading.size, body.size);
}
```

The base comes last and is any expression of the literal's type; a base of
another type is a compile error, and so is a written field the struct does not
declare. A field taken from the base holds the same value the base holds, so a
list or a map in it is shared between the two, as it is when one variable is
assigned to another.

### Default values

A field may declare the value it starts with, after its type:

```rust
struct Options {
    cwd: string = ".",
    retries: int = 3,
    verbose: bool,
    env: {string: string},
}
```

`Options::default()` builds a value from those declarations. A field that
declares none starts empty: `0`, `0.0`, `false`, `""`, `[]`, `{}`, `null` for an
`any`, and the default of its own type for a
struct field. An enum or a function has no empty value, so a field of one needs
a declared value before its struct has a default; asking for the default of such
a struct is a compile error that names the field.

End a literal with a bare `..` to take every field it does not write from that
default. It gives a value that states only what differs:

```rust
struct Options {
    cwd: string = ".",
    retries: int = 3,
    verbose: bool,
}

fn run(name: string, opts: Options) {
    print(name, opts.cwd, opts.retries, opts.verbose);
}

fn main() {
    run("build", Options { verbose: true, .. });
    run("test", Options { .. });
    run("lint", Options::default());
}
```

`Options { .. }` and `Options::default()` are the same value.

A declared value is an expression, evaluated each time a default is built, in
the file that declares the struct and with none of the caller's variables in
scope. Each default therefore gets lists and maps of its own.

To build the default in code instead, declare a `default` function in the
struct's `impl` block. It takes no parameters and returns the struct, and it
replaces the default the fields give everywhere one is built:
`Window::default()`, a literal ending in `..`, and a field whose type is the
struct.

```rust
struct Window {
    width: int = 800,
    height: int = 600,
    area: int,
}

impl Window {
    fn default() -> Window {
        let w = Window { .. };
        w.area = w.width * w.height;
        return w;
    }
}

fn main() {
    print(Window::default().area);
}
```

Inside that function the struct's default is still the one its fields give, so
`Window { .. }` and `Window::default()` there start from the declared values
instead of calling the function again. A `default` that takes parameters
or returns another type is a compile error.

To give a struct behaviour, write an `impl` block; see [Methods](methods.md).
To let one struct hold a field of whatever type you name at the point of use,
give it a type parameter; see [Generics](generics.md).
