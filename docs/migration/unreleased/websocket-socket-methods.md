# A WebSocket connection is a `websocket::Socket` with methods

`websocket::open` answers a `websocket::Socket`, and the closure it calls
receives that socket with each event, so a handler sends and closes through
it. `websocket::send(handle, text)`, `websocket::close(handle, code, reason)`
and `websocket::state(handle)` are `socket.send(text)`,
`socket.close(code, reason)` and `socket.state()`. `close` takes its code and
reason as named arguments with the defaults 1000 and `""`.

This affects you if a program opens a WebSocket.

Before:

```rust
let socket = 0;
socket = websocket::open(url, fn(e) {
    match e {
        Open => { websocket::send(socket, "hello"); }
        Message(text) => { websocket::close(socket, 1000, "done"); }
        _ => {}
    }
});
```

After:

```rust
websocket::open(url) fn(socket, e) {
    match e {
        Open => { socket.send("hello"); }
        Message(text) => { socket.close(reason: "done"); }
        _ => {}
    }
};
```
