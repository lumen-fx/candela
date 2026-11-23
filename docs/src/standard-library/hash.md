# hash

md5, sha1 and sha256 digests of a string or a file, each answered as
lowercase hex.

```rust
import "std/hash";
```

A string's digest covers its UTF-8 bytes, so the same text hashes the same in
candela as it does in any other tool. Use md5 and sha1 for ids and checksums:
checking a download against a published sum, or deriving a stable id from a
name. They are not for security; collisions for both can be made on purpose.
Reach for sha256 when an attacker could choose the input.

The digests run inside the runtime, so the module needs no native library and
gives the same answers in a `.cdlb` under `candela-vm` and in a browser.

## md5

```rust
hash::md5(s)
```

- `s`: a string.
- Returns: the md5 digest of `s`, as 32 lowercase hex digits.

```rust
import "std/hash";

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

## file

```rust
hash::file(path, algorithm)
```

- `algorithm`: `"md5"`, `"sha1"` or `"sha256"`.
- Returns: the digest of the file at `path`, as lowercase hex, in a
  `string!string`. A file that cannot be read gives its `fs_*` error, and
  another algorithm gives `Err("hash_unknown_algorithm")`.

The file streams through the digest, so a large one never sits in memory
whole. A browser has no file system, so there this answers an error.

```rust
import "std/hash";

fn main() {
    print(hash::file("notes.txt", "sha256") else "no notes.txt");
}
```
