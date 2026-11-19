# A `for` over a map gives its entries

A `for` over a map used to bind each key, and reading a value meant `m[k]`
inside the loop. It now gives each entry, a key and a value, taken apart with
a pattern: `for (k, v) in m`. A lone name over a map is reported as
`map_iteration_takes_a_pattern`.

This affects you if a program loops over a map with `for k in m`.

Before:

```rust
for k in m {
    print(k, m[k]);
}
```

After:

```rust
for (k, v) in m {
    print(k, v);
}
for k in m.keys() {
    print(k);
}
```
