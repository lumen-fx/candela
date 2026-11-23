//! `std/archive` from candela: `archive::extract`, with its one event arriving
//! in the closure a program passes.

use std::path::Path;

use candela_std_archive::testkit;
use candela_std_testkit::{run, scratch, stage};

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A program whose `start` extracts `src` into `dest` keeping `include`.
fn program(src: &Path, dest: &Path, include: &str) -> String {
    format!(
        r#"
import "std/archive";

host "test" {{
    fn seen(text: string);
}}

fn start() {{
    archive::extract("{src}", "{dest}", include: {include}) fn(e) {{
        match e {{
            Done(dest, files) => test::seen("done " + string(files)),
            Failed(why) => test::seen("failed " + why),
        }}
    }};
}}

fn main() {{}}
"#,
        src = slashes(src),
        dest = slashes(dest),
    )
}

fn ended(seen: &[String]) -> bool {
    !seen.is_empty()
}

#[test]
fn every_format_unpacks() {
    let dir = scratch("archive-formats");
    let libs = stage(&["archive"], &dir);
    for (name, make) in [
        (
            "a.zip",
            testkit::normal_zip as fn(&Path) -> std::io::Result<()>,
        ),
        ("a.tar.gz", testkit::normal_tar_gz),
        ("a.tar", testkit::normal_tar),
    ] {
        let src = dir.join(name);
        make(&src).unwrap();
        let dest = dir.join(format!("out-{name}"));
        let seen = run(&libs, &dir, &program(&src, &dest, "[]"), ended);
        assert_eq!(seen, ["done 3"], "{name}");
        for (member, body) in testkit::MEMBERS {
            assert_eq!(std::fs::read_to_string(dest.join(member)).unwrap(), body);
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn include_keeps_only_what_it_matches() {
    let dir = scratch("archive-include");
    let libs = stage(&["archive"], &dir);
    let src = dir.join("natives.jar");
    testkit::natives_jar(&src).unwrap();
    let dest = dir.join("natives");
    let seen = run(&libs, &dir, &program(&src, &dest, r#"["*.so"]"#), ended);
    assert_eq!(seen, ["done 1"]);
    assert!(dest.join("linux/x64/org/lwjgl/liblwjgl.so").is_file());
    assert!(!dest.join("META-INF").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_entry_that_escapes_fails_the_extraction() {
    let dir = scratch("archive-escape");
    let libs = stage(&["archive"], &dir);
    let src = dir.join("escape.zip");
    testkit::escaping_zip(&src).unwrap();
    let seen = run(&libs, &dir, &program(&src, &dir.join("out"), "[]"), ended);
    assert_eq!(seen.len(), 1);
    assert!(seen[0].starts_with("failed "), "{seen:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `create` writes a zip and a tar.gz that `extract` reads back, and `gzip`
/// and `gunzip` round-trip one file.
#[test]
fn created_archives_and_gzip_round_trip() {
    let dir = scratch("archive-create");
    let libs = stage(&["archive"], &dir);
    let tree = dir.join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("a.txt"), "alpha").unwrap();
    std::fs::write(tree.join("sub").join("b.txt"), "beta").unwrap();
    let program = format!(
        r#"
import "std/archive";

host "test" {{
    fn seen(text: string);
}}

fn report(what: string, e: archive::Event) {{
    match e {{
        Done(dest, files) => test::seen(what + " " + string(files)),
        Failed(why) => test::seen(what + " failed " + why),
    }}
}}

fn start() {{
    archive::set_max_concurrent(16);
    archive::create("{d}/out.zip", ["{t}/a.txt", "{t}/sub"], base: "{t}") fn(e) {{
        report("zip", e);
        archive::extract("{d}/out.zip", "{d}/unzipped") fn(e) {{ report("unzip", e); }};
    }};
    archive::create("{d}/out.tar.gz", ["{t}"], base: "{d}") fn(e) {{
        report("tgz", e);
        archive::extract("{d}/out.tar.gz", "{d}/untarred") fn(e) {{ report("untar", e); }};
    }};
    archive::create("{d}/out.rar", ["{t}"]) fn(e) {{ report("rar", e); }};
    archive::create("{d}/outside.zip", ["{t}/a.txt"], base: "{t}/sub") fn(e) {{ report("outside", e); }};
    archive::gzip("{t}/a.txt", "{d}/a.gz") fn(e) {{
        report("gzip", e);
        archive::gunzip("{d}/a.gz", "{d}/a.back") fn(e) {{ report("gunzip", e); }};
    }};
}}

fn main() {{}}
"#,
        d = slashes(&dir),
        t = slashes(&tree),
    );
    let seen = run(&libs, &dir, &program, |seen| seen.len() == 8);
    for expected in ["zip 2", "unzip 2", "tgz 2", "untar 2", "gzip 1", "gunzip 1"] {
        assert!(seen.iter().any(|s| s == expected), "{expected}: {seen:?}");
    }
    assert!(
        seen.iter().any(|s| s.starts_with("rar failed ")),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|s| s.starts_with("outside failed ")),
        "{seen:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("unzipped/sub/b.txt")).unwrap(),
        "beta"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("untarred/tree/a.txt")).unwrap(),
        "alpha"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a.back")).unwrap(),
        "alpha"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A job started while the cap's worth are running fails at once and names
/// the limit; once a slot frees, the next one runs. A cap below one counts as
/// one. The held job reads from a FIFO, so it runs until the test writes.
#[cfg(unix)]
#[test]
fn a_job_past_the_cap_fails_until_a_slot_frees() {
    let dir = scratch("archive-cap");
    let libs = stage(&["archive"], &dir);
    let fifo = dir.join("held");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    std::fs::write(dir.join("a.txt"), "alpha").unwrap();
    let program = format!(
        r#"
import "std/archive";

host "test" {{
    fn seen(text: string);
}}

fn report(what: string, e: archive::Event) {{
    match e {{
        Done(dest, files) => test::seen(what + " " + string(files)),
        Failed(why) => test::seen(what + " failed " + why),
    }}
}}

fn start() {{
    archive::set_max_concurrent(0);
    archive::gzip("{d}/held", "{d}/held.gz") fn(e) {{
        report("held", e);
        archive::gzip("{d}/a.txt", "{d}/after.gz") fn(e) {{ report("after", e); }};
    }};
    archive::gzip("{d}/a.txt", "{d}/past.gz") fn(e) {{ report("past", e); }};
}}

fn main() {{}}
"#,
        d = slashes(&dir),
    );
    let released = std::cell::Cell::new(false);
    let seen = run(&libs, &dir, &program, |seen| {
        if !released.get() && seen.iter().any(|s| s.starts_with("past ")) {
            // Opening the write end lets the held job's read go on; closing
            // it ends the input.
            std::fs::write(&fifo, "held input").unwrap();
            released.set(true);
        }
        seen.len() == 3
    });
    assert_eq!(
        seen[0],
        "past failed 1 archive jobs are already running, which is the limit; raise it with \
         archive::set_max_concurrent or wait for one to finish"
    );
    assert_eq!(&seen[1..], ["held 1", "after 1"]);
    std::fs::remove_dir_all(&dir).ok();
}
