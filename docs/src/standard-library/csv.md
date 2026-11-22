# csv

Comma-separated values: rows of text fields, as RFC 4180 writes them.

```rust
import "std/csv";
```

```rust
import "std/csv";

fn main() {
    let rows = csv::parse("name,qty\nbolt,4\n")?;
    print(rows[1][0]);
    print(csv::stringify([["a", "b,c"], ["1", "2"]]));
}
```

Every field is a string; convert one with `int()` or
[`s.parse<T>()`](builtins.md#string-methods).

## parse

```rust
csv::parse(text)
```

- Returns: the rows of `text`, each a `string[]` of its fields, as a
  `string[][]!string`. A row ends at `\n` or `\r\n`, and a line break at the
  very end starts no row. A quoted field can hold commas, line breaks, and a
  quote written twice. A quote that never closes gives
  `Err("csv_unterminated_quote")`.

## stringify

```rust
csv::stringify(rows)
```

- Returns: `rows` as CSV text, each row ending in `\n`. A field is quoted when
  it holds a comma, a quote or a line break, and a quote inside it is doubled.
