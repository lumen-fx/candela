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
| Function | `fn(A) -> R` | `fn(x) { return x; }` |

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

The predicates `is_int`, `is_float`, `is_str`, `is_bool`, `is_list`, `is_map`,
and `is_null` answer the same question as a `bool`.

```rust
fn main() {
    print(is_int(1), is_str("a"), is_null(null));
}
```

## No implicit conversion

Candela never converts a value behind your back. Mixing an `int` and a `float`
in the same arithmetic, or adding a number to a string, is a compile error.
Convert first with `int`, `float`, `str`, or `bool`.

```rust
fn main() {
    let count = 3;
    let rate = 1.5;
    print(float(count) * rate);
    print("count: " + str(count));
}
```

`int` accepts a `string` or a `float`, `float` accepts a `string` or an `int`,
`str` accepts any value, and `bool` accepts the strings `"true"` and `"false"`.
A conversion that cannot succeed raises at runtime; see
[Error handling](error-handling.md).

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
it, so a variable is never implicitly empty. Test for it with `is_null`.

Passing `null` to `print` produces no output, so print a placeholder rather than
the value itself when a value may be absent. For a value that is optional by
design, prefer the `Option` enum from the standard library over `null`.

## Where you write a type

Type annotations appear on struct fields, enum variant payloads, the signature
blocks that declare foreign functions, and, where you want them, function
parameters and return types. The type grammar is the same in all of them.

- `int`, `float`, `bool`, `string`, `null`: the built-in types.
- `T[]`: a list of `T`, for example `string[]` or `int[][]`.
- `{K: V}`: a map from `K` to `V`, for example `{string: int}`.
- A struct or enum name: that type.
- `A|B`: a union, a value that is either an `A` or a `B`.
- `fn(A, B) -> R`: a function taking an `A` and a `B` and returning an `R`.
  Leave the arrow off for one that returns nothing.
- `(T)`: parentheses, which group a type the way they group an expression.
- `any`: a slot whose type is decided by the value, written on a struct
  field, a parameter, a return type, or an enum payload.

`[]` after a function type belongs to its return type, so `fn(int) -> int[]` is
a function returning a list of ints. Parenthesise to put the function itself in
a list: `(fn(int) -> int)[]`. The same rule reads a map value type,
`{string: fn(int) -> int}`, as a map of functions.

An annotation is not only checked against the value. Where a collection carries
no element type of its own the annotation supplies one: an empty list has no
element type and an empty map has neither a key nor a value type, so a parameter
declared `Value[]` decides that the body sees `Value` elements, a parameter
declared `{string: Value}` decides that it sees `string` keys and `Value`
values, and a `-> Value[]` or `-> {string: Value}` return type decides what the
caller gets back from `return []` or `return {}`. An annotation never overrides
a type the value does have, so passing `[1, 2]` to a `Value[]` parameter is
still an error. A `let` takes no annotation, so an empty collection in a local
takes its element types from the first value put into it instead.

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
back; name the type again with `as_int`, `as_str` or another downcast.

```rust
struct Slot {
    v: any,
}

fn main() {
    let s = Slot { v: 5 };
    print(as_int(s.v) + 1);
}
```

To give a struct behaviour, write an `impl` block; see [Methods](methods.md).
To let one struct hold a field of whatever type you name at the point of use,
give it a type parameter; see [Generics](generics.md).
