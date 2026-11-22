# fs

Read, write, copy and delete files, digest them, and list and make
directories.

```rust
import "std/fs";
```

Every function takes the path as a string. One that can fail answers a
[`Result`](result.md) whose error names the cause: `fs_not_found`,
`fs_permission_denied`, `fs_is_a_directory`, `fs_storage_full`, and the rest,
listed in [the error catalogue](../reference/errors.md). One that has nothing
to give back answers `Ok(true)`, a `bool!string`. Pass a failure on with `?`,
give a fallback with `else`, or test for it with `is Err(e)`; see
[error handling](../language/error-handling.md).

A relative path resolves against the working directory. A Rust host that runs
a script on behalf of an application can name another directory for it to
resolve against instead; see [embedding](../integration/embedding.md#the-base-directory).

```rust
import "std/fs";

fn main() {
    fs::write("notes.txt", "one\n")?;
    fs::append("notes.txt", "two\n")?;
    print(fs::read("notes.txt")?);
    fs::delete("notes.txt")?;
    print(fs::exists("notes.txt"));
    if fs::read("missing.txt") is Err(e) {
        print(e);
    }
}
```

The module is pure candela over the runtime's own file primitives, so it
compiles into a `.cdlb` artifact and runs under `candela-vm` with no dynamic
library. A browser has no file system, so it does not import there.

## read

```rust
fs::read(path)
```

- Returns: the whole file at `path`, as a `string!string`.

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
does not exist. Returns `Ok(true)`.

## append

```rust
fs::append(path, contents)
```

Appends `contents` to the end of the file at `path`. Creates the file when it
does not exist. Returns `Ok(true)`.

## delete

```rust
fs::delete(path)
```

Deletes the file at `path`. Returns `Ok(true)`, or an `Err` when the path does
not exist or names a directory.

## delete_dir

```rust
fs::delete_dir(path)
```

Deletes the empty directory at `path`. Returns `Ok(true)`, or an `Err` when the
directory is missing or still has entries.

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
  sorted, as a `string[]!string`. Names, not paths: join one onto `path` to reach the
  entry. The listing is one level deep.

## mkdir

```rust
fs::mkdir(path)
```

Creates the directory at `path` and every missing directory above it. A
directory that is already there is no error. Returns `Ok(true)`.

## copy

```rust
fs::copy(from, to)
```

Copies the file at `from` to `to`, replacing what was at `to`. Returns
`Ok(true)`, or `Err("fs_is_a_directory")` when `from` is a directory: copy the
files inside it.

## read_bytes

```rust
fs::read_bytes(path)
```

- Returns: the bytes of the file at `path`, as an `int[]!string` of values
  from 0 to 255.

## write_bytes

```rust
fs::write_bytes(path, bytes)
```

Writes `bytes` to `path`, replacing what was there. Every element has to be a
byte, 0 to 255; an element that is not gives `Err("fs_not_a_byte")` and
nothing is written. Returns `Ok(true)`.

## rename

```rust
fs::rename(from, to)
```

Moves the file or directory at `from` to `to`, replacing a file at `to`.
Returns `Ok(true)`.

## size

```rust
fs::size(path)
```

- Returns: the size of the file at `path` in bytes, as an `int!string`.

## modified

```rust
fs::modified(path)
```

- Returns: when the file at `path` last changed, in seconds since the unix
  epoch, as an `int!string`. Compare it with [`time::now`](time.md).

## delete_all

```rust
fs::delete_all(path)
```

Deletes the file at `path`, or the directory there with everything in it.
Where `delete` and `delete_dir` refuse a directory that still has entries,
this does not, so check the path before you call it. Returns `Ok(true)`.

## absolute

```rust
fs::absolute(path)
```

- Returns: `path` as an absolute path, in a `string!string`, joined onto the
  working directory or the base directory the host set when it is relative.
  `..` segments and links stay as written, and nothing needs to exist there;
  [`path::normalize`](path.md#normalize) tidies the segments.
