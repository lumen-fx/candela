# `Default::default()` is removed; a struct literal ends in a bare `..` instead

`Default::default()` is gone in every position it used to resolve in: after
`..` in a struct literal, as an argument to a struct-typed parameter, in a
`return`, and as the value of a struct-typed field. A literal that ends in a
bare `..` takes every field it does not write from the struct's default, and
`S::default()` builds the whole default as before. Calling
`Default::default()` is a compile error, `unknown_function`, whose help names
`S { .. }` and `S::default()`.

This affects you if a program writes `Default::default()`.

Before:

```rust
struct Options {
    cwd: string = ".",
    verbose: bool,
}

fn run(opts: Options) -> string {
    return opts.cwd;
}

fn main() {
    let o = Options { verbose: true, ..Default::default() };
    print(run(Default::default()));
}
```

After:

```rust
struct Options {
    cwd: string = ".",
    verbose: bool,
}

fn run(opts: Options) -> string {
    return opts.cwd;
}

fn main() {
    let o = Options { verbose: true, .. };
    print(run(Options::default()));
}
```
