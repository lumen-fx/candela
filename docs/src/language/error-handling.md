# Error handling

Candela separates the mistakes it can see from the ones it cannot, and the
failures a program expects from the ones it does not. Type errors, unknown
names and malformed syntax stop the program before it starts. A failure the
program expects, a missing file or a string that is not a number, is a value:
an `Option` or a `Result` the caller handles. Anything else is a bug, which
ends the program unless `main` catches it.

## Compile errors

The compiler checks the whole program, including imported modules, before
running anything. A type mismatch, a call to an undeclared function, a missing
`main`, or a value used outside its scope is reported with the offending source
line and nothing runs. Fixing these is part of writing the program rather than
part of handling failure.

## Expected absence and failure

When a value may reasonably be missing, or an operation may reasonably fail,
say so in the type: `T?`, an `Option<T>`, for a value that may be absent, and
`T!E`, a `Result<T, E>`, for an operation that may fail with an error of type
`E`. Both are enums from the prelude, so they need no import, and the caller
cannot forget the failing case. The lookups that may find nothing answer an
`Option`: `m.get(k)`, `xs.first()`, `s.parse<int>()`. The standard library
answers a `Result` where it can fail: `fs::read`, `json::parse`.

```rust
fn divide(a: int, b: int) -> int!string {
    if b == 0 {
        return Err("division by zero");
    }
    return Ok(a / b);
}

fn main() {
    match divide(10, 2) {
        Ok(value) => print(value),
        Err(message) => print(message),
    }
    if divide(1, 0) is Err(e) {
        print("failed: " + e);
    }
}
```

`E` is any type: a string, or an enum that names each way the operation fails.

### A fallback with else

`value else fallback` reads the value out of a `Some` or an `Ok`, and gives the
fallback for a `None` or an `Err`. `value else |e| { ... }` binds an `Err`'s
error for the block, whose last expression is the fallback. A fallback that
leaves, with `return`, `break`, `continue` or `throw`, gives no value: the
variable after it always holds a real one.

```rust
fn port(text: string) -> int {
    let n = text.parse<int>() else {
        return 8080;
    };
    return n;
}

fn half(n: int) -> int!string {
    if n % 2 == 1 {
        return Err("odd: " + string(n));
    }
    return Ok(n / 2);
}

fn main() {
    print("42".parse<int>() else 0, "x".parse<int>() else 0);
    print(port("80"), port("eighty"));
    print(half(3) else |e| { e.len() });
}
```

`else` binds looser than every operator, so `a + b else 0` falls back for the
whole sum.

### Passing a failure on

`x?` after an `Option` or a `Result` reads the value out of a `Some` or an `Ok`.
On a `None` or an `Err` it returns that value from the function it is written
in, at once, so the caller gets the failure:

```rust
fn parse(text: string) -> int!string {
    if text.parse<int>() is Some(n) {
        return Ok(n);
    }
    return Err("not a number: " + text);
}

fn sum(a: string, b: string) -> int!string {
    let x = parse(a)?;
    let y = parse(b)?;
    return Ok(x + y);
}

fn main() {
    print(sum("1", "2"));
    print(sum("1", "two"));
}
```

The function a `?` is in has to return the same enum as its operand: an
`Option` for a `?` on an `Option`, and for a `Result` one whose error type is
the same. The success side does not have to match, since a `None` or an `Err`
carries none of it. A function with no return annotation that uses `?` returns
that enum. In `main`, which has no caller, a `?` on a `None` or an `Err` ends
the program with the error.

```rust
import "std/fs";

fn main() {
    fs::write("notes.txt", "hello")?;
    print(fs::read("notes.txt")?);
    fs::delete("notes.txt")?;
}
```

## What should not happen

A bug ends the program. The runtime raises an error when a program does what it
cannot: an index past the end of a list, a division by zero, `unwrap` on a
`None`. Each error carries a kind, a short lower-case string naming what went
wrong.

- `invalid_int`, `invalid_float`, `invalid_bool`: a conversion whose input does
  not parse. `s.parse<int>()` answers an `Option` instead.
- `index_out_of_bounds`, `slice_out_of_bounds`: a list or string index past the
  end.
- `unknown_map_key`: `m[k]` on a key the map does not hold. `m.get(k)` answers
  an `Option` instead.
- `division_by_zero`, `modulo_by_zero`.

`throw "kind";` raises one of your own, for a case your program rules out. An
error unwinds every call between where it was raised and `main`, then prints a
report pointing at the expression that raised it. A host that called into the
program receives it as the call's error.

### Catching in main

`main` can catch what would end the program, to report it its own way or carry
on. A `catch` with a quoted kind handles that kind only; a `catch` naming a
variable handles anything and binds the kind to that name. A `try` needs at
least one `catch`, and the catch-all comes last. A `try` written in any other
function is a compile error: a failure a function expects goes back to its
caller as a `Result`.

```rust
fn withdraw(balance: int, amount: int) -> int {
    if amount > balance {
        throw "insufficient_funds";
    }
    return balance - amount;
}

fn main() {
    try {
        print(withdraw(100, 30));
        print(withdraw(10, 50));
    } catch "insufficient_funds" {
        print("declined");
    } catch e {
        print("failed: " + e);
    }
}
```

A quoted kind that no built-in error carries and nothing in the program throws
is reported as a warning, since a misspelt kind never matches; see
[the error catalogue](../reference/errors.md#warnings).

## Assertions

The `assert` module raises when a condition does not hold, which is how the
standard library's own tests fail loudly.

```rust
import "std/assert";

fn main() {
    assert::is_true(1 == 1);
    assert::eq(2, 2);
    print("checks passed");
}
```

A failed assertion raises the message it prints, so it stops the program on its
own, and a `try` in `main` around the check catches it like any other error.
That is how a test harness reports which check failed instead of ending the
run.

```rust
import "std/assert";

fn main() {
    try {
        assert::eq(2, 3);
    } catch e {
        print("check failed: " + e);
    }
}
```
