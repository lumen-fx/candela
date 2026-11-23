# websocket

WebSocket connections, text frames only, on the desktop and in a browser.

```rust
import "std/websocket";
```

```rust
import "std/websocket";

fn main() {
    websocket::open("wss://echo.example.com") fn(socket, e) {
        match e {
            Open => { socket.send("hello"); }
            Message(text) => {
                print(text);
                socket.close(reason: "done");
            }
            Error(why) => eprint(why),
            Close(code, reason, clean) => print("closed " + string(code)),
        }
    };
}
```

On the desktop each connection runs on a thread of its own and `wss://` is TLS
with the Mozilla root set, through the `net` native library it shares with
[http](http.md). In a browser the page's own WebSocket carries it, through
`libs/std/web/websocket.js`, which the host loads beside the page and binds to
the module's `host "websocket"` block. An `http://` or `https://` URL is taken
as `ws://` or `wss://`, as a page takes it. A binary frame is reported as an
`Error` rather than delivered.

## open

```rust
websocket::open(url, on_event)
```

- `on_event`: a `fn(websocket::Socket, websocket::Event)`, called for each
  event with the socket it happened on, so a handler can send and close
  through it. It is the last argument, so it can follow the call.
- Returns: a `websocket::Socket`.

| `Event` variant | Meaning |
| --- | --- |
| `Open` | The connection is up |
| `Message(text)` | A text frame arrived |
| `Error(message)` | Something failed; a `Close` follows when the connection is over |
| `Close(code, reason, clean)` | The connection is over, with the close code, the reason, and whether the closing handshake finished cleanly. Always last |

A connection that never opens reports an `Error` and then `Close(1006, "",
false)`.

## Socket

A connection `open` started. Its `id` is the event source its events arrive
under, for [`event::wait`](event.md#wait).

### send

```rust
socket.send(text)
```

- Returns: false when the connection is not open.

### close

```rust
socket.close(code = 1000, reason = "")
```

Starts the closing handshake. The `Close` event follows once it is over.

- `code`: 1000 for a normal close, or 3000 to 4999 for one of your own.
- `reason`: at most 123 bytes.
- Returns: a `bool!string`: `Ok(false)` when there is no connection to close,
  and `Err("websocket_bad_close")` for another code or a longer reason.

### state

```rust
socket.state()
```

- Returns: `connecting`, `open`, `closing` or `closed`, which is also the
  answer once the connection is over.
