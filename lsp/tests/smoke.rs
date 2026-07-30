//! Headless stdio smoke test for `candela-lsp`.
//!
//! Spawns the built `candela-lsp` binary as a real subprocess (no editor, no
//! window, just the LSP JSON-RPC protocol over stdin/stdout, per this
//! repo's "verify headless" convention), sends `initialize` + `initialized`,
//! then `textDocument/didOpen` with a deliberately broken `.cdl` snippet, and
//! asserts a non-empty `textDocument/publishDiagnostics` notification comes
//! back. This is the one piece of behavior the whole crate exists for: a
//! real parse/type error in the buffer must surface as a live diagnostic.

use serde_json::{Value, json};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(10);

async fn write_message(writer: &mut (impl AsyncWriteExt + Unpin), msg: &Value) {
    let body = serde_json::to_string(msg).expect("serialize LSP message");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer
        .write_all(header.as_bytes())
        .await
        .expect("write LSP header");
    writer
        .write_all(body.as_bytes())
        .await
        .expect("write LSP body");
    writer.flush().await.expect("flush stdin");
}

/// Reads one `Content-Length`-framed LSP message, panicking (via the
/// caller's `timeout`) if the server never sends a full one.
async fn read_message(reader: &mut (impl AsyncBufReadExt + Unpin)) -> Value {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .expect("read LSP header line");
        assert_ne!(
            n, 0,
            "candela-lsp closed stdout before sending a full message"
        );
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(v.trim().parse::<usize>().expect("valid Content-Length"));
        }
    }
    let content_length = content_length.expect("response header included Content-Length");
    let mut buf = vec![0u8; content_length];
    reader.read_exact(&mut buf).await.expect("read LSP body");
    serde_json::from_slice(&buf).expect("LSP body is valid JSON")
}

/// Sends the standard LSP `shutdown` request followed by an `exit`
/// notification, and asserts the server process terminates.
///
/// `shutdown`/`exit` take no parameters in the LSP spec; tower-lsp's
/// JSON-RPC layer rejects a literal `"params": null` for them (it wants the
/// field omitted, not present-and-null).
///
/// tower-lsp's own read loop (`Server::serve`) does not proactively hang up
/// on an `exit` notification; like a real client, it relies on the pipe
/// being closed to end the read loop and let `serve().await` (and thus
/// `main()`) return. This takes `stdin` by value and drops it (closing the
/// write half, so the child sees EOF) right after sending `exit`, exactly
/// as an editor closing the stream would.
async fn shutdown_and_exit(
    child: &mut Child,
    mut stdin: tokio::process::ChildStdin,
    stdout: &mut BufReader<tokio::process::ChildStdout>,
) {
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
    )
    .await;
    // Skip any notifications (e.g. window/logMessage) until the response
    // matching this request's id arrives.
    for _ in 0..20 {
        let msg = timeout(TIMEOUT, read_message(stdout))
            .await
            .expect("timed out waiting for the shutdown response");
        if msg["id"] == 2 {
            break;
        }
    }
    write_message(&mut stdin, &json!({ "jsonrpc": "2.0", "method": "exit" })).await;
    drop(stdin); // close the pipe so the server's stdin-read loop sees EOF.
    let status = timeout(TIMEOUT, child.wait())
        .await
        .expect("timed out waiting for candela-lsp to exit after the `exit` notification")
        .expect("failed to wait on candela-lsp process");
    assert!(status.success(), "candela-lsp exited non-zero: {status:?}");
}

async fn spawn_server() -> (
    Child,
    tokio::process::ChildStdin,
    BufReader<tokio::process::ChildStdout>,
) {
    let exe = env!("CARGO_BIN_EXE_candela-lsp");
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn candela-lsp binary");
    let stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    (child, stdin, BufReader::new(stdout))
}

#[tokio::test]
async fn broken_program_produces_a_live_diagnostic() {
    let (mut child, mut stdin, mut stdout) = spawn_server().await;

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "processId": null, "rootUri": null, "capabilities": {} }
        }),
    )
    .await;
    let init_response = timeout(TIMEOUT, read_message(&mut stdout))
        .await
        .expect("timed out waiting for the initialize response");
    assert_eq!(
        init_response["id"], 1,
        "expected the initialize response, got: {init_response}"
    );
    assert!(
        init_response["result"]["capabilities"]["hoverProvider"].is_boolean()
            || init_response["result"]["capabilities"]["hoverProvider"].is_object(),
        "initialize result should advertise hover support: {init_response}"
    );

    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;

    // Missing the right-hand side of `1 +`, a parser error, not merely a
    // type error, so this also exercises the parser half of the frontend.
    let broken_source = "fn main() {\n    let x = 1 +\n}\n";
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///tmp/candela-lsp-smoke-test.cdl",
                    "languageId": "candela",
                    "version": 1,
                    "text": broken_source
                }
            }
        }),
    )
    .await;

    // Skip over any other notifications (e.g. window/logMessage) until the
    // diagnostics publication for this document arrives.
    let mut diagnostics = None;
    for _ in 0..20 {
        let msg = timeout(TIMEOUT, read_message(&mut stdout))
            .await
            .expect("timed out waiting for textDocument/publishDiagnostics");
        if msg["method"] == "textDocument/publishDiagnostics" {
            diagnostics = msg["params"]["diagnostics"].as_array().cloned();
            break;
        }
    }
    let diagnostics =
        diagnostics.expect("server never published diagnostics for the broken document");
    assert!(
        !diagnostics.is_empty(),
        "expected at least one diagnostic for a syntactically broken program"
    );
    let message = diagnostics[0]["message"].as_str().unwrap_or_default();
    assert!(
        !message.is_empty(),
        "diagnostic should carry a human-readable message"
    );
    assert_eq!(
        diagnostics[0]["source"], "candela",
        "diagnostic should be tagged as coming from candela"
    );

    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

#[tokio::test]
async fn well_typed_program_produces_no_diagnostics() {
    let (mut child, mut stdin, mut stdout) = spawn_server().await;

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "processId": null, "rootUri": null, "capabilities": {} }
        }),
    )
    .await;
    let _ = timeout(TIMEOUT, read_message(&mut stdout))
        .await
        .expect("timed out waiting for the initialize response");
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;

    let good_source =
        "fn add(a, b) {\n    return a + b;\n}\n\nfn main() {\n    print(add(1, 2));\n}\n";
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///tmp/candela-lsp-smoke-test-good.cdl",
                    "languageId": "candela",
                    "version": 1,
                    "text": good_source
                }
            }
        }),
    )
    .await;

    let mut diagnostics = None;
    for _ in 0..20 {
        let msg = timeout(TIMEOUT, read_message(&mut stdout))
            .await
            .expect("timed out waiting for textDocument/publishDiagnostics");
        if msg["method"] == "textDocument/publishDiagnostics" {
            diagnostics = msg["params"]["diagnostics"].as_array().cloned();
            break;
        }
    }
    let diagnostics = diagnostics.expect("server never published a diagnostics notification");
    assert!(
        diagnostics.is_empty(),
        "a well-typed program should not produce diagnostics, got: {diagnostics:?}"
    );

    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}
