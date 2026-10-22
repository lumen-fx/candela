//! The first-party packages under `packages/`, from the `candela` command.
//!
//! Every package checks, for the desktop and, where it has a browser half,
//! with `--cfg web`. One package also runs from a plain `candela` source run,
//! which is the case the event seam exists for: the program's handlers run
//! after `main` returns, and the program exits once its child has.
//!
//! The run needs the package's native library, which `cargo build
//! --workspace` and `cargo test --workspace` leave beside the `candela`
//! binary; a root `cargo test` builds no package, and that test says it
//! skipped.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn candela(dir: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .args(args)
        .current_dir(dir)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    common::output_with_deadline(&mut command, &format!("candela {}", args.join(" ")))
}

fn packages() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(repo().join("packages"))
        .expect("packages/ is there")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("candela.toml").is_file())
        .collect();
    found.sort();
    found
}

#[test]
fn every_package_checks_on_each_side() {
    let all = packages();
    assert_eq!(all.len(), 6, "{all:?}");
    for package in all {
        let mut sides = vec![vec!["check"]];
        if package.join("web").is_dir() {
            sides.push(vec!["check", "--cfg", "web"]);
        }
        for args in sides {
            let out = candela(&package, &args);
            assert!(
                out.status.success(),
                "{} {args:?}:\n{}\n{}",
                package.display(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

/// `browser` and `js` raise `browser_only` off the web, so a page's program
/// still runs on the desktop and can tell.
#[test]
fn a_browser_only_package_raises_off_the_web() {
    let dir = std::env::temp_dir().join(format!("candela_browser_only_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = |name: &str| {
        repo()
            .join("packages")
            .join(name)
            .join("src")
            .join(format!("{name}.cdl"))
            .to_string_lossy()
            .replace('\\', "/")
    };
    std::fs::write(
        dir.join("app.cdl"),
        format!(
            r#"
import "{browser}";
import "{js}";

fn main() {{
    try {{
        browser::is_fullscreen();
    }} catch e {{
        print(e);
    }}
    try {{
        js::get("navigator.language");
    }} catch e {{
        print(e);
    }}
}}
"#,
            browser = entry("browser"),
            js = entry("js")
        ),
    )
    .unwrap();
    let out = candela(&dir, &["app.cdl"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        "browser_only\nbrowser_only\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A program that starts a child and returns from `main` sees every line the
/// child writes and its exit, from a plain source run, and then exits.
#[test]
fn a_process_reports_after_main_returns() {
    let binary = Path::new(env!("CARGO_BIN_EXE_candela"));
    let built = binary.parent().unwrap();
    let (prefix, ext) = if cfg!(windows) {
        ("", "dll")
    } else if cfg!(target_os = "macos") {
        ("lib", "dylib")
    } else {
        ("lib", "so")
    };
    let library = built.join(format!("{prefix}candela_process.{ext}"));
    if !library.is_file() {
        eprintln!(
            "skipping: {} is not built; run cargo test --workspace",
            library.display()
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("candela_process_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let package = dir.join("process");
    std::fs::create_dir_all(package.join("dist")).unwrap();
    std::fs::create_dir_all(package.join("src")).unwrap();
    let source = repo().join("packages/process");
    std::fs::copy(source.join("candela.toml"), package.join("candela.toml")).unwrap();
    std::fs::copy(
        source.join("src/process.cdl"),
        package.join("src/process.cdl"),
    )
    .unwrap();
    std::fs::copy(
        &library,
        package.join("dist").join(format!("process.{ext}")),
    )
    .unwrap();

    // The child is candela itself, running a script that writes two lines
    // and exits with 3.
    std::fs::write(
        dir.join("child.cdl"),
        "fn main() {\n    print(\"one\");\n    print(\"two\");\n    exit(3);\n}\n",
    )
    .unwrap();
    let candela_path = binary.to_string_lossy().replace('\\', "/");
    std::fs::write(
        dir.join("app.cdl"),
        format!(
            r#"
import "./process/src/process.cdl";

fn main() {{
    process::start("{candela_path}", ["child.cdl"], process::Options {{ .. }}, fn(e) {{
        match e {{
            Stdout(line) => print("out " + line),
            Stderr(line) => print("err " + line),
            Exit(code) => print("exit " + string(code)),
            Failed(why) => print("failed " + why),
        }}
    }});
    print("main is done");
}}
"#
        ),
    )
    .unwrap();

    let out = candela(&dir, &["app.cdl"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        "main is done\nout one\nout two\nexit 3\n"
    );
    std::fs::remove_dir_all(&dir).ok();
}
