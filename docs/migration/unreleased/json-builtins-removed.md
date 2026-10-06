# The `json_parse` and `json_stringify` built-ins are removed

The two built-in functions are gone. Parsing and serialising json is
`json::parse` and `json::stringify` from `std/json`, which behave exactly as the
built-ins did. Calling either old name is a compile error, `unknown_function`,
whose help names the module function and its import.

This affects you if a program calls `json_parse` or `json_stringify`.

Before:

```rust
fn main() {
    let doc = json_parse("{\"n\": 7}");
    print(json_stringify(doc));
}
```

After:

```rust
import "std/json" as json;

fn main() {
    let doc = json::parse("{\"n\": 7}");
    print(json::stringify(doc));
}
```
