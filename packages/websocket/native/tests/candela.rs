//! The package from candela: `websocket::open`, `send`, `close` and `state`,
//! against an echo server on the loopback interface.

use std::net::TcpListener;
use std::path::Path;

use candela_package_testkit::{import, run, scratch, stage};

fn package() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

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
fn program(root: &Path, url: &str) -> String {
    format!(
        r#"
{import}

host "test" {{
    seen(string);
}}

fn start() {{
    let socket = 0;
    let echoes = 0;
    socket = websocket::open("{url}", fn(e) {{
        match e {{
            Open => {{
                test::seen("open " + websocket::state(socket));
                websocket::send(socket, "one");
                websocket::send(socket, "two");
            }}
            Message(text) => {{
                test::seen(text);
                echoes += 1;
                if echoes == 2 {{
                    test::seen("closing " + string(websocket::close(socket, 1000, "done")));
                }}
            }}
            Error(why) => test::seen("error " + why),
            Close(code, reason, clean) => test::seen("close " + string(code)),
        }}
    }});
    test::seen("state " + websocket::state(socket));
}}

fn main() {{}}
"#,
        import = import(root, "websocket"),
    )
}

fn closed(seen: &[String]) -> bool {
    seen.iter().any(|s| s.starts_with("close "))
}

#[test]
fn frames_go_both_ways_and_the_close_is_last() {
    let dir = scratch("websocket-echo");
    let root = stage(package(), "websocket", &dir);
    let seen = run(&dir, &program(&root, &echo_server()), closed);
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
    let root = stage(package(), "websocket", &dir);
    // A port nothing listens on: bound, then dropped.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let seen = run(
        &dir,
        &program(&root, &format!("ws://127.0.0.1:{port}")),
        closed,
    );
    assert!(seen.iter().any(|s| s.starts_with("error ")), "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("close 1006"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_close_code_the_protocol_reserves_raises() {
    let dir = scratch("websocket-bad-close");
    let root = stage(package(), "websocket", &dir);
    let program = format!(
        r#"
{import}

host "test" {{
    seen(string);
}}

fn start() {{
    try {{
        websocket::close(1, 1001, "");
    }} catch "websocket_bad_close" {{
        test::seen("refused");
    }}
}}

fn main() {{}}
"#,
        import = import(&root, "websocket"),
    );
    let seen = run(&dir, &program, |seen| !seen.is_empty());
    assert_eq!(seen, ["refused"]);
    std::fs::remove_dir_all(&dir).ok();
}
