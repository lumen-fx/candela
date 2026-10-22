//! One connection: the thread that owns its socket, the state a script reads,
//! and the queue a script's sends and closes go through.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

/// How long a connection has to be established and to finish its handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a read waits before the thread looks at what the script queued.
/// This is the most a send waits behind a quiet socket.
const POLL: Duration = Duration::from_millis(10);

/// What a connection is doing, with the names a page's `readyState` has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Connecting = 0,
    Open = 1,
    Closing = 2,
    Closed = 3,
}

impl State {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Open => "open",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Connecting,
            1 => Self::Open,
            2 => Self::Closing,
            _ => Self::Closed,
        }
    }
}

/// What a script asked a connection to do.
pub enum Outgoing {
    Text(String),
    Close(u16, String),
}

/// A script's handle on one connection.
pub struct Connection {
    state: Arc<AtomicU8>,
    outbox: Sender<Outgoing>,
}

impl Connection {
    #[must_use]
    pub fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    pub fn set_state(&self, state: State) {
        self.state.store(state as u8, Ordering::Release);
    }

    /// Queue `out` for the connection's thread. False when the thread is gone.
    pub fn push(&self, out: Outgoing) -> bool {
        self.outbox.send(out).is_ok()
    }
}

/// The URL a socket opens: `ws://` and `wss://` as they are, `http://` and
/// `https://` taken as the socket scheme that matches, as a page takes them.
///
/// # Errors
///
/// A URL with any other scheme, which no socket opens.
pub fn socket_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("ws://") || lower.starts_with("wss://") {
        Ok(url.to_string())
    } else if lower.starts_with("http://") {
        Ok(format!("ws://{}", &url["http://".len()..]))
    } else if lower.starts_with("https://") {
        Ok(format!("wss://{}", &url["https://".len()..]))
    } else {
        Err(format!(
            "`{url}` is not a ws://, wss://, http:// or https:// URL"
        ))
    }
}

/// What a connection's thread reports: called with each event's JSON payload,
/// and with `None` once, after the last, when the connection is over.
pub type Report = Arc<dyn Fn(Option<serde_json::Value>) + Send + Sync>;

/// Start a connection to `url` on a thread of its own.
///
/// # Errors
///
/// When there is no thread to run it on.
pub fn start(url: String, report: Report) -> Result<Connection, String> {
    let state = Arc::new(AtomicU8::new(State::Connecting as u8));
    let (outbox, inbox) = channel();
    let thread_state = Arc::clone(&state);
    std::thread::Builder::new()
        .name("candela-websocket".into())
        .spawn(move || {
            let events = Events {
                report,
                state: thread_state,
            };
            run(&events, &url, &inbox);
            events.set(State::Closed);
            (events.report)(None);
        })
        .map_err(|e| format!("no thread to connect on: {e}"))?;
    Ok(Connection { state, outbox })
}

/// What a connection's thread reports through.
struct Events {
    report: Report,
    state: Arc<AtomicU8>,
}

impl Events {
    fn set(&self, state: State) {
        self.state.store(state as u8, Ordering::Release);
    }

    fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    fn emit(&self, payload: serde_json::Value) {
        (self.report)(Some(payload));
    }

    fn error(&self, message: String) {
        self.emit(serde_json::json!({"kind": "error", "message": message}));
    }

    fn closed(&self, code: u16, reason: String, clean: bool) {
        self.set(State::Closed);
        self.emit(serde_json::json!({
            "kind": "close",
            "code": code,
            "reason": reason,
            "clean": clean,
        }));
    }
}

/// Connect, then carry frames both ways until the connection ends.
fn run(events: &Events, url: &str, inbox: &Receiver<Outgoing>) {
    let mut socket = match connect(url) {
        Ok(socket) => socket,
        Err(message) => {
            events.error(format!("the connection failed: {message}"));
            events.closed(1006, String::new(), false);
            return;
        }
    };
    if events.state() == State::Connecting {
        events.set(State::Open);
    }
    events.emit(serde_json::json!({"kind": "open"}));

    // The close frame the other side sent, which is what the close reports.
    let mut received: Option<(u16, String)> = None;
    loop {
        loop {
            match inbox.try_recv() {
                Ok(Outgoing::Text(text)) => {
                    if let Err(e) = socket.send(Message::text(text)) {
                        events.error(format!("the send failed: {e}"));
                    }
                }
                Ok(Outgoing::Close(code, reason)) => {
                    let frame = CloseFrame {
                        code: CloseCode::from(code),
                        reason: reason.into(),
                    };
                    let _ = socket.close(Some(frame));
                }
                // Every handle to the connection is gone, which only happens
                // once the program is shutting down.
                Err(TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                events.emit(serde_json::json!({"kind": "message", "text": text.as_str()}));
            }
            Ok(Message::Binary(_)) => events.error(
                "a binary frame arrived; this package delivers text frames only".to_string(),
            ),
            Ok(Message::Close(frame)) => {
                events.set(State::Closing);
                received = Some(frame.map_or((1005, String::new()), |f| {
                    (u16::from(f.code), f.reason.to_string())
                }));
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                let (code, reason) = received.unwrap_or((1005, String::new()));
                events.closed(code, reason, true);
                return;
            }
            Err(e) => {
                events.error(format!("the connection failed: {e}"));
                events.closed(1006, String::new(), false);
                return;
            }
        }
    }
}

/// Open the TCP connection, TLS when the URL says so, and do the handshake.
fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let request = url.into_client_request().map_err(|e| e.to_string())?;
    let uri = request.uri();
    let secure = uri.scheme_str() == Some("wss");
    let host = uri
        .host()
        .ok_or_else(|| format!("`{url}` names no host"))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let addrs = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?;
    let mut last = format!("{host} resolves to no address");
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = format!("{addr}: {e}"),
        }
    }
    let stream = stream.ok_or(last)?;
    let _ = stream.set_nodelay(true);
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let connector = if secure {
        Connector::Rustls(tls_config()?)
    } else {
        Connector::Plain
    };
    let (socket, _response) =
        tungstenite::client_tls_with_config(request, stream, None, Some(connector)).map_err(
            |e| match e {
                tungstenite::HandshakeError::Interrupted(_) => {
                    "the handshake did not finish in time".to_string()
                }
                tungstenite::HandshakeError::Failure(e) => e.to_string(),
            },
        )?;
    let tcp = match socket.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => &s.sock,
        _ => {
            return Err("the connection came back in a form this package cannot poll".to_string());
        }
    };
    tcp.set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    Ok(socket)
}

/// The TLS configuration every `wss://` connection shares: rustls with the
/// ring provider and the Mozilla root set.
fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map(|builder| Arc::new(builder.with_root_certificates(roots).with_no_client_auth()))
            .map_err(|e| format!("TLS: {e}"))
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_scheme_is_taken_as_the_socket_scheme() {
        assert_eq!(socket_url("ws://a/b").unwrap(), "ws://a/b");
        assert_eq!(socket_url("WSS://a").unwrap(), "WSS://a");
        assert_eq!(socket_url("http://a:1/x").unwrap(), "ws://a:1/x");
        assert_eq!(socket_url("https://a").unwrap(), "wss://a");
        assert!(socket_url("/relative").is_err());
        assert!(socket_url("ftp://a").is_err());
    }

    #[test]
    fn the_tls_configuration_builds() {
        assert!(tls_config().is_ok());
    }
}
