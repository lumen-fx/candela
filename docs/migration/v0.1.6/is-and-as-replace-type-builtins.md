# The `is_*` and `as_*` built-ins are replaced by `is` and `as`

Testing and downcasting a dynamic value are operators now. `v is T` replaces
the seven `is_int`, `is_float`, `is_str`, `is_bool`, `is_list`, `is_map` and
`is_null` functions, and `v as T` replaces the six `as_int`, `as_float`,
`as_str`, `as_bool`, `as_list` and `as_map` functions. `T` is any type an
annotation names, so the operators also test structs, enums, unions and what a
list or a map holds. Inside an `if` or a `while` whose condition is `v is T`,
`v` already has the type `T`, so a test followed by a downcast becomes the test
alone. `is` is a keyword now.

Calling an old function is a compile error, `unknown_function`, whose help
names the replacement.

This affects you if a program calls any of the thirteen functions, or names a
variable or a function `is`.

| Before | After |
| --- | --- |
| `is_int(v)`, `is_float(v)`, `is_bool(v)` | `v is int`, `v is float`, `v is bool` |
| `is_str(v)` | `v is string` |
| `is_list(v)`, `is_map(v)` | `v is any[]`, `v is {any: any}` |
| `is_null(v)` | `v == null` |
| `as_int(v)`, `as_float(v)`, `as_bool(v)` | `v as int`, `v as float`, `v as bool` |
| `as_str(v)` | `v as string` |
| `as_list(v)`, `as_map(v)` | `v as any[]`, `v as {any: any}` |

`is` and `as` bind like `<`: looser than arithmetic, so `as_int(v) + 1` is
`v as int + 1`, and `x + as_int(v)` needs parentheses, `x + (v as int)`.

Before:

```rust
import "std/json" as json;

fn main() {
    let doc = as_map(json::parse("{\"n\": 7}"));
    if is_int(doc["n"]) {
        print(as_int(doc["n"]) + 1);
    }
}
```

After:

```rust
import "std/json";

fn main() {
    let doc = json::parse("{\"n\": 7}") as {string: any};
    let n = doc["n"];
    if n is int {
        print(n + 1);
    }
}
```
