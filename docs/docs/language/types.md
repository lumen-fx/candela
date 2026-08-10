# Types

Candela is statically typed with inference. You rarely write a type, but every
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
| Function | inferred | `fn(x) { return x; }` |

`int` is a signed 32-bit integer and `float` is double precision. A numeric
literal with a decimal point is a `float`; without one it is an `int`.

Lists and maps are covered in [Collections](collections.md), functions in
[Functions](functions.md), and your own types in [Enums](enums.md) and in
[Structs](#structs) below.

## Inspecting a type

`type(value)` returns the type as a string, which is the quickest way to check
what the compiler inferred.

```rust
fn main() {
    print(type("hi"), type(1), type(1.5), type(true), type(null));
    print(type([1, 2]), type({"a": 1}));
}
```

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

There is none. A condition is a `bool` expression, and the way to test a value
is to write the comparison out. A non-bool condition is not rejected by the
compiler, and it takes the true branch whatever its value, `0` and `null`
included, so write the comparison rather than relying on the value.

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

Type annotations appear in the places where the compiler cannot infer a type:
struct fields, enum variant payloads, and the signature blocks that declare
foreign functions. The type grammar is the same in all of them.

- `int`, `float`, `bool`, `string`, `null`: the built-in types.
- `T[]`: a list of `T`, for example `string[]` or `int[][]`.
- `{K: V}`: a map from `K` to `V`, for example `{string: int}`.
- A struct or enum name: that type.
- `A|B`: a union, a value that is either an `A` or a `B`.
- `any`: a slot whose type is decided by the value, allowed in an enum payload.

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

To give a struct behaviour, write an `impl` block; see [Methods](methods.md).
