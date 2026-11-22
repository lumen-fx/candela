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
