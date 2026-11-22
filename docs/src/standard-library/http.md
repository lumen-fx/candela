# http

HTTP: requests answered in memory, and downloads of a URL to a file, with
progress, an optional checksum, and a destination that only ever holds a
complete file.

```rust
import "std/http";
```

```rust
import "std/http";

fn main() {
    let sum = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    http::download("https://example.com/hello.txt", "hello.txt", checksum: sum) fn(e) {
        match e {
            Received(received, Some(total)) => print(string(received) + " of " + string(total)),
            Received(received, None) => print(string(received)),
            Saved(path) => print("saved " + path),
            Failed(why) => eprint(why),
        }
    };
}
```

The bytes land in a temporary file beside the destination, and replace it
only once the whole body has arrived and the checksum, when there is one,
matched. Any failure removes the temporary file and leaves the destination as
it was. A reply with a status outside 2xx fails the transfer. Redirects are
followed, and `https` is TLS with the Mozilla root set.

The module binds the `net` native library, which ships in the toolchain's
`libs/std_src/net/` and also carries [websocket](websocket.md), so the two
share one TLS stack. In a browser `request` runs on the page's `fetch`,
through `libs/std/web/http.js`, which the host binds to the module's
`host "http"` block, and the page's rules on other origins apply. A browser
has no file to download to, so there `download` raises `http_desktop_only`.

## request

```rust
http::request(url, method = "GET", headers = {}, body = "", timeout_ms = 0, max_bytes = 0, on_done)
```

- `method`: `GET`, `POST`, `PUT` and the rest, in either case.
- `headers`: a `{string: string}` sent as given.
- `body`: text sent as UTF-8.
- `timeout_ms`: how long each wait for the server lasts; 0 waits
  indefinitely.
- `max_bytes`: the largest body read; 0 is 64 MiB.
- `on_done`: a `fn(http::Response!string)`, called once: `Ok` with the
  response, whatever its status, or `Err` when there was none, such as a
  refused connection, a timeout, or `cancelled`.
- Returns: an `http::Transfer`.

A `Response` has `status`, an `int`; `headers`, a `{string: string}` by
lowercase name, with the values of a repeated header joined by `, `; and
`body`, the reply as text.

```rust
import "std/http";
import "std/json";

fn main() {
    http::request("https://example.com/api", method: "POST", body: json::stringify({"q": 1})) fn(r) {
        match r {
            Ok(reply) => print(string(reply.status) + " " + reply.body),
            Err(why) => eprint(why),
        }
    };
}
```

## download

```rust
http::download(url, path, checksum = "", timeout_ms = 0, max_bytes = 0, on_event)
```

- `path`: where the file goes; missing directories on the way are created.
- `checksum`: `sha256:<64 hex digits>`, `sha1:<40 hex digits>`, or a bare
  digest whose length names the algorithm; empty checks nothing.
- `timeout_ms`: how long a quiet server is waited on, for the reply to start
  and then for each next piece of the body; 0 waits indefinitely. A body that
  keeps arriving is never cut off.
- `max_bytes`: the largest body accepted; 0 accepts any size.
- `on_event`: a `fn(http::Progress)`, called for each event. It is the last
  argument, so it can follow the call.
- Returns: an `http::Transfer`.

| `Progress` variant | Meaning |
| --- | --- |
| `Received(received, total)` | Bytes so far, and the size the server declared as an `int?`: `None` when it declared none. At most every 100 ms, and once more at the end |
| `Saved(path)` | The file is in place |
| `Failed(message)` | The transfer did not happen; `cancelled` after a `cancel` |

`Saved` or `Failed` is always the last event.

## Transfer

The transfer `request` or `download` started. Its `id` is the event source
its events arrive under: `event::wait(transfer.id)` runs handlers until it is over; see
[event](event.md#wait).

### cancel

```rust
transfer.cancel()
```

Stops the transfer. A download then reports `Failed("cancelled")` as its last
event, and a request answers `Err("cancelled")`.

- Returns: false when it had already finished, whose own outcome is on its
  way.

## set_max_concurrent

```rust
http::set_max_concurrent(n)
```

Caps how many transfers, requests and downloads together, run at once, from 1
to 64; 4 until it is called. One started past the cap fails at once. A browser
keeps its own limits, so there this does nothing.
