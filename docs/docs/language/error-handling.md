# Error handling

Candela separates the mistakes it can see from the ones it cannot. Type errors,
unknown names, and malformed syntax stop the program before it starts. What
remains at runtime is raised as an error you can catch.

## Compile errors

The compiler checks the whole program, including imported modules, before
running anything. A type mismatch, a call to an undeclared function, a missing
`main`, or a value used outside its scope is reported with the offending source
line and nothing runs. Fixing these is part of writing the program rather than
part of handling failure.

## Runtime errors

A runtime error carries a kind: a short lower-case string naming what went
wrong. The common ones come from conversions and lookups.

- `invalid_int`, `invalid_float`, `invalid_bool`: a conversion whose input does
  not parse.
- `index_out_of_bounds`, `slice_out_of_bounds`: a list or string index past the
  end.
- `unknown_map_key`: `get` on a key the map does not hold.
- `division_by_zero`, `modulo_by_zero`.
- `json_parse_error`: malformed input to `json_parse`.
- `fs_not_found`, `fs_permission_denied`, and the other `fs_` kinds: file system
  calls.

An error that nobody catches stops the program and prints a report pointing at
the expression that raised it.

## try and catch

Wrap the risky work in `try` and handle the failure in `catch`. A `catch` with a
quoted kind handles that kind only; a `catch` naming a variable handles anything
and binds the kind to that name. A `try` needs at least one `catch`.

```rust
fn main() {
    let zero = 0;
    try {
        print(10 / zero);
    } catch "division_by_zero" {
        print("cannot divide by zero");
    }

    try {
        let xs = [1];
        print(xs[9]);
    } catch e {
        print("failed: " + e);
    }
}
```

Several `catch` blocks may follow one `try`. The catch-all comes last and takes
whatever the earlier ones did not.

```rust
fn main() {
    try {
        print(int("not a number"));
    } catch "invalid_int" {
        print("that was not a number");
    } catch e {
        print("something else: " + e);
    }
}
```

What a `try` catches is what the block itself raises: a `throw`, and the
failures of operators and built-in functions. A call to a function written in
candela does not return when it sits inside a `try` block, and that includes the
standard library modules written in candela. Put the risky expression in the
block and call your own functions outside it.

Recovering usually means producing a fallback value, which reads best as a small
function.

```rust
fn parse_or(text, fallback) {
    let value = fallback;
    try {
        value = int(text);
    } catch e {
        value = fallback;
    }
    return value;
}

fn main() {
    print(parse_or("42", 0), parse_or("nope", 0));
}
```

## Raising your own

`throw` raises an error with the kind you give it, so your own failures catch
exactly like the built-in ones.

```rust
fn main() {
    let balance = 10;
    let amount = 50;
    try {
        if amount > balance {
            throw("insufficient_funds");
        }
        print(balance - amount);
    } catch "insufficient_funds" {
        print("declined");
    }
}
```

## Expected absence and failure

An error is for the unexpected. When a value may reasonably be missing, or an
operation may reasonably fail, return it in the type instead: `Option` for a
value that may be absent, `Result` for an operation that may fail. Both are
enums from the standard library, so the caller handles them with a `match` and
cannot forget the failing case.

```rust
import "std/result" as result;

fn divide(a, b) {
    if b == 0 {
        return Err("division by zero");
    }
    return Ok(a / b);
}

fn main() {
    match divide(10, 2) {
        Ok(value) => { print(value); }
        Err(message) => { print(message); }
    }
    print(result::unwrap_or(divide(1, 0), 0));
}
```

`unwrap` on a `None` or an `Err` raises, which is the deliberate way to say a
case cannot happen. See [Enums](enums.md) for matching them.

## Assertions

The `assert` module raises when a condition does not hold, which is how the
standard library's own tests fail loudly.

```rust
import "std/assert";

fn main() {
    assert_true(1 == 1);
    assert_eq(2, 2);
    print("checks passed");
}
```

A failed assertion stops the program with a message naming the two values.
