//! The read-eval-print loop, driven the way a pipe drives it.
//!
//! `candela` with no arguments reads lines, compiles the session so far, and
//! passes what the program printed on to its own stdout. The test feeds it a
//! session and reads both streams back, which is as close to sitting at the
//! prompt as a test gets.

use std::io::Read;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SESSION_DEADLINE: Duration = Duration::from_mins(1);

/// Feeds `session` to the REPL and returns what it wrote to stdout and stderr.
fn session(session: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_candela"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("candela binary runs");

    let mut child_stdin = child.stdin.take().expect("stdin is piped");
    child_stdin
        .write_all(session.as_bytes())
        .expect("hand over the session");
    // Closing the input is the end-of-file the loop leaves on.
    drop(child_stdin);

    let mut child_stdout = child.stdout.take().expect("stdout is piped");
    let mut child_stderr = child.stderr.take().expect("stderr is piped");
    let stdout = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = child_stdout.read_to_end(&mut buffer);
        buffer
    });
    let stderr = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = child_stderr.read_to_end(&mut buffer);
        buffer
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("poll the session") {
            Some(status) => break status,
            None if started.elapsed() > SESSION_DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the REPL never left, and the input had run out");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    let stdout = String::from_utf8_lossy(&stdout.join().expect("read stdout")).into_owned();
    let stderr = String::from_utf8_lossy(&stderr.join().expect("read stderr")).into_owned();
    assert!(
        status.success(),
        "the REPL exited with {:?}\nstdout: {stdout}\nstderr: {stderr}",
        status.code(),
    );
    (stdout, stderr)
}

/// Everything the loop puts on screen: the banner, a prompt per line, what a
/// line printed, the tip that answers `exit()`, and a failed line's error. Only
/// the last of those goes to stderr, and a line that fails is dropped from the
/// session rather than kept, so the lines after it still run.
#[test]
fn a_session_prints_its_prompts_output_and_errors() {
    let (out, err) = session("print(\"one\");\nlet x = ;\nprint(\"two\");\nexit()\n");

    assert!(
        out.contains("REPL"),
        "the banner names the loop, got:\n{out}"
    );
    assert!(
        out.contains("one") && out.contains("two"),
        "each line's output is passed on, got:\n{out}"
    );
    assert!(
        out.contains("To exit, press Ctrl+D"),
        "typing exit() is answered with the way out, got:\n{out}"
    );
    assert_eq!(
        out.matches("> ").count(),
        5,
        "a prompt for each line and one more waiting on the input that ended, got:\n{out}"
    );
    assert!(
        err.contains("Error"),
        "the line that would not compile says why, got:\n{err}"
    );
}
