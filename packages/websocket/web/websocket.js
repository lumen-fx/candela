// The browser half of the `websocket` package: the page's own WebSocket,
// answering the functions the package's `host "websocket"` block declares.
//
// The embedder calls `install` once with the program's event queue, then
// binds each exported function to the host function of the same name. Every
// event goes to the program as the JSON object the desktop half posts, so one
// candela decoder reads both.

let candela;
const sockets = new Map();
const STATES = ["connecting", "open", "closing", "closed"];

export function install(runtime) {
  candela = runtime;
}

function post(source, event) {
  candela.post(source, JSON.stringify(event));
}

export function candela_websocket_open(source, url) {
  candela.hold(source);
  let socket;
  try {
    // A relative URL means the site's own server, reached over the socket
    // scheme that matches the page's.
    const resolved = new URL(url, location.href);
    if (resolved.protocol === "http:") {
      resolved.protocol = "ws:";
    } else if (resolved.protocol === "https:") {
      resolved.protocol = "wss:";
    }
    socket = new WebSocket(resolved.href);
  } catch (error) {
    post(source, { kind: "error", message: String(error) });
    post(source, { kind: "close", code: 1006, reason: "", clean: false });
    candela.release(source);
    return;
  }
  socket.binaryType = "arraybuffer";
  sockets.set(source, socket);
  socket.addEventListener("open", () => post(source, { kind: "open" }));
  socket.addEventListener("message", (event) => {
    if (typeof event.data === "string") {
      post(source, { kind: "message", text: event.data });
    } else {
      post(source, {
        kind: "error",
        message: "a binary frame arrived; this package delivers text frames only",
      });
    }
  });
  socket.addEventListener("error", () =>
    post(source, { kind: "error", message: "the connection failed" }),
  );
  socket.addEventListener("close", (event) => {
    sockets.delete(source);
    post(source, {
      kind: "close",
      code: event.code,
      reason: event.reason,
      clean: event.wasClean,
    });
    candela.release(source);
  });
}

export function candela_websocket_send(source, text) {
  const socket = sockets.get(source);
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    return 0;
  }
  socket.send(text);
  return 1;
}

export function candela_websocket_close(source, code, reason) {
  if (!(code === 1000 || (code >= 3000 && code <= 4999))) {
    return -1;
  }
  if (new TextEncoder().encode(reason).length > 123) {
    return -1;
  }
  const socket = sockets.get(source);
  if (!socket || socket.readyState >= WebSocket.CLOSING) {
    return 0;
  }
  socket.close(code, reason);
  return 1;
}

export function candela_websocket_state(source) {
  const socket = sockets.get(source);
  return socket ? STATES[socket.readyState] : "closed";
}
