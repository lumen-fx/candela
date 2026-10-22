# archive

Unpack zip, tar, and gzip-compressed tar archives into a directory, off your
program's thread.

```rust
import "archive";
```

```rust
import "archive";

fn main() {
    archive::extract("natives.jar", "natives", archive::Options { include: ["*.so"] }, fn(e) {
        match e {
            Done(dest, files) => print(string(files) + " files in " + dest),
            Failed(why) => print(why),
        }
    });
}
```

The bytes decide the format; the file name is consulted only when they say
nothing. What an archive may write is checked before anything is written: an
entry naming an absolute path, climbing out with `..`, or resolving outside the
destination fails the whole extraction, naming the entry. Symbolic and hard
links are skipped, because a link inside the destination can point outside it
once extraction is over.

## extract

```rust
archive::extract(src, dest, options, on_event)
```

- `dest`: the directory to unpack into; it is created, and a file already in
  the way is overwritten.
- `options`: an `archive::Options`; `archive::Options { .. }` keeps every
  file.
- `on_event`: a `fn(archive::Event)`, called once.
- Returns: a handle.

| `Options` field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `include` | `string[]` | `[]` | Glob patterns of the files to write; empty writes every file. `*` and `?` stay inside one path segment and `**` spans segments; a pattern without a `/` matches the file name at any depth |

| `Event` variant | Meaning |
| --- | --- |
| `Done(dest, files)` | The archive is out; `files` counts the files written, not directories or skipped links |
| `Failed(message)` | It was not extracted, or not all of it; what earlier entries wrote stays on disk |

## set_max_concurrent

```rust
archive::set_max_concurrent(n)
```

Caps how many extractions run at once, from 1 to 64; 4 until it is called. An
extraction started past the cap reports `Failed` at once.
