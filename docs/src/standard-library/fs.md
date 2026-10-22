# fs

Read, write, copy and delete files, digest them, and list and make
directories.

```rust
import "std/fs";
```

Every function takes the path as a string and raises a catchable error on
failure. The code names the cause: `fs_not_found`, `fs_permission_denied`,
`fs_is_a_directory`, `fs_storage_full`, and the rest, listed in
[the error catalogue](../reference/errors.md).

A relative path resolves against the working directory. A Rust host that runs
a script on behalf of an application can name another directory for it to
resolve against instead; see [embedding](../integration/embedding.md#the-base-directory).

```rust
import "std/fs";

fn main() {
    fs::write("notes.txt", "one\n");
    fs::append("notes.txt", "two\n");
    print(fs::read("notes.txt"));
    fs::delete("notes.txt");
    print(fs::exists("notes.txt"));
}
```

The module is pure candela over the runtime's own file primitives, so it
compiles into a `.cdlb` artifact and runs under `candela-vm` with no dynamic
library. A browser has no file system, so it does not import there.

## read

```rust
fs::read(path)
```

- Returns: the whole file at `path`, as a string.

## exists

```rust
fs::exists(path)
```

- Returns: true when something exists at `path`.

## write

```rust
fs::write(path, contents)
```

Writes `contents` to `path`, replacing what was there. Creates the file when it
does not exist. Returns nothing.

## append

```rust
fs::append(path, contents)
```

Appends `contents` to the end of the file at `path`. Creates the file when it
does not exist. Returns nothing.

## delete

```rust
fs::delete(path)
```

Deletes the file at `path`. Raises when the path does not exist or names a
directory. Returns nothing.

## delete_dir

```rust
fs::delete_dir(path)
```

Deletes the empty directory at `path`. Raises when the directory is missing or
still has entries. Returns nothing.

## is_dir

```rust
fs::is_dir(path)
```

- Returns: true when `path` names a directory. A file or a missing path is
  false.

## list

```rust
fs::list(path)
```

- Returns: the names of the entries directly inside the directory at `path`,
  sorted, as a `string[]`. Names, not paths: join one onto `path` to reach the
  entry. The listing is one level deep.

## mkdir

```rust
fs::mkdir(path)
```

Creates the directory at `path` and every missing directory above it. A
directory that is already there is no error. Returns nothing.

## copy

```rust
fs::copy(from, to)
```

Copies the file at `from` to `to`, replacing what was at `to`. Raises
`fs_is_a_directory` when `from` is a directory: copy the files inside it.
Returns nothing.

## read_bytes

```rust
fs::read_bytes(path)
```

- Returns: the bytes of the file at `path`, as an `int[]` of values from 0 to
  255.

## write_bytes

```rust
fs::write_bytes(path, bytes)
```

Writes `bytes` to `path`, replacing what was there. Every element has to be a
byte, 0 to 255; an element that is not raises `fs_not_a_byte` and nothing is
written. Returns nothing.

## digest

```rust
fs::digest(path, algorithm)
```

- `algorithm`: `"md5"`, `"sha1"` or `"sha256"`.
- Returns: the digest of the file at `path`, as lowercase hex.
- Raises: `fs_unknown_digest` for another algorithm.

The file streams through the digest, so a large one never sits in memory
whole. md5 and sha1 are for ids and checksums, not for security. To digest a
string, use [hash](hash.md).
