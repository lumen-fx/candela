# websocket

WebSocket connections, text frames only, on the desktop and in a browser.

```rust
import "websocket";
```

```rust
import "websocket";

fn main() {
    let socket = 0;
    socket = websocket::open("wss://echo.example.com", fn(e) {
        match e {
            Open => { websocket::send(socket, "hello"); }
            Message(text) => {
                print(text);
                websocket::close(socket, 1000, "done");
            }
            Error(why) => print(why),
            Close(code, reason, clean) => print("closed " + string(code)),
        }
    });
}
```

On the desktop each connection runs on a thread of its own and `wss://` is TLS
with the Mozilla root set. In a browser the page's own WebSocket carries it,
through the package's JavaScript module. An `http://` or `https://` URL is
taken as `ws://` or `wss://`, as a page takes it. A binary frame is reported as
an `Error` rather than delivered.

## open

```rust
websocket::open(url, on_event)
```

- `on_event`: a `fn(websocket::Event)`, called for each event.
- Returns: a handle for `send`, `close` and `state`.

| `Event` variant | Meaning |
| --- | --- |
| `Open` | The connection is up |
| `Message(text)` | A text frame arrived |
| `Error(message)` | Something failed; a `Close` follows when the connection is over |
| `Close(code, reason, clean)` | The connection is over, with the close code, the reason, and whether the closing handshake finished cleanly. Always last |

A connection that never opens reports an `Error` and then `Close(1006, "",
false)`.

## send

```rust
websocket::send(handle, text)
```

- Returns: false when the connection is not open.

## close

```rust
websocket::close(handle, code, reason)
```

Starts the closing handshake. The `Close` event follows once it is over.

- `code`: 1000 for a normal close, or 3000 to 4999 for one of your own.
- `reason`: at most 123 bytes.
- Returns: false when there is no connection to close.
- Raises: `websocket_bad_close` for another code or a longer reason.

## state

```rust
websocket::state(handle)
```

- Returns: `connecting`, `open`, `closing` or `closed`, which is also the
  answer for a handle with no connection.
