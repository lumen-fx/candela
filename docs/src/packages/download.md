# download

Download a URL to a file, with progress, an optional checksum, and a
destination that only ever holds a complete file.

```rust
import "download";
```

```rust
import "download";

fn main() {
    let options = download::Options { checksum: "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", .. };
    download::to_file("https://example.com/hello.txt", "hello.txt", options, fn(e) {
        match e {
            Progress(received, total) => print(string(received) + " of " + string(total)),
            Done(path) => print("saved " + path),
            Failed(why) => print(why),
        }
    });
}
```

The bytes land in a temporary file beside the destination, and replace it
only once the whole body has arrived and the checksum, when there is one,
matched. Any failure removes the temporary file and leaves the destination as
it was. A reply with a status outside 2xx fails the transfer. Redirects are
followed, and `https` is TLS with the Mozilla root set.

## to_file

```rust
download::to_file(url, path, options, on_event)
```

- `path`: where the file goes; missing directories on the way are created.
- `options`: a `download::Options`; `download::Options { .. }` takes every
  default.
- `on_event`: a `fn(download::Event)`, called for each event.
- Returns: a handle for `cancel`.

| `Options` field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `checksum` | string | `""` | `sha256:<64 hex digits>`, `sha1:<40 hex digits>`, or a bare digest whose length names the algorithm; empty checks nothing |
| `timeout_ms` | int | `0` | How long a quiet server is waited on, for the reply to start and then for each next piece of the body; 0 waits indefinitely. A body that keeps arriving is never cut off |
| `max_bytes` | int | `0` | The largest body accepted; 0 accepts any size |

| `Event` variant | Meaning |
| --- | --- |
| `Progress(received, total)` | Bytes so far, and the size the server declared, or -1 when it declared none. At most every 100 ms, and once more at the end |
| `Done(path)` | The file is in place |
| `Failed(message)` | The transfer did not happen; `cancelled` after a `cancel` |

`Done` or `Failed` is always the last event.

## cancel

```rust
download::cancel(handle)
```

Stops the transfer. It then reports `Failed("cancelled")` as its last event.

- Returns: false when it had already finished, whose own outcome is on its
  way.

## set_max_concurrent

```rust
download::set_max_concurrent(n)
```

Caps how many transfers run at once, from 1 to 64; 4 until it is called. A
transfer started past the cap reports `Failed` at once.
