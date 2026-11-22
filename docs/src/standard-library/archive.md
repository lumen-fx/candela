# archive

Unpack zip, tar, and gzip-compressed tar archives, write zip and
gzip-compressed tar ones, and gzip a single file, off your program's thread.

```rust
import "std/archive";
```

```rust
import "std/archive";

fn main() {
    archive::extract("natives.jar", "natives", include: ["*.so"]) fn(e) {
        match e {
            Done(dest, files) => print(string(files) + " files in " + dest),
            Failed(why) => eprint(why),
        }
    };
}
```

The bytes decide the format; the file name is consulted only when they say
nothing. What an archive may write is checked before anything is written: an
entry naming an absolute path, climbing out with `..`, or resolving outside the
destination fails the whole extraction, naming the entry. Symbolic and hard
links are skipped, because a link inside the destination can point outside it
once extraction is over.

The module binds the `archive` native library, which ships in the toolchain's
`libs/std_src/archive/`. A browser has no file system, so the module does not
run there.

## extract

```rust
archive::extract(src, dest, include = [], on_event)
```

- `dest`: the directory to unpack into; it is created, and a file already in
  the way is overwritten.
- `include`: glob patterns of the files to write; empty writes every file.
  `*` and `?` stay inside one path segment and `**` spans segments; a pattern
  without a `/` matches the file name at any depth.
- `on_event`: a `fn(archive::Event)`, called once. It is the last argument, so
  it can follow the call.
- Returns: an `archive::Job`, whose `id` is the event source for
  [`event::wait`](event.md#wait).

| `Event` variant | Meaning |
| --- | --- |
| `Done(dest, files)` | The archive is out; `files` counts the files written, not directories or skipped links |
| `Failed(message)` | It was not extracted, or not all of it; what earlier entries wrote stays on disk |

## create

```rust
archive::create(dest, files, base = "", on_event)
```

Writes `files` into a new archive at `dest`: a zip when `dest` ends in `.zip`,
a gzip-compressed tar when it ends in `.tar.gz` or `.tgz`; another name fails.
A directory among `files` goes in with everything under it, in name order.
Each entry is named by its path relative to `base`, the working directory when
empty, and a file outside `base` fails the job. `Done(dest, files)` counts the
files that went in. Returns an `archive::Job`.

```rust
import "std/archive";

fn main() {
    archive::create("site.zip", ["site"]) fn(e) { print(e); };
}
```

## gzip

```rust
archive::gzip(src, dest, on_event)
```

Compresses the file `src` with gzip into `dest`; `Done(dest, 1)` once it is
there. Returns an `archive::Job`.

## gunzip

```rust
archive::gunzip(src, dest, on_event)
```

Decompresses the gzip file `src` into `dest`; `Done(dest, 1)` once it is
there, and `Failed` naming `src` for input that is not gzip. Returns an
`archive::Job`.

## set_max_concurrent

```rust
archive::set_max_concurrent(n)
```

Caps how many jobs of any kind run at once, from 1 to 64; 4 until it is
called. A job started past the cap reports `Failed` at once.
