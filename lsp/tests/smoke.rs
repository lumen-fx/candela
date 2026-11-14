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

/// The `file://` URI the server names the file at `path` by. Imports reach a
/// file by its canonical path, so this canonicalizes first: a temporary
/// folder behind a symlink (macOS) and a verbatim `\\?\C:` path (Windows)
/// then compare equal to what the server publishes.
fn file_uri(path: &std::path::Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    tower_lsp::lsp_types::Url::from_file_path(&path)
        .expect("an absolute path")
        .to_string()
}

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

/// Hover over a function whose parameter is a user enum names that enum. This
/// is the whole request path an editor takes, through the server's own
/// rendering, and the type it shows is the one a compiler diagnostic would
/// print.
#[tokio::test]
async fn hover_over_an_enum_typed_parameter_names_the_enum() {
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

    // `width` is on line 2 (zero-based), its name starting at column 3.
    let source = "enum Value { Num(int), Text(string) }\n\
                  \n\
                  fn width(v: Value) -> int {\n\
                  \x20   return 1;\n\
                  }\n\
                  \n\
                  fn main() {}\n";
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///tmp/candela-lsp-smoke-test-hover.cdl",
                    "languageId": "candela",
                    "version": 1,
                    "text": source
                }
            }
        }),
    )
    .await;

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///tmp/candela-lsp-smoke-test-hover.cdl" },
                "position": { "line": 2, "character": 4 }
            }
        }),
    )
    .await;

    let mut hover = None;
    for _ in 0..20 {
        let msg = timeout(TIMEOUT, read_message(&mut stdout))
            .await
            .expect("timed out waiting for the hover response");
        if msg["id"] == 2 {
            hover = Some(msg);
            break;
        }
    }
    let hover = hover.expect("server never answered the hover request");
    let shown = hover["result"]["contents"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(
        shown.contains("v: Value"),
        "hover should name the parameter's enum: {shown}"
    );
    assert!(
        shown.contains("fn width(v: Value) -> int"),
        "the signature should name it too: {shown}"
    );
    assert!(
        !shown.contains("enum"),
        "no type should read as the bare word `enum`: {shown}"
    );

    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

/// Reads messages until the response to request `id` arrives.
async fn response(stdout: &mut BufReader<tokio::process::ChildStdout>, id: i64) -> Value {
    for _ in 0..50 {
        let msg = timeout(TIMEOUT, read_message(stdout))
            .await
            .expect("timed out waiting for a response");
        if msg["id"] == id {
            return msg;
        }
    }
    panic!("server never answered request {id}");
}

/// A package named in `initializationOptions` resolves `import "engine"`, and
/// completion, hover and go-to-definition reach into it: a glob import's
/// attribute after `@`, a struct's fields after `.` on a typed parameter
/// while the buffer does not compile, and the field's declaration in the
/// package's file.
#[tokio::test]
async fn an_import_root_from_the_settings_reaches_completion_and_definition() {
    let dir = std::env::temp_dir().join(format!("candela_lsp_roots_{}", std::process::id()));
    let package = dir.join("engine");
    std::fs::create_dir_all(package.join("src")).expect("create package dir");
    std::fs::write(
        package.join("src/main.cdl"),
        "host \"engine\" {\n    attribute system(after: fn[] = []) on fn;\n}\n\n\
         /// Where a thing is.\nstruct Spot {\n    /// Across.\n    x: float = 0.0,\n    y: float = 0.0,\n}\n",
    )
    .expect("write package");
    // The package says where its documentation lives, so hover links to it.
    std::fs::write(
        package.join("candela.toml"),
        "[package]\nname = \"engine\"\nversion = \"0.1.0\"\ndocs = \"https://engine.dev/api?name={name}\"\n",
    )
    .expect("write manifest");
    let game = dir.join("game.cdl");
    let compiles = "import \"engine\" { * };\n\n@system\nfn walk(s: Spot) {\n    s.x += 1.0;\n}\n";
    std::fs::write(&game, compiles).expect("write game");
    let uri = file_uri(&game);

    let (mut child, mut stdin, mut stdout) = spawn_server().await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {},
                "initializationOptions": { "importRoots": { "engine": package } }
            }
        }),
    )
    .await;
    let _ = response(&mut stdout, 1).await;
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "languageId": "candela", "version": 1, "text": compiles } }
        }),
    )
    .await;

    // Mid-edit: `s.` on its own line does not compile.
    let editing = compiles.replace("    s.x += 1.0;", "    s.");
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": editing }]
            }
        }),
    )
    .await;
    let request = |id: i64, method: &str, line: u32, character: u32| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        })
    };
    write_message(&mut stdin, &request(3, "textDocument/completion", 4, 6)).await;
    let fields: Vec<String> = response(&mut stdout, 3).await["result"]
        .as_array()
        .expect("a completion list")
        .iter()
        .map(|i| i["label"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(fields, vec!["x", "y"]);

    write_message(&mut stdin, &request(4, "textDocument/completion", 2, 1)).await;
    let attrs = response(&mut stdout, 4).await;
    assert_eq!(attrs["result"][0]["label"], "system", "{attrs}");

    // Back to the compiling text: hover and definition on `s.x`.
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 3 },
                "contentChanges": [{ "text": compiles }]
            }
        }),
    )
    .await;
    write_message(&mut stdin, &request(5, "textDocument/hover", 4, 6)).await;
    let hover = response(&mut stdout, 5).await;
    let shown = hover["result"]["contents"]["value"]
        .as_str()
        .unwrap_or_default();
    assert!(
        shown.contains("Spot.x: float = 0.0") && shown.contains("Across."),
        "{shown}"
    );
    assert!(
        shown.ends_with("[Open docs](https://engine.dev/api?name=Spot.x)"),
        "{shown}"
    );

    write_message(&mut stdin, &request(6, "textDocument/definition", 4, 6)).await;
    let def = response(&mut stdout, 6).await;
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        def["result"]["uri"]
            .as_str()
            .unwrap_or_default()
            .ends_with("engine/src/main.cdl"),
        "{def}"
    );
    assert_eq!(def["result"]["range"]["start"]["line"], 7, "{def}");

    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

/// Settings that change after start-up apply to the open buffers: a module
/// named by its `.cdl` file under a `candela` key resolves once the
/// configuration names it. Outline, signature help, an attribute warning, an
/// error inside the imported file, save and close all answer on that buffer.
#[tokio::test]
async fn settings_changed_later_apply_to_open_buffers() {
    let dir = std::env::temp_dir().join(format!("candela_lsp_config_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let module = dir.join("kit.cdl");
    std::fs::write(
        &module,
        "host \"kit\" {\n    attribute tag(n: int = 0) on fn;\n}\n\n\
         // Adds.\nfn add(a: int, b: int) -> int { return a + b; }\nenum Mode { On, Off }\n",
    )
    .expect("write module");
    let game = dir.join("game.cdl");
    let text = "import \"kit\" { * };\n\n@tag\n@tag\nfn run() {\n    print(add(1, 2));\n}\n";
    std::fs::write(&game, text).expect("write game");
    let uri = file_uri(&game);

    let (mut child, mut stdin, mut stdout) = spawn_server().await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "processId": null, "rootUri": null, "capabilities": {} }
        }),
    )
    .await;
    let _ = response(&mut stdout, 1).await;
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "languageId": "candela", "version": 1, "text": text } }
        }),
    )
    .await;
    let diagnostics = |msg: &Value| {
        msg["params"]["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let next_diagnostics = async |stdout: &mut BufReader<tokio::process::ChildStdout>| loop {
        let msg = timeout(TIMEOUT, read_message(stdout))
            .await
            .expect("diagnostics");
        if msg["method"] == "textDocument/publishDiagnostics" {
            return msg;
        }
    };
    let first = next_diagnostics(&mut stdout).await;
    assert_eq!(
        diagnostics(&first)[0]["code"],
        "cannot_read_file",
        "{first}"
    );

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "method": "workspace/didChangeConfiguration",
            "params": { "settings": { "candela": {
                "importRoots": { "kit": module },
                "libPath": dir.join("no-std-here")
            } } }
        }),
    )
    .await;
    let after = next_diagnostics(&mut stdout).await;
    let codes: Vec<String> = diagnostics(&after)
        .iter()
        .filter_map(|d| d["code"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(codes, vec!["attribute_repeated"], "{after}");

    let position = |line: u32, character: u32| json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } });
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/signatureHelp", "params": position(5, 17) }),
    )
    .await;
    let help = response(&mut stdout, 3).await;
    assert_eq!(
        help["result"]["signatures"][0]["label"], "fn add(a: int, b: int) -> int",
        "{help}"
    );
    assert_eq!(help["result"]["activeParameter"], 1, "{help}");

    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "id": 4, "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": uri } }
        }),
    )
    .await;
    let outline = response(&mut stdout, 4).await;
    assert_eq!(outline["result"][0]["name"], "run", "{outline}");

    // An error in the imported file is published on that file, at its span.
    std::fs::write(
        &module,
        "host \"kit\" {\n    attribute tag(n: int = 0) on fn;\n}\n\nfn add(a: int, b: int) -> int { return a + \"x\"; }\n",
    )
    .expect("break module");
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
    )
    .await;
    let module_uri = file_uri(&module);
    let saved = loop {
        let saved = next_diagnostics(&mut stdout).await;
        if saved["params"]["uri"] == module_uri.as_str() {
            break saved;
        }
    };
    assert_eq!(
        diagnostics(&saved)[0]["range"]["start"]["line"],
        4,
        "{saved}"
    );

    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didClose", "params": { "textDocument": { "uri": uri } } }),
    )
    .await;
    // What the save published for the buffer may still be on its way; the
    // close clears it after.
    loop {
        let closed = next_diagnostics(&mut stdout).await;
        if closed["params"]["uri"] == uri.as_str() && diagnostics(&closed).is_empty() {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

/// A package whose entry gathers its other files with `export import` keeps
/// each name's definition in the file that declares it.
#[tokio::test]
async fn a_definition_through_an_export_import_lands_in_the_declaring_file() {
    let dir = std::env::temp_dir().join(format!("candela_lsp_export_{}", std::process::id()));
    let package = dir.join("engine");
    std::fs::create_dir_all(package.join("src/parts")).expect("create package dir");
    std::fs::write(
        package.join("src/main.cdl"),
        "export import \"engine/parts/spot\" { * };\n",
    )
    .expect("write package entry");
    std::fs::write(
        package.join("src/parts/spot.cdl"),
        "// Where a thing is.\nstruct Spot {\n    x: float = 0.0,\n}\n",
    )
    .expect("write package part");
    let game = dir.join("game.cdl");
    let text = "import \"engine\" { * };\n\nfn walk(s: Spot) {\n    s.x += 1.0;\n}\n";
    std::fs::write(&game, text).expect("write game");
    let uri = file_uri(&game);

    let (mut child, mut stdin, mut stdout) = spawn_server().await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {},
                "initializationOptions": { "importRoots": { "engine": package } }
            }
        }),
    )
    .await;
    let _ = response(&mut stdout, 1).await;
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "languageId": "candela", "version": 1, "text": text } }
        }),
    )
    .await;
    let request = |id: i64, line: u32, character: u32| {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        })
    };
    // The type in the signature, then the field it is used through.
    write_message(&mut stdin, &request(2, 2, 12)).await;
    let ty = response(&mut stdout, 2).await;
    write_message(&mut stdin, &request(3, 3, 6)).await;
    let field = response(&mut stdout, 3).await;
    let _ = std::fs::remove_dir_all(&dir);
    for (def, line) in [(&ty, 1), (&field, 2)] {
        assert!(
            def["result"]["uri"]
                .as_str()
                .unwrap_or_default()
                .ends_with("engine/src/parts/spot.cdl"),
            "{def}"
        );
        assert_eq!(def["result"]["range"]["start"]["line"], line, "{def}");
    }

    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

/// Starts a server with `options`, opens `files` in order, and returns it.
async fn open_files(
    options: Value,
    files: &[(&str, &str)],
) -> (
    Child,
    tokio::process::ChildStdin,
    BufReader<tokio::process::ChildStdout>,
) {
    let (child, mut stdin, mut stdout) = spawn_server().await;
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "processId": null, "rootUri": null, "capabilities": {}, "initializationOptions": options }
        }),
    )
    .await;
    let _ = response(&mut stdout, 1).await;
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    for (uri, text) in files {
        write_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0", "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": uri, "languageId": "candela", "version": 1, "text": text } }
            }),
        )
        .await;
    }
    (child, stdin, stdout)
}

/// Reads publications until one for `uri` satisfies `done`, and returns its
/// diagnostics.
async fn diagnostics_for(
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    uri: &str,
    done: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    loop {
        let msg = timeout(TIMEOUT, read_message(stdout))
            .await
            .unwrap_or_else(|_| panic!("no diagnostics for {uri}"));
        if msg["method"] == "textDocument/publishDiagnostics" && msg["params"]["uri"] == uri {
            let list = msg["params"]["diagnostics"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if done(&list) {
                return list;
            }
        }
    }
}

/// A call with its arguments out of order shows each one that does not fit,
/// naming its parameter and pointing at the declaration, and a second error in
/// another statement of the same body shows too.
#[tokio::test]
async fn every_error_of_a_buffer_shows_at_once() {
    let text = "struct A { id: int = 0 }\nstruct B { width: float = 0.0 }\n\
                fn take(a: A, b: B) -> float { return b.width; }\n\
                fn run(a: A, b: B) {\n    let w = take(b, a);\n    let h = b.height;\n}\n";
    let uri = "file:///tmp/candela-lsp-every-error.cdl";
    let (mut child, stdin, mut stdout) = open_files(json!({}), &[(uri, text)]).await;
    let found = diagnostics_for(&mut stdout, uri, |d| !d.is_empty()).await;
    let messages: Vec<&str> = found.iter().filter_map(|d| d["message"].as_str()).collect();
    assert_eq!(found.len(), 3, "{messages:#?}");
    assert_eq!(
        found[0]["message"],
        "Parameter a of take takes A, but this argument is B"
    );
    assert_eq!(
        found[0]["range"]["start"],
        json!({ "line": 4, "character": 17 })
    );
    let related = &found[0]["relatedInformation"][0];
    assert_eq!(
        related["location"]["range"]["start"],
        json!({ "line": 2, "character": 8 }),
        "{related}"
    );
    assert_eq!(
        found[1]["message"],
        "Parameter b of take takes B, but this argument is A"
    );
    assert!(messages[2].contains("height"), "{messages:#?}");
    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

/// Editing an imported file, without saving it, checks the open file that
/// imports it again, against the edited text; the error shows on the file
/// it is in.
#[tokio::test]
async fn an_unsaved_change_to_an_import_checks_its_importer() {
    let dir = std::env::temp_dir().join(format!("candela_lsp_dependents_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let lib = dir.join("lib.cdl");
    let lib_text = "fn half(x: float) -> float { return x / 2.0; }\n";
    std::fs::write(&lib, lib_text).expect("write lib");
    let game = dir.join("game.cdl");
    let game_text = "import \"lib.cdl\" { * };\n\nfn main() {\n    print(half(3.0));\n}\n";
    std::fs::write(&game, game_text).expect("write game");
    let lib_uri = file_uri(&lib);
    let game_uri = file_uri(&game);
    let (mut child, mut stdin, mut stdout) = open_files(
        json!({ "diagnosticsDelay": 50 }),
        &[(&game_uri, game_text), (&lib_uri, lib_text)],
    )
    .await;
    let clean = diagnostics_for(&mut stdout, &game_uri, |_| true).await;
    assert!(clean.is_empty(), "{clean:?}");

    // `half` now takes a string; the importer's call no longer fits.
    let broken = "fn half(x: string) -> float { return 1.0; }\n";
    write_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": { "textDocument": { "uri": lib_uri, "version": 2 }, "contentChanges": [{ "text": broken }] }
        }),
    )
    .await;
    let found = diagnostics_for(&mut stdout, &game_uri, |d| !d.is_empty()).await;
    assert_eq!(found[0]["code"], "argument_type_mismatch", "{found:?}");
    assert_eq!(
        found[0]["relatedInformation"][0]["location"]["uri"],
        lib_uri.as_str(),
        "{found:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}

/// A package whose candela.toml names a `check` command has it run after a
/// save of a program that imports it, and what it prints shows, on the file
/// it names, labelled with the package.
#[cfg(unix)]
#[tokio::test]
async fn a_package_check_runs_after_a_save() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("candela_lsp_check_{}", std::process::id()));
    let package = dir.join("engine");
    std::fs::create_dir_all(package.join("src")).expect("create package");
    std::fs::write(
        package.join("src/main.cdl"),
        "fn speed() -> float { return 1.0; }\n",
    )
    .expect("write entry");
    std::fs::write(
        package.join("candela.toml"),
        "[package]\nname = \"engine\"\nversion = \"0.1.0\"\ncheck = [\"./check.sh\", \"{file}\"]\n",
    )
    .expect("write manifest");
    // Reports one problem in settings.toml beside the checked file, and
    // names the file it was given.
    let script = package.join("check.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '[{\"file\": \"settings.toml\", \"line\": 2, \"severity\": \"warning\", \"code\": \"bad\", \"message\": \"checked %s\"}]' \"$1\"\n",
    )
    .expect("write check");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let game = dir.join("game.cdl");
    let text = "import \"engine\" { * };\n\nfn main() {\n    print(speed());\n}\n";
    std::fs::write(&game, text).expect("write game");
    std::fs::write(dir.join("settings.toml"), "[a]\nb = 1\n").expect("write settings");
    let uri = format!("file://{}", game.display());
    let settings_uri = format!("file://{}", dir.join("settings.toml").display());
    let options = json!({ "importRoots": { "engine": package } });
    let (mut child, mut stdin, mut stdout) = open_files(options, &[(&uri, text)]).await;
    write_message(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "textDocument/didSave", "params": { "textDocument": { "uri": uri } } }),
    )
    .await;
    let found = diagnostics_for(&mut stdout, &settings_uri, |d| !d.is_empty()).await;
    assert_eq!(found[0]["source"], "engine", "{found:?}");
    assert_eq!(found[0]["severity"], 2, "{found:?}");
    assert_eq!(found[0]["code"], "bad", "{found:?}");
    assert_eq!(found[0]["message"], format!("checked {}", game.display()));
    assert_eq!(
        found[0]["range"]["start"],
        json!({ "line": 1, "character": 0 })
    );
    assert_eq!(
        found[0]["range"]["end"],
        json!({ "line": 1, "character": 5 })
    );
    std::fs::remove_dir_all(&dir).ok();
    shutdown_and_exit(&mut child, stdin, &mut stdout).await;
}
