//! `std/process` from candela: `process::start` and `Child::stop`, with their
//! events arriving in the closure a program passes.

use candela_std_testkit::{run, scratch, stage};

/// The test program, by a path a candela string literal carries on every
/// platform.
fn child() -> String {
    env!("CARGO_BIN_EXE_candela-process-test-child").replace('\\', "/")
}

/// A program whose `start` runs `call`, reporting every event of the process
/// it starts.
fn program(call: &str) -> String {
    format!(
        r#"
import "std/process";

host "test" {{
    fn seen(text: string);
}}

fn report(e: process::Event) {{
    match e {{
        Stdout(line) => test::seen("out " + line),
        Stderr(line) => test::seen("err " + line),
        Exit(code) => test::seen("exit " + string(code)),
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
        .any(|s| s.starts_with("exit ") || s.starts_with("failed "))
}

#[test]
fn every_line_arrives_and_then_the_exit() {
    let dir = scratch("process-lines");
    let libs = stage(&["process"], &dir);
    let call = format!(
        r#"process::start("{}", ["3", "--lines", "2"]) fn(e) {{ report(e); }};"#,
        child()
    );
    let seen = run(&libs, &dir, &program(&call), ended);
    let out: Vec<&String> = seen.iter().filter(|s| s.starts_with("out ")).collect();
    assert_eq!(
        out,
        ["out 3", "out --lines", "out 2", "out line-1", "out line-2"]
    );
    assert!(seen.contains(&"err child stderr".to_string()), "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("exit 3"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_child_starts_where_and_with_what_it_was_given() {
    let dir = scratch("process-options");
    let libs = stage(&["process"], &dir);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let call = format!(
        r#"process::start("{}", ["0", "--cwd", "--env", "CANDELA_PROCESS_TEST"], cwd: "{}", env: {{"CANDELA_PROCESS_TEST": "set"}}) fn(e) {{ report(e); }};"#,
        child(),
        work.to_string_lossy().replace('\\', "/")
    );
    let seen = run(&libs, &dir, &program(&call), ended);
    assert!(seen.contains(&"out env=set".to_string()), "{seen:?}");
    let cwd = seen
        .iter()
        .find_map(|s| s.strip_prefix("out cwd="))
        .expect("the child reported its directory");
    assert_eq!(
        std::fs::canonicalize(cwd).unwrap(),
        std::fs::canonicalize(&work).unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_program_that_is_not_there_fails_and_says_why() {
    let dir = scratch("process-missing");
    let libs = stage(&["process"], &dir);
    let call = r#"process::start("no-such-program-8f2c") fn(e) { report(e); };"#;
    let seen = run(&libs, &dir, &program(call), ended);
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(
        seen[0].starts_with("failed ") && seen[0].contains("no-such-program-8f2c"),
        "{seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_stop_ends_a_running_child_whose_exit_still_arrives() {
    let dir = scratch("process-stop");
    let libs = stage(&["process"], &dir);
    let call = format!(
        r#"let child = process::Child {{ id: 0 }};
    child = process::start("{}", ["0", "--sleep", "30000"]) fn(e) {{
        report(e);
        if e is Stdout(line) && line == "--sleep" {{
            test::seen("stopped " + string(child.stop()));
        }}
    }};"#,
        child()
    );
    let seen = run(&libs, &dir, &program(&call), ended);
    assert!(seen.contains(&"stopped true".to_string()), "{seen:?}");
    assert!(
        seen.last().is_some_and(|s| s.starts_with("exit ")),
        "{seen:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A child started with `stdin: true` reads what `write` sends, and ends once
/// `close_input` gives it end of file; one started without has nothing to
/// write to.
#[test]
fn a_child_reads_what_is_written_to_its_stdin() {
    let dir = scratch("process-stdin");
    let libs = stage(&["process"], &dir);
    let call = format!(
        r#"let child = process::start("{}", ["0", "--echo-stdin"], stdin: true) fn(e) {{ report(e); }};
    test::seen("wrote " + string(child.write("one\ntwo\n")));
    test::seen("closed " + string(child.close_input()));
    test::seen("again " + string(child.close_input()));
    let plain = process::start("{}", ["0"]) fn(e) {{}};
    test::seen("plain " + string(plain.write("x")));"#,
        child(),
        child()
    );
    let seen = run(&libs, &dir, &program(&call), |seen| {
        seen.iter().any(|s| s == "exit 0")
    });
    assert_eq!(
        &seen[..4],
        ["wrote true", "closed true", "again false", "plain false"]
    );
    let out: Vec<&String> = seen.iter().filter(|s| s.starts_with("out in ")).collect();
    assert_eq!(out, ["out in one", "out in two"], "{seen:?}");
    std::fs::remove_dir_all(&dir).ok();
}
