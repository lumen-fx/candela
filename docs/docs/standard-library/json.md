---
icon: lucide/braces
---
# Json library

Parses and serializes json text. Import the library with `import "std/json";`
at the top-level.

*[at the top-level]: Outside of any function.

This module is written in Candela and uses no dynamic library, so a program
that imports it builds to a `.cdlb` artifact that runs under `candela-vm` with
the module bytecode inlined.

## What a value parses into

`parse` maps each json type onto a value candela already has; there is no
separate json type:

- an object becomes a map (`{string: any}`)
- an array becomes a list (`any[]`)
- a string becomes a `string`
- a number becomes an `int` when it has no fractional part or exponent and
  fits in one, and a `float` otherwise
- `true` and `false` become `bool`
- `null` becomes the null value

A map does not preserve insertion order, so an object's entries come back in
hash order rather than the order they appeared in the source text; the same
is true in reverse when `stringify` writes a map back out.

Because a parsed document mixes types under one value, `parse` returns `any`.
Read a field back with a type test (`is_int`, `is_float`, `is_str`, `is_bool`,
`is_list`, `is_map`, `is_null`) or a checked downcast (`as_int`, `as_float`,
`as_str`, `as_bool`, `as_list`, `as_map`). These are global functions that work
on any `any`-typed value, not part of this module, so they need no import.
A downcast raises a catchable error (`catch "bad_downcast"` or `catch e`) when
the value does not hold the type you asked for.

A malformed document raises a catchable error from `parse`; catch it by name
(`catch "json_parse_error"`) or with the catch-all (`catch e`). See
[Error handling](../language-tour/error-handling.md).

### Parse
`parse(text: string) -> any`: parses `text` as json and returns the value.
Raises a catchable error when `text` is not valid json.
```rust
import "std/json" as json;

fn main() {
    let doc = as_map(json::parse("{\"name\": \"a\", \"nums\": [1, 2, 3]}"));
    print(as_str(doc.get("name")));   // "a"
    let nums = as_list(doc.get("nums"));
    print(as_int(nums[1]));           // 2
}
```
### Stringify
`stringify(value) -> string`: serializes `value` to a json string. A struct or
enum value has no json shape, so it is written as its text form, quoted as a
string.
```rust
import "std/json" as json;

fn main() {
    print(json::stringify([1, 2, 3]));   // [1,2,3]
    print(json::stringify({"a": 1}));    // {"a":1}
}
```

## Reading a parsed value

`parse` returns `any`, so use these global functions to test or extract the
concrete type underneath. They are not specific to json: the same functions
work on any dynamically typed value.

- `is_int(x) -> bool`, `is_float(x) -> bool`, `is_str(x) -> bool`,
  `is_bool(x) -> bool`, `is_list(x) -> bool`, `is_map(x) -> bool`,
  `is_null(x) -> bool`: true when `x` currently holds a value of the named
  type.
- `as_int(x) -> int`, `as_float(x) -> float`, `as_str(x) -> string`,
  `as_bool(x) -> bool`, `as_list(x) -> T[]`, `as_map(x) -> {K: V}`: `x`
  downcast to the named type. Raises a catchable error when `x` does not hold
  that type.
