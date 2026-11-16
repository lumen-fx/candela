# `Engine::compile` checks imported files, and a host error can carry a kind

`Engine::compile` now compiles every annotated function of the files the
script imports, as it already did for the script itself. An error in the body
of such a function comes back from `compile`, with its file and place, instead
of from the first `Program::call` that reaches it. A program that compiled
before while an imported file held a broken function nothing called no longer
compiles; fix the function or remove it. Functions of a library (the standard
library, a package, an import root) are still compiled by the call that
reaches them. `candela build` is unchanged.

`HostError::with_kind(kind, message)` raises an error a script catches as
`kind` rather than `host_fn_error`. `ErrType::HostFn` carries that kind as a
third field:

Before:

```rust
ErrType::HostFn(function, message) => ...
```

After:

```rust
ErrType::HostFn(function, message, kind) => ...
```

An error whose body was compiled for a call that gave untyped parameters
their types lists that call in `Diagnostic::related`.
