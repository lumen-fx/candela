# fs

Read, write, append to and delete files.

```rust
import "std/fs" as fs;
```

Every function takes the path as a string and raises a catchable error on
failure. The code names the cause: `fs_not_found`, `fs_permission_denied`,
`fs_is_a_directory`, `fs_storage_full`, and the rest, listed in
[the error catalogue](../reference/errors.md).

```rust
import "std/fs" as fs;

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
