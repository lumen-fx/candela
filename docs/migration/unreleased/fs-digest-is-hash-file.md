# `fs::digest` is now `hash::file`

Digests have one home, `std/hash`. `fs::digest(path, algorithm)` is gone; call
`hash::file(path, algorithm)` instead. It answers a `string!string` like the
other functions that read a file, and an algorithm other than `"md5"`,
`"sha1"` or `"sha256"` gives `Err("hash_unknown_algorithm")`, which was the
`fs_unknown_digest` error.

The string digests `hash::md5`, `hash::sha1` and `hash::sha256` now run inside
the runtime, so `std/hash` no longer binds a native library: a `.cdlb` that
imports it runs with no `libs/std_src/hash` beside `candela-vm`, and a string
holding a NUL hashes all of its bytes instead of raising.

This affects you if a program calls `fs::digest` or matches
`fs_unknown_digest`.

Before:

```rust
import "std/fs";

fn main() {
    print(fs::digest("pack.zip", "sha256")?);
}
```

After:

```rust
import "std/hash";

fn main() {
    print(hash::file("pack.zip", "sha256")?);
}
```
