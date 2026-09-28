# Returning an any from a function declared with another type checks the value

A function declared to return a type other than `any` used to hand an `any`
value through to its caller unchecked, whatever the value held. The value is
now checked on the way out, and a value of the wrong type raises
`bad_downcast`.

This affects you if a function returns an `any`, such as the result of
`json_parse`, from a function declared with a narrower type, and the value can
hold something else. This used to print `x` and now stops with
`Cannot read this string value as int`:

```rust
fn count(text: string) -> int {
    return json_parse(text);
}

fn main() {
    print(count("\"x\""));
}
```

Catch the error where a wrong value is expected:

```rust
fn main() {
    try {
        print(count("\"x\""));
    } catch "bad_downcast" {
        print("not a number");
    }
}
```

Or declare the function `-> any` and check the value in the caller:

```rust
fn count(text: string) -> any {
    return json_parse(text);
}

fn main() {
    let value = count("\"x\"");
    if is_int(value) {
        print(as_int(value) + 1);
    } else {
        print("not a number");
    }
}
```
