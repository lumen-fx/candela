---
icon: lucide/circle-question-mark
---
# Option library

The `Option` type: a value that is either present (`Some`) or absent (`None`).
Import the library with `import "std/option";` at the top-level.

*[at the top-level]: Outside of any function.

Reach for `Option` when a value may legitimately be missing and you want the
type to say so, instead of standing in a sentinel such as `-1`, an empty
string, or `null` that the caller has to know to check for.

`Option` is an ordinary enum, so `Some(x)` and `None` are constructed directly
and matched with payload binding, the same as any other enum:

```rust
import "std/option";

fn main() {
    let o = Some(5);
    match o {
        Some(v) => { print(v); }
        None => { print("nothing"); }
    }
}
```

The payload of `Some` is typed `any`, so it can hold a value of any type; a
function built on it, such as `map`, works through type-agnostic operations
(`str`, another `Option`-returning function, ...) rather than arithmetic that
needs a concrete type.

The `Option` enum and the `Some`/`None` constructors are always reachable
unqualified, even through a namespaced import (`import "std/option" as
option;`); only the free functions below sit behind the `option::` alias.

Every function below works both as a free function (`unwrap(o)`, with a bare
import) and as a method on the value (`o.unwrap()`), since `option.cdl` also
defines an `impl Option` block; see
[Methods (`impl` blocks)](../language-tour/data-types.md#methods-impl-blocks).

This module is written in Candela and uses no dynamic library, so a program
that imports it builds to a `.cdlb` artifact that runs under `candela-vm` with
the module bytecode inlined.

### Is some
`is_some(o: Option) -> bool`: true when the option holds a value. Method form:
`o.is_some()`.
### Is none
`is_none(o: Option) -> bool`: true when the option is empty. Method form:
`o.is_none()`.
### Unwrap
`unwrap(o: Option) -> any`: the contained value, or raises when the option is
`None`. Method form: `o.unwrap()`.
```rust
import "std/option" as option;

fn main() {
    let s = Some(5);
    print(option::unwrap(s));   // 5
    print(s.unwrap());          // 5, same call as a method
}
```
### Unwrap or
`unwrap_or(o: Option, default: any) -> any`: the contained value, or `default`
when the option is `None`. Method form: `o.unwrap_or(default)`.
```rust
import "std/option" as option;

fn main() {
    let n = None;
    print(option::unwrap_or(n, 99));   // 99
    print(n.unwrap_or(99));            // 99
}
```
### Map
`map(o: Option, f) -> Option`: applies `f` to the contained value and returns
a new option holding the result, or `None` when the option was already
`None`. `f` takes one argument. Method form: `o.map(f)`.
```rust
import "std/option" as option;

fn describe(x) { return str(x); }

fn main() {
    let m = option::map(Some(5), describe);
    print(option::unwrap(m));          // "5"
    print(option::is_none(option::map(None, describe))); // true
}
```

## Relation to `throw`

`unwrap` and `unwrap_or` raise their error with `throw`, so an unhandled
`None.unwrap()` is a catchable error like any other; see
[Error handling](../language-tour/error-handling.md). `Option` itself does not
carry a reason for the absence, only that a value is missing. Use
[`Result`](result.md) when the caller needs to know why an operation did not
produce a value.
