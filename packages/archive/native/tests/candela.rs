//! The package from candela: `archive::extract`, with its one event arriving
//! in the closure a program passes.

use std::path::Path;

use candela_archive::testkit;
use candela_package_testkit::{import, run, scratch, stage};

fn package() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A program whose `start` extracts `src` into `dest` keeping `include`.
fn program(root: &Path, src: &Path, dest: &Path, include: &str) -> String {
    format!(
        r#"
{import}

host "test" {{
    seen(string);
}}

fn start() {{
    archive::extract("{src}", "{dest}", archive::Options {{ include: {include} }}, fn(e) {{
        match e {{
            Done(dest, files) => test::seen("done " + string(files)),
            Failed(why) => test::seen("failed " + why),
        }}
    }});
}}

fn main() {{}}
"#,
        import = import(root, "archive"),
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
    let root = stage(package(), "archive", &dir);
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
        let seen = run(&dir, &program(&root, &src, &dest, "[]"), ended);
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
    let root = stage(package(), "archive", &dir);
    let src = dir.join("natives.jar");
    testkit::natives_jar(&src).unwrap();
    let dest = dir.join("natives");
    let seen = run(&dir, &program(&root, &src, &dest, r#"["*.so"]"#), ended);
    assert_eq!(seen, ["done 1"]);
    assert!(dest.join("linux/x64/org/lwjgl/liblwjgl.so").is_file());
    assert!(!dest.join("META-INF").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_entry_that_escapes_fails_the_extraction() {
    let dir = scratch("archive-escape");
    let root = stage(package(), "archive", &dir);
    let src = dir.join("escape.zip");
    testkit::escaping_zip(&src).unwrap();
    let seen = run(&dir, &program(&root, &src, &dir.join("out"), "[]"), ended);
    assert_eq!(seen.len(), 1);
    assert!(seen[0].starts_with("failed "), "{seen:?}");
    std::fs::remove_dir_all(&dir).ok();
}
