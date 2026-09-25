# hash

md5, sha1 and sha256 digests of a string, each answered as lowercase hex.

```rust
import "std/hash" as hash;
```

A digest covers the string's UTF-8 bytes, so the same text hashes the same in
candela as it does in any other tool. Use md5 and sha1 for ids and checksums:
checking a download against a published sum, or deriving a stable id from a
name. They are not for security; collisions for both can be made on purpose.
Reach for sha256 when an attacker could choose the input.

A string that holds a NUL character cannot cross into the native library, so on
a desktop hashing one raises an error.

In the WebAssembly build the three digests are built in and give the same
answers as on a desktop.

The module binds a small dynamic library, so it is one of the four std modules
that need that library present at run time. A `.cdlb` built from a program that
imports `std/hash` records the binding by name and re-opens it when the
artifact runs; see [artifacts](../reference/artifacts.md).

## md5

```rust
hash::md5(s)
```

- `s`: a string.
- Returns: the md5 digest of `s`, as 32 lowercase hex digits.

```rust
import "std/hash" as hash;

fn main() {
    print(hash::md5("abc"));
}
```

## sha1

```rust
hash::sha1(s)
```

- `s`: a string.
- Returns: the sha1 digest of `s`, as 40 lowercase hex digits.

## sha256

```rust
hash::sha256(s)
```

- `s`: a string.
- Returns: the sha256 digest of `s`, as 64 lowercase hex digits.
