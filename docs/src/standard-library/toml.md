# toml

TOML, the format of `candela.toml`: text to candela values and back.

```rust
import "std/toml";
```

```rust
import "std/toml";

fn main() {
    let config = toml::parse("[server]\nport = 8080\n")? as {string: any};
    let server = config["server"] as {string: any};
    print(server["port"]);
}
```

A document reads the way [`json::parse`](json.md) reads JSON: a table becomes
a `{string: any}` map, an array a list, and strings, ints, floats and bools
the same values. TOML's dates and times have no candela type, so they read as
their text.

The module binds the `toml` native library, which ships in the toolchain's
`libs/std_src/toml/`. A browser runs no native library, so there both calls
raise `toml_desktop_only`.

## parse

```rust
toml::parse(text)
```

- Returns: the document, an `any` holding a map, in an `any!string`;
  `Err("toml_invalid")` for text that is not TOML.

## stringify

```rust
toml::stringify(value)
```

- `value`: a map, such as one `parse` gave.
- Returns: `value` written as TOML, in a `string!string`;
  `Err("toml_unrepresentable")` for a value with no TOML form, such as `null`
  or a list at the top.
