# json

Parse a json string into candela values, and serialise candela values back to
json.

```rust
import "std/json" as json;
```

The module is pure candela over the runtime's own json primitives, so it
compiles into a `.cdlb` artifact and runs under `candela-vm` with no dynamic
library.

## parse

```rust
json::parse(text)
```

- `text`: the json document, as a string.
- Returns: the parsed value, typed `any`.
- Raises: `json_parse_error` when the input is malformed. The message names the
  reason: `unexpected end of input`, `object key must be a string`, `unterminated
  string`, `invalid number`, `invalid escape`, `trailing characters after value`,
  and the rest.

Objects and arrays nest to a fixed depth, far beyond what a document written for
a program to read reaches. Text that nests deeper is rejected as malformed with
the reason `nesting too deep`, so a hostile document cannot exhaust the stack.

The mapping from json to candela is:

| json | candela |
| --- | --- |
| object | map keyed by strings |
| array | list |
| string | string |
| number without a fraction or exponent | int |
| number with a fraction or exponent | float |
| `true`, `false` | bool |
| `null` | null |

An object becomes a map that holds its keys in document order, which is the
order `stringify` writes them back out in.

The result is typed `any`, so read a field back with a downcast, `v as int`, or
test it first with `v is int`; see
[types](../language/types.md#testing-a-type-with-is). A document mixes types
within one object or array, and the downcast keeps that: `as {string: any}`
gives a map with string keys and `any` values, so `m[k] = v` stores a string
beside an int.

```rust
import "std/json" as json;

fn main() {
    let doc = json::parse("{\"name\": \"ada\", \"scores\": [1, 2, 3]}");
    let obj = doc as {string: any};
    print(obj["name"] as string);
    print((obj["scores"] as int[]).len());
}
```

## stringify

```rust
json::stringify(value)
```

- `value`: any value.
- Returns: a json string.
- Raises: nothing.

Ints, floats, bools, strings, null, lists, and maps serialise to their json
counterparts. A float keeps a decimal point so it parses back as a float, and a
float that is infinite or not a number serialises as `null`. A map key that is
not a string is rendered to its text form and quoted, because json object keys
are strings. A struct or enum value has no json shape, so it serialises as a
quoted string of its literal form. A map serialises in the order its keys went
in, so a document parsed with `parse` and handed straight back to `stringify`
keeps the key order it was written with.

```rust
import "std/json" as json;

fn main() {
    let m = {"a": 1, "b": 2};
    print(json::stringify(m));
    print(json::stringify([1.5, 2.5]));
}
```
