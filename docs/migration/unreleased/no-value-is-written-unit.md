# The type of no value is written ()

`()` is now the type of no value and its one value. A function that returns
nothing returns `()`, and a standard library call that succeeds with nothing
to hand back answers `Ok(())`, a `()!string`, where it answered `Ok(true)`, a
`bool!string`, earlier in this release: `fs::write`, `fs::delete`, `os::set_cwd`
and the rest, and the `on_done` of `browser::share` and
`browser::request_fullscreen`.

`type(x)` and compile errors name the type `()` where they said `null`:
`type(xs.reverse())` is `"()"`. At run time `()` is the same value as `null`,
so `print(())` still prints `null`, and a host sees `Value::Null`.

Before:

```rust
fn main() {
    if fs::write("a.txt", "hi") == Ok(true) {
        print("written");
    }
}
```

After:

```rust
fn main() {
    match fs::write("a.txt", "hi") {
        Ok(()) => print("written"),
        Err(e) => print(e),
    }
}
```
