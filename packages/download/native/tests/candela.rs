//! The package from candela: `download::to_file` and `download::cancel`, with
//! their events arriving in the closure a program passes, against the crate's
//! own loopback server.

use std::path::Path;

use candela_download::testkit::{BODY, TestServer};
use candela_package_testkit::{import, run, scratch, stage};

fn package() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

/// A program whose `start` runs `call`, reporting the events of the
/// transfers it starts.
fn program(root: &Path, call: &str) -> String {
    format!(
        r#"
{import}

host "test" {{
    seen(string);
}}

fn report(e: download::Event) {{
    match e {{
        Progress(received, total) => {{
            if received == total {{
                test::seen("all " + string(total));
            }}
        }}
        Done(path) => test::seen("done " + path),
        Failed(why) => test::seen("failed " + why),
    }}
}}

fn start() {{
    {call}
}}

fn main() {{}}
"#,
        import = import(root, "download")
    )
}

fn ended(seen: &[String]) -> bool {
    seen.iter()
        .any(|s| s.starts_with("done ") || s.starts_with("failed "))
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    candela_download::transfer::hex(Sha256::digest(bytes).as_slice())
}

#[test]
fn a_verified_file_lands_and_progress_reaches_its_total() {
    let server = TestServer::start();
    let dir = scratch("download-done");
    let root = stage(package(), "download", &dir);
    let dest = dir.join("out").join("body.bin");
    let call = format!(
        r#"download::to_file("{}", "{}", download::Options {{ checksum: "sha256:{}", .. }}, fn(e) {{ report(e); }});"#,
        server.url("/fixed"),
        dest.to_string_lossy().replace('\\', "/"),
        sha256(BODY)
    );
    let seen = run(&dir, &program(&root, &call), ended);
    assert_eq!(seen[0], format!("all {}", BODY.len()));
    assert!(seen[1].starts_with("done "), "{seen:?}");
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_file_and_a_mismatch_fail() {
    let server = TestServer::start();
    let dir = scratch("download-failed");
    let root = stage(package(), "download", &dir);
    let missing = format!(
        r#"download::to_file("{}", "{}", download::Options {{ .. }}, fn(e) {{ report(e); }});"#,
        server.url("/missing"),
        dir.join("a.bin").to_string_lossy().replace('\\', "/")
    );
    let seen = run(&dir, &program(&root, &missing), ended);
    assert!(seen.last().unwrap().contains("404"), "{seen:?}");

    let mismatch = format!(
        r#"download::to_file("{}", "{}", download::Options {{ checksum: "sha256:{}", .. }}, fn(e) {{ report(e); }});"#,
        server.url("/mismatch"),
        dir.join("b.bin").to_string_lossy().replace('\\', "/"),
        sha256(BODY)
    );
    let seen = run(&dir, &program(&root, &mismatch), ended);
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
    let root = stage(package(), "download", &dir);
    let call = format!(
        r#"let handle = download::to_file("{}", "{}", download::Options {{ .. }}, fn(e) {{ report(e); }});
    test::seen("cancelled " + string(download::cancel(handle)));
    test::seen("again " + string(download::cancel(handle)));"#,
        server.url("/stall"),
        dir.join("c.bin").to_string_lossy().replace('\\', "/")
    );
    let seen = run(&dir, &program(&root, &call), ended);
    assert_eq!(
        seen,
        ["cancelled true", "again false", "failed cancelled"],
        "{seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}
