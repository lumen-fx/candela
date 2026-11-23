//! `std/http` from candela: `http::download` and `Transfer::cancel`, with
//! their events arriving in the closure a program passes, against the crate's
//! own loopback server.

use candela_std_net::http::testkit::{BODY, TestServer};
use candela_std_testkit::{run, scratch, stage};

/// A program whose `start` runs `call`, reporting the events of the
/// transfers it starts.
fn program(call: &str) -> String {
    format!(
        r#"
import "std/http";

host "test" {{
    fn seen(text: string);
}}

fn report(e: http::Progress) {{
    match e {{
        Received(received, Some(total)) => {{
            if received == total {{
                test::seen("all " + string(total));
            }}
        }}
        Received(_, None) => {{}}
        Saved(path) => test::seen("done " + path),
        Failed(why) => test::seen("failed " + why),
    }}
}}

fn start() {{
    {call}
}}

fn main() {{}}
"#,
    )
}

fn ended(seen: &[String]) -> bool {
    seen.iter()
        .any(|s| s.starts_with("done ") || s.starts_with("failed "))
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    candela_std_net::http::transfer::hex(Sha256::digest(bytes).as_slice())
}

#[test]
fn a_verified_file_lands_and_progress_reaches_its_total() {
    let server = TestServer::start();
    let dir = scratch("download-done");
    let libs = stage(&["net"], &dir);
    let dest = dir.join("out").join("body.bin");
    let call = format!(
        r#"http::download("{}", "{}", checksum: "sha256:{}") fn(e) {{ report(e); }};"#,
        server.url("/fixed"),
        dest.to_string_lossy().replace('\\', "/"),
        sha256(BODY)
    );
    let seen = run(&libs, &dir, &program(&call), ended);
    assert_eq!(seen[0], format!("all {}", BODY.len()));
    assert!(seen[1].starts_with("done "), "{seen:?}");
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_file_and_a_mismatch_fail() {
    let server = TestServer::start();
    let dir = scratch("download-failed");
    let libs = stage(&["net"], &dir);
    let missing = format!(
        r#"http::download("{}", "{}") fn(e) {{ report(e); }};"#,
        server.url("/missing"),
        dir.join("a.bin").to_string_lossy().replace('\\', "/")
    );
    let seen = run(&libs, &dir, &program(&missing), ended);
    assert!(seen.last().unwrap().contains("404"), "{seen:?}");

    let mismatch = format!(
        r#"http::download("{}", "{}", checksum: "sha256:{}") fn(e) {{ report(e); }};"#,
        server.url("/mismatch"),
        dir.join("b.bin").to_string_lossy().replace('\\', "/"),
        sha256(BODY)
    );
    let seen = run(&libs, &dir, &program(&mismatch), ended);
    assert!(
        seen.last().unwrap().contains("checksum mismatch"),
        "{seen:?}"
    );
    assert!(!dir.join("b.bin").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cancel_is_the_last_event() {
    let server = TestServer::start();
    let dir = scratch("download-cancel");
    let libs = stage(&["net"], &dir);
    let call = format!(
        r#"let handle = http::download("{}", "{}") fn(e) {{ report(e); }};
    test::seen("cancelled " + string(handle.cancel()));
    test::seen("again " + string(handle.cancel()));"#,
        server.url("/stall"),
        dir.join("c.bin").to_string_lossy().replace('\\', "/")
    );
    let seen = run(&libs, &dir, &program(&call), ended);
    assert_eq!(
        seen,
        ["cancelled true", "again false", "failed cancelled"],
        "{seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// `http::request` answers the reply in memory, whatever its status: the
/// method, a header and the body go out, and the status, headers and body
/// come back; a cancel answers `cancelled`.
#[test]
fn a_request_answers_the_reply() {
    let server = TestServer::start();
    let dir = scratch("http-request");
    let libs = stage(&["net"], &dir);
    let program = format!(
        r#"
import "std/http";

host "test" {{
    fn seen(text: string);
}}

fn show(r: http::Response!string) {{
    match r {{
        Ok(reply) => test::seen(string(reply.status) + " " + (reply.headers.get("x-echo") else "-") + " " + reply.body),
        Err(why) => test::seen("error " + why),
    }}
}}

fn start() {{
    http::request("{echo}", method: "post", headers: {{"X-Test": "t"}}, body: "hi") fn(r) {{
        show(r);
        http::request("{missing}") fn(r) {{
            show(r);
            let stalled = http::request("{stall}") fn(r) {{ show(r); }};
            test::seen("cancel " + string(stalled.cancel()));
        }};
    }};
}}

fn main() {{}}
"#,
        echo = server.url("/echo"),
        missing = server.url("/missing"),
        stall = server.url("/stall"),
    );
    let seen = run(&libs, &dir, &program, |seen| {
        seen.iter().any(|s| s.starts_with("error "))
    });
    assert_eq!(
        seen,
        [
            "201 yes POST t hi",
            "404 - not found",
            "cancel true",
            "error cancelled"
        ],
        "{seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A transfer with nothing to fetch or nowhere to put it fails at once, and
/// so does one started while the cap's worth are running; cancelling the one
/// that holds the slot lets the next start.
#[test]
fn bad_input_and_a_full_cap_fail_at_once() {
    let server = TestServer::start();
    let dir = scratch("http-cap");
    let libs = stage(&["net"], &dir);
    let program = format!(
        r#"
import "std/http";

host "test" {{
    fn seen(text: string);
}}

fn saw(what: string, e: http::Progress) {{
    match e {{
        Received(_, _) => {{}}
        Saved(_) => test::seen(what + " saved"),
        Failed(why) => test::seen(what + " failed " + why),
    }}
}}

fn answered(what: string, r: http::Response!string) {{
    match r {{
        Ok(reply) => test::seen(what + " " + string(reply.status)),
        Err(why) => test::seen(what + " error " + why),
    }}
}}

fn start() {{
    http::download("", "{d}/a.bin") fn(e) {{ saw("no url", e); }};
    http::download("{fixed}", " ") fn(e) {{ saw("no path", e); }};
    http::request(" ") fn(r) {{ answered("no request url", r); }};
    http::set_max_concurrent(0);
    let held = http::download("{stall}", "{d}/held.bin") fn(e) {{ saw("held", e); }};
    http::download("{fixed}", "{d}/b.bin") fn(e) {{ saw("past download", e); }};
    http::request("{fixed}") fn(r) {{
        answered("past request", r);
        held.cancel();
        http::request("{fixed}") fn(r) {{
            answered("after", r);
            http::request("{fixed}", method: "NO SPACES") fn(r) {{ answered("bad method", r); }};
        }};
    }};
}}

fn main() {{}}
"#,
        d = dir.to_string_lossy().replace('\\', "/"),
        fixed = server.url("/fixed"),
        stall = server.url("/stall"),
    );
    let seen = run(&libs, &dir, &program, |seen| {
        seen.iter().any(|s| s.starts_with("bad method "))
    });
    let limit = "1 downloads are already running, which is the limit; raise it with \
                 http::set_max_concurrent or wait for one to finish";
    let request_limit = "1 transfers are already running, which is the limit; raise it with \
                         http::set_max_concurrent or wait for one to finish";
    assert!(
        seen.iter().any(|s| s.starts_with("bad method error ")),
        "{seen:?}"
    );
    for expected in [
        "no url failed download was given no url".to_owned(),
        "no path failed download was given no destination path".to_owned(),
        "no request url error request was given no url".to_owned(),
        format!("past download failed {limit}"),
        format!("past request error {request_limit}"),
        "held failed cancelled".to_owned(),
        "after 200".to_owned(),
    ] {
        assert!(seen.contains(&expected), "{expected}: {seen:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
