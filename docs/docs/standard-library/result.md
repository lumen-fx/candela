# result

`Result` is either a success or a failure that carries a reason.

```rust
import "std/result" as result;
```

## The type

`Result` is an ordinary candela enum:

```rust
enum Result {
    Ok(any),
    Err(any),
}
```

Importing the module brings the variants into scope, so you construct and match
them directly:

```rust
import "std/result" as result;

fn main() {
    let r = Ok(5);
    match r {
        Ok(v) => { print(v); }
        Err(e) => { print(e); }
    }
}
```

Both payloads are typed `any`, so read one back with a downcast (`as_int`,
`as_str`) or work on it through type-agnostic operations such as `str`. See
[enums](../language/enums.md) for the enum and match syntax.

A `Result` is a value you pass around and inspect. It is separate from the
language's raised errors, which unwind to a `try`/`catch`; see
[error handling](../language/error-handling.md).

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## Helpers

Every helper has two spellings that do the same work: a free function taking the
result as its first argument, and a method on the value. Under a bare
`import "std/result";` the free functions are unqualified (`unwrap(r)`); under
`as result` they are qualified (`result::unwrap(r)`). The method form
(`r.unwrap()`) works either way.

### is_ok

```rust
result::is_ok(r)
r.is_ok()
```

- Returns: a bool, true when the result is `Ok`.

### is_err

```rust
result::is_err(r)
r.is_err()
```

- Returns: a bool, true when the result is `Err`.

### unwrap

```rust
result::unwrap(r)
r.unwrap()
```

- Returns: the success value.
- Raises: `called unwrap on an Err result` when the result is `Err`. The message
  does not include the error payload; read it with `unwrap_err`.

### unwrap_err

```rust
result::unwrap_err(r)
r.unwrap_err()
```

- Returns: the error value.
- Raises: `called unwrap_err on an Ok result` when the result is `Ok`.

### unwrap_or

```rust
result::unwrap_or(r, default)
r.unwrap_or(default)
```

- `default`: the value to return when the result is `Err`.
- Returns: the success value, or `default`.
- Raises: nothing.

```rust
import "std/result" as result;

fn parse_port(text) {
    if text.is_int() {
        return Ok(int(text));
    }
    return Err("not a number: " + text);
}

fn main() {
    print(parse_port("8080").unwrap());
    print(parse_port("http").unwrap_or(80));
    print(parse_port("http").unwrap_err());
}
```

There is no `map` on `Result`; match on the value when you want to transform one
side.
