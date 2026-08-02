---
icon: lucide/split
---
# Result library

The `Result` type: either a success (`Ok`) or a failure (`Err`). Import the
library with `import "std/result";` at the top-level.

*[at the top-level]: Outside of any function.

Reach for `Result` when a function can fail and the caller needs to know why:
returning `Err(reason)` carries the failure as an ordinary value the caller
can inspect, pass along, or store, rather than unwinding the call stack
immediately the way `throw` does.

`Result` is an ordinary enum, so `Ok(x)` and `Err(e)` are constructed directly
and matched with payload binding, the same as any other enum:

```rust
import "std/result";

fn main() {
    let r = Ok(5);
    match r {
        Ok(v) => { print(v); }
        Err(e) => { print(e); }
    }
}
```

The payloads of `Ok` and `Err` are both typed `any`, so either can hold a
value of any type; a failure is commonly a string message, but is not
required to be one.

The `Result` enum and the `Ok`/`Err` constructors are always reachable
unqualified, even through a namespaced import (`import "std/result" as
result;`); only the free functions below sit behind the `result::` alias.

Every function below works both as a free function (`unwrap(r)`, with a bare
import) and as a method on the value (`r.unwrap()`), since `result.cdl` also
defines an `impl Result` block; see
[Methods (`impl` blocks)](../language-tour/data-types.md#methods-impl-blocks).

This module is written in Candela and uses no dynamic library, so a program
that imports it builds to a `.cdlb` artifact that runs under `candela-vm` with
the module bytecode inlined.

### Is ok
`is_ok(r: Result) -> bool`: true when the result is `Ok`. Method form:
`r.is_ok()`.
### Is err
`is_err(r: Result) -> bool`: true when the result is `Err`. Method form:
`r.is_err()`.
### Unwrap
`unwrap(r: Result) -> any`: the success value, or raises when the result is
`Err`. Method form: `r.unwrap()`.
```rust
import "std/result" as result;

fn main() {
    let ok = Ok(5);
    print(result::unwrap(ok));   // 5
    print(ok.unwrap());          // 5, same call as a method
}
```
### Unwrap err
`unwrap_err(r: Result) -> any`: the error value, or raises when the result is
`Ok`. Method form: `r.unwrap_err()`.
```rust
import "std/result" as result;

fn main() {
    let err = Err("boom");
    print(result::unwrap_err(err));   // "boom"
    print(err.unwrap_err());          // "boom"
}
```
### Unwrap or
`unwrap_or(r: Result, default: any) -> any`: the success value, or `default`
when the result is `Err`. Method form: `r.unwrap_or(default)`.
```rust
import "std/result" as result;

fn main() {
    let err = Err("boom");
    print(result::unwrap_or(err, 42));   // 42
    print(err.unwrap_or(42));            // 42
}
```

`Result` has no `map`/`map_err` helper the way [`Option`](option.md) has
`map`; transform a value out of a `Result` with `match` or after `unwrap`.

## Relation to `throw`

`unwrap` and `unwrap_err` raise their error with `throw`, so an unhandled
`Err.unwrap()` is a catchable error like any other; see
[Error handling](../language-tour/error-handling.md). This lets a function
return failure as data (`Err(reason)`) and defer the decision of whether to
raise it to the caller, which chooses between inspecting the result with
`is_ok`/`is_err`, falling back with `unwrap_or`, or calling `unwrap` to turn
an `Err` into a thrown, catchable error at the point that suits it.
