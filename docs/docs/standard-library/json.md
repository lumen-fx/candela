# json

Parse a json string into candela values, and serialise candela values back to
json.

```rust
import "std/json" as json;
```

Both functions wrap a built-in: `json::parse` is `json_parse` and
`json::stringify` is `json_stringify`. The module is pure candela, so it compiles
into a `.cdlb` artifact and runs under `candela-vm` with no dynamic library.

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

The result is typed `any`, so read a field back with a downcast (`as_int`,
`as_str`, `as_map`, `as_list`) or test it first with `is_int`, `is_map`, and the
rest. Those are [built-in functions](builtins.md).

```rust
import "std/json" as json;

fn main() {
    let doc = json::parse("{\"name\": \"ada\", \"scores\": [1, 2, 3]}");
    let obj = as_map(doc);
    print(as_str(obj.get("name")));
    print(as_list(obj.get("scores")).len());
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
quoted string of its literal form. A map serialises in the map's own iteration
order, which is unspecified.

```rust
import "std/json" as json;

fn main() {
    let m = {"a": 1, "b": 2};
    print(json::stringify(m));
    print(json::stringify([1.5, 2.5]));
}
```
