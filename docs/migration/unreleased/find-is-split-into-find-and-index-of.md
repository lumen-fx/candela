# `find` is split: `find(pred)` searches with a predicate, `index_of(x)` finds a position

`xs.find(x)` and `s.find(needle)` were the index search and answered `-1` when
nothing matched; `xs.find(f)` with a function was the list module's predicate
search and answered `null`. Each operation now has one name, and neither has a
sentinel:

- `xs.index_of(x)` and `s.index_of(needle)` answer `Some` of the position, or
  `None`.
- `xs.find(f)` answers `Some` of the first element `f` accepts, or `None`.

`find` on a list with an argument that is not a function is a compile error,
`find_takes_a_function`, whose help names `index_of`. `find` on a string is
`no_such_method`, whose help names `index_of`.

This affects you if a program calls `find`, compares a position with `-1`, or
calls the list module's `index_of`, which also answered `-1`.

Before:

```rust
fn main() {
    let xs = [10, 20, 30];
    let i = xs.find(20);
    if i != -1 {
        print(i);
    }
    print("hello".find("l"));
    print(xs.find(fn(x) => x > 15));
}
```

After:

```rust
fn main() {
    let xs = [10, 20, 30];
    if xs.index_of(20) is Some(i) {
        print(i);
    }
    print("hello".index_of("l").unwrap_or(-1));
    print(xs.find(fn(x) => x > 15).unwrap());
}
```
