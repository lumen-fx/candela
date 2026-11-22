//! `std/websocket` from candela: `websocket::open` and the socket's `send`,
//! `close` and `state`,
//! against an echo server on the loopback interface.

use std::net::TcpListener;

use candela_std_testkit::{run, scratch, stage};

/// A server that echoes every text frame of one connection, and answers a
/// close with one. Answers its address.
fn echo_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut socket = tungstenite::accept(stream).expect("handshake");
        loop {
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    let echo = format!("echo {}", text.as_str());
                    if socket.send(tungstenite::Message::text(echo)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    format!("ws://{addr}")
}

/// A program whose `start` opens `url`, sends two frames once open, and
/// closes after the second echo.
fn program(url: &str) -> String {
    format!(
        r#"
import "std/websocket";

host "test" {{
    fn seen(text: string);
}}

fn start() {{
    let echoes = 0;
    let socket = websocket::open("{url}") fn(socket, e) {{
        match e {{
            Open => {{
                test::seen("open " + socket.state());
                socket.send("one");
                socket.send("two");
            }}
            Message(text) => {{
                test::seen(text);
                echoes += 1;
                if echoes == 2 {{
                    test::seen("closing " + string(socket.close(reason: "done").unwrap()));
                }}
            }}
            Error(why) => test::seen("error " + why),
            Close(code, reason, clean) => test::seen("close " + string(code)),
        }}
    }};
    test::seen("state " + socket.state());
}}

fn main() {{}}
"#,
    )
}

fn closed(seen: &[String]) -> bool {
    seen.iter().any(|s| s.starts_with("close "))
}

#[test]
fn frames_go_both_ways_and_the_close_is_last() {
    let dir = scratch("websocket-echo");
    let libs = stage(&["net"], &dir);
    let seen = run(&libs, &dir, &program(&echo_server()), closed);
    assert_eq!(
        seen,
        [
            "state connecting",
            "open open",
            "echo one",
            "echo two",
            "closing true",
            "close 1000",
        ],
        "{seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_connection_that_never_opens_reports_an_error_and_a_close() {
    let dir = scratch("websocket-refused");
    let libs = stage(&["net"], &dir);
    // A port nothing listens on: bound, then dropped.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let seen = run(
        &libs,
        &dir,
        &program(&format!("ws://127.0.0.1:{port}")),
        closed,
    );
    assert!(seen.iter().any(|s| s.starts_with("error ")), "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("close 1006"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_close_code_the_protocol_reserves_is_an_error() {
    let dir = scratch("websocket-bad-close");
    let libs = stage(&["net"], &dir);
    let program = r#"
import "std/websocket";

host "test" {
    fn seen(text: string);
}

fn start() {
    let socket = websocket::Socket { id: 1 };
    if socket.close(code: 1001) is Err(e) {
        test::seen("refused " + e);
    }
}

fn main() {}
"#;
    let seen = run(&libs, &dir, program, |seen| !seen.is_empty());
    assert_eq!(seen, ["refused websocket_bad_close"]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A server that goes away without a close frame ends the connection as a
/// failure: an error naming it, then a close that is not clean, with the code
/// that means no close frame came.
#[test]
fn a_server_that_drops_the_connection_is_a_failed_close() {
    let dir = scratch("websocket-dropped");
    let libs = stage(&["net"], &dir);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let socket = tungstenite::accept(stream).expect("handshake");
        drop(socket);
    });
    let program = format!(
        r#"
import "std/websocket";

host "test" {{
    fn seen(text: string);
}}

fn start() {{
    websocket::open("ws://{addr}") fn(socket, e) {{
        match e {{
            Open => test::seen("open"),
            Message(text) => test::seen(text),
            Error(why) => test::seen("error " + why),
            Close(code, reason, clean) => test::seen("close " + string(code) + " " + string(clean)),
        }}
    }};
}}

fn main() {{}}
"#,
    );
    let seen = run(&libs, &dir, &program, closed);
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert_eq!(seen[0], "open");
    assert!(
        seen[1].starts_with("error the connection failed: "),
        "{seen:?}"
    );
    assert_eq!(seen[2], "close 1006 false");
    std::fs::remove_dir_all(&dir).ok();
}
