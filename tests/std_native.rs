//! The standard library modules that run work off the program's thread or in
//! a browser: process, http, archive, websocket, browser and js.
//!
//! Every function of each one compiles, for the desktop and with `--cfg web`.
//! browser and js raise `browser_only` off the web. process runs from a plain
//! `candela` source run and from a `.cdlb` under `candela-vm`, which is the
//! case the event seam exists for: the program's handlers run after `main`
//! returns, and the program exits once its child has.
//!
//! A run needs the module's native library, which the test builds and lays
//! out under a library directory of its own; see `built`.

mod built;
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn candela(dir: &Path, libs: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .args(args)
        .current_dir(dir)
        .env("CANDELA_LIB_PATH", libs);
    common::output_with_deadline(&mut command, &format!("candela {}", args.join(" ")))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("candela_std_native_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A call to every function the modules declare, behind a branch that never
/// runs, so the check compiles each one.
const EVERY_FUNCTION: &str = r#"
import "std/archive";
import "std/browser";
import "std/cli";
import "std/event";
import "std/http";
import "std/js";
import "std/json";
import "std/process";
import "std/regex";
import "std/term";
import "std/toml";
import "std/websocket";

fn main() {
    if argv().len() > 1000 {
        let child = process::start("x", ["a"], cwd: "", env: {}, end_at_exit: true, stdin: true) fn(e) {
            print(e);
        };
        print(child.write("t"));
        print(child.close_input());
        print(child.stop());
        let request = http::request("u", method: "POST", headers: {"a": "b"}, body: "x", timeout_ms: 1, max_bytes: 1) fn(r) {
            match r {
                Ok(response) => print(response.status),
                Err(why) => print(why),
            }
        };
        print(request.cancel());
        let transfer = http::download("u", "p", checksum: "", timeout_ms: 1, max_bytes: 1) fn(e) {
            print(e);
        };
        print(transfer.cancel());
        http::set_max_concurrent(2);
        let job = archive::extract("a.zip", "out", include: ["*"]) fn(e) { print(e); };
        event::wait(job.id);
        archive::create("o.zip", ["a"], base: "") fn(e) { print(e); };
        archive::gzip("a", "a.gz") fn(e) { print(e); };
        archive::gunzip("a.gz", "a") fn(e) { print(e); };
        archive::set_max_concurrent(2);
        let re = regex::compile("a+").unwrap();
        print(re.is_match("a"), re.find("a"), re.find_all("aa"), re.captures("a"));
        print(re.replace_all("a", "b"), re.split("bab"), re.pattern);
        print(toml::parse("a = 1"), toml::stringify(json::parse("{}").unwrap()));
        print(term::is_tty("stderr"), term::size(), term::color("x", "red"), term::bold("x"));
        print(term::dim("x"), term::underline("x"), term::strip("x"));
        let args = cli::parse(["-v"], takes_value: ["o"]).unwrap();
        print(args.flag("v"), args.option("o"), args.positional);
        let socket = websocket::open("ws://x") fn(s, e) {
            print(s.state());
            print(e);
        };
        print(socket.send("t"));
        print(socket.close(code: 1000, reason: "r"));
        print(browser::open_popup("u", "n", ""));
        print(browser::close_popup("n"));
        print(browser::post_message("opener", 1, "*"));
        browser::close_window();
        browser::share({"title": "t"}) fn(r) { print(r); };
        print(browser::can_share({"title": "t"}));
        browser::save_text("a.txt", "text/plain", "hi");
        browser::guard_unload(true);
        print(browser::match_media("(x)"));
        browser::watch_media("(x)") fn(on) { print(on); };
        browser::pick_file(".txt") fn(r) { print(r); };
        browser::request_fullscreen("") fn(r) { print(r); };
        browser::exit_fullscreen();
        print(browser::is_fullscreen());
        browser::on_message() fn(m) { print(m.origin); };
        browser::on_fullscreen_change() fn(id, on) { print(id); };
        print(js::call("Math.max", [1, 2]));
        js::call_async("f", []) fn(r) { print(r); };
        print(js::get("x"));
        js::set("x", 1);
        js::load("./m.js") fn(r) { print(r); };
        print(js::invoke(1, "m", []));
        js::invoke_async(1, "m", []) fn(r) { print(r); };
        print(js::release(1));
        print(js::listen("window", "click") fn(e) { print(e); });
        print(js::unlisten(1));
    }
}
"#;

#[test]
fn every_function_compiles_on_each_side() {
    let dir = scratch("every");
    std::fs::write(dir.join("app.cdl"), EVERY_FUNCTION).unwrap();
    for args in [
        vec!["check", "app.cdl"],
        vec!["check", "--cfg", "web", "app.cdl"],
    ] {
        let out = candela(&dir, &repo().join("libs"), &args);
        assert!(
            out.status.success(),
            "{args:?}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// `browser` and `js` raise `browser_only` off the web, so a page's program
/// still runs on the desktop and can tell.
#[test]
fn browser_and_js_raise_off_the_web() {
    let dir = scratch("browser_only");
    std::fs::write(
        dir.join("app.cdl"),
        r#"
import "std/browser";
import "std/js";

fn main() {
    try {
        browser::is_fullscreen();
    } catch e {
        print(e);
    }
    try {
        js::get("navigator.language");
    } catch e {
        print(e);
    }
}
"#,
    )
    .unwrap();
    let out = candela(&dir, &repo().join("libs"), &["app.cdl"]);
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

/// A library directory laid out as an install lays it out: the modules in
/// `std/`, and the process library as `std_src/process/process.<ext>`.
fn libs_with_process(dir: &Path) -> PathBuf {
    let library = built::build(
        &["-p", "candela-std-process"],
        &format!(
            "{}candela_std_process{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ),
    );
    let libs = dir.join("libs");
    let std_dir = libs.join("std");
    std::fs::create_dir_all(&std_dir).unwrap();
    for entry in std::fs::read_dir(repo().join("libs/std")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "cdl") {
            std::fs::copy(&path, std_dir.join(path.file_name().unwrap())).unwrap();
        }
    }
    let native = libs.join("std_src").join("process");
    std::fs::create_dir_all(&native).unwrap();
    std::fs::copy(
        &library,
        native.join(format!("process.{}", std::env::consts::DLL_EXTENSION)),
    )
    .unwrap();
    libs
}

const CHILD_REPORTS: &str = r#"
import "std/process";

fn main() {
    process::start("CANDELA", ["child.cdl"]) fn(e) {
        match e {
            Stdout(line) => print("out " + line),
            Stderr(line) => print("err " + line),
            Exit(code) => print("exit " + string(code)),
            Failed(why) => print("failed " + why),
        }
    };
    print("main is done");
}
"#;

/// The child is candela itself, running a script that writes two lines and
/// exits with 3.
fn write_child_program(dir: &Path) {
    std::fs::write(
        dir.join("child.cdl"),
        "fn main() {\n    print(\"one\");\n    print(\"two\");\n    exit(3);\n}\n",
    )
    .unwrap();
    let candela_path = env!("CARGO_BIN_EXE_candela").replace('\\', "/");
    std::fs::write(
        dir.join("app.cdl"),
        CHILD_REPORTS.replace("CANDELA", &candela_path),
    )
    .unwrap();
}

/// A program that starts a child and returns from `main` sees every line the
/// child writes and its exit, from a plain source run, and then exits.
#[test]
fn a_process_reports_after_main_returns() {
    let dir = scratch("process_run");
    let libs = libs_with_process(&dir);
    write_child_program(&dir);
    let out = candela(&dir, &libs, &["app.cdl"]);
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

/// The same program built to a `.cdlb` runs under `candela-vm`, which finds
/// the process library in the library directory the way the compiler did.
#[test]
fn a_process_artifact_finds_its_library_under_candela_vm() {
    let dir = scratch("process_artifact");
    let libs = libs_with_process(&dir);
    write_child_program(&dir);
    let built = candela(&dir, &libs, &["build", "app.cdl", "-o", "app.cdlb"]);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let vm = built::build(
        &["-p", "candela-vm", "--bin", "candela-vm"],
        &format!("candela-vm{}", std::env::consts::EXE_SUFFIX),
    );
    let mut command = Command::new(vm);
    command
        .arg("app.cdlb")
        .current_dir(&dir)
        .env("CANDELA_LIB_PATH", &libs);
    let out = common::output_with_deadline(&mut command, "candela-vm app.cdlb");
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

/// `candela build --stage` lays out an app that runs with nothing installed:
/// the artifact, `candela-vm`, and only the native libraries the artifact
/// binds, here the process library and not the net one.
#[test]
fn a_staged_app_carries_only_the_libraries_it_binds() {
    let dir = scratch("stage");
    let install = dir.join("toolchain");
    std::fs::create_dir_all(&install).unwrap();
    let libs = libs_with_process(&dir);
    std::fs::rename(&libs, install.join("libs")).unwrap();
    let exe = std::env::consts::EXE_SUFFIX;
    let candela_bin = install.join(format!("candela{exe}"));
    std::fs::copy(env!("CARGO_BIN_EXE_candela"), &candela_bin).unwrap();
    let vm = built::build(
        &["-p", "candela-vm", "--bin", "candela-vm"],
        &format!("candela-vm{exe}"),
    );
    std::fs::copy(&vm, install.join(format!("candela-vm{exe}"))).unwrap();
    std::fs::write(install.join("LICENSE"), "the license").unwrap();
    std::fs::write(install.join("NOTICE"), "the notice").unwrap();
    write_child_program(&dir);

    let run_fresh = |command: &mut Command, what: &str| {
        // A copied binary can be briefly unexecutable while a sibling test's
        // child still holds the write handle ("text file busy").
        let mut attempt = 0;
        loop {
            match common::try_output_with_deadline(command, what) {
                Ok(output) => break output,
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 50 => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(e) => panic!("{what}: {e}"),
            }
        }
    };
    let mut build = Command::new(&candela_bin);
    build
        .args(["build", "app.cdl", "--stage", "app"])
        .current_dir(&dir)
        .env_remove("CANDELA_LIB_PATH");
    let built = run_fresh(&mut build, "candela build --stage");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let app = dir.join("app");
    let ext = std::env::consts::DLL_EXTENSION;
    assert!(app.join("app.cdlb").is_file());
    assert!(app.join(format!("candela-vm{exe}")).is_file());
    assert!(
        app.join(format!("libs/std_src/process/process.{ext}"))
            .is_file()
    );
    assert!(!app.join("libs/std_src/net").exists());
    // The license files beside the toolchain go with it; one it lacks is not
    // made up.
    assert_eq!(
        std::fs::read_to_string(app.join("LICENSE")).unwrap(),
        "the license"
    );
    assert!(app.join("NOTICE").is_file());
    assert!(!app.join("THIRD-PARTY-LICENSE").exists());

    let mut command = Command::new(app.join(format!("candela-vm{exe}")));
    command
        .arg(app.join("app.cdlb"))
        .current_dir(&dir)
        .env_remove("CANDELA_LIB_PATH");
    let out = run_fresh(&mut command, "staged candela-vm");
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

/// `candela build` says what stops it: a flag missing its value is a misuse,
/// and an output or a stage directory it cannot write, or a toolchain with no
/// `candela-vm` to stage, fails and names the path. `--debug` and
/// `--offline` build as asked.
#[test]
fn a_build_names_what_it_cannot_write_or_stage() {
    let dir = scratch("build_failures");
    let libs = dir.join("libs");
    std::fs::create_dir_all(&libs).unwrap();
    std::fs::write(dir.join("app.cdl"), "fn main() {\n    print(\"hi\");\n}\n").unwrap();
    let stderr = |out: &std::process::Output| String::from_utf8_lossy(&out.stderr).into_owned();

    let out = candela(
        &dir,
        &libs,
        &[
            "build",
            "app.cdl",
            "--debug",
            "--offline",
            "-o",
            "debug.cdlb",
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(dir.join("debug.cdlb").is_file());

    let out = candela(&dir, &libs, &["build", "app.cdl", "--stage"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("--stage needs the directory"),
        "{}",
        stderr(&out)
    );

    let out = candela(&dir, &libs, &["build", "app.cdl", "-o", "missing/app.cdlb"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot write missing/app.cdlb"),
        "{}",
        stderr(&out)
    );

    std::fs::write(dir.join("blocker"), "").unwrap();
    let out = candela(&dir, &libs, &["build", "app.cdl", "--stage", "blocker/app"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot create blocker"),
        "{}",
        stderr(&out)
    );

    // A candela on its own, with no candela-vm beside it.
    let alone = dir.join("alone");
    std::fs::create_dir_all(&alone).unwrap();
    let exe = std::env::consts::EXE_SUFFIX;
    let candela_bin = alone.join(format!("candela{exe}"));
    std::fs::copy(env!("CARGO_BIN_EXE_candela"), &candela_bin).unwrap();
    let mut attempt = 0;
    let out = loop {
        let mut build = Command::new(&candela_bin);
        build
            .args(["build", "app.cdl", "--stage", "staged"])
            .current_dir(&dir)
            .env("CANDELA_LIB_PATH", &libs);
        // A copied binary can be briefly unexecutable while a sibling test's
        // child still holds the write handle ("text file busy").
        match common::try_output_with_deadline(&mut build, "candela build --stage") {
            Ok(output) => break output,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 50 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("candela build --stage: {e}"),
        }
    };
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&format!("there is no candela-vm{exe} beside")),
        "{}",
        stderr(&out)
    );
    std::fs::remove_dir_all(&dir).ok();
}
