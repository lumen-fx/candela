//! The read-eval-print loop, driven the way a pipe drives it.
//!
//! `candela` with no arguments reads lines, compiles the session so far, and
//! passes what the program printed on to its own stdout. The test feeds it a
//! session and reads both streams back, which is as close to sitting at the
//! prompt as a test gets.

use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod common;

const SESSION_DEADLINE: Duration = Duration::from_mins(1);

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A scratch directory of this test binary's own, for the files a session
/// writes and the programs a test compares a session against.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("candela_repl_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

/// Runs `source` as a file and returns the status it left with, so a test can
/// hold the prompt to what the same line does in a program.
fn file_status(name: &str, source: &str) -> Option<i32> {
    let dir = scratch(name);
    let path = dir.join("prog.cdl");
    std::fs::write(&path, source).expect("write the program");
    let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
    command
        .arg(&path)
        .env("CANDELA_LIB_PATH", repo().join("libs"));
    let output = common::output_with_deadline(&mut command, "program run");
    let _ = std::fs::remove_dir_all(&dir);
    output.status.code()
}

/// Feeds `session` to the REPL and returns what it wrote to stdout and
/// stderr, having left with no failure.
fn session(session: &str) -> (String, String) {
    let (status, stdout, stderr) = session_status(session);
    assert_eq!(
        status,
        Some(0),
        "the REPL exited with {status:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    (stdout, stderr)
}

/// Feeds `session` to the REPL and returns the status it left with, and what
/// it wrote to stdout and stderr.
fn session_status(session: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_candela"))
        .env("CANDELA_LIB_PATH", repo().join("libs"))
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
    (status.code(), stdout, stderr)
}

/// Everything the loop puts on screen: the banner, the way out, a prompt per
/// line, what a line printed, and a failed line's error. Only the last of
/// those goes to stderr, and a line that fails is dropped from the session
/// rather than kept, so the lines after it still run.
#[test]
fn a_session_prints_its_prompts_output_and_errors() {
    let (out, err) = session("print(\"one\");\nlet x = ;\nprint(\"two\");\n");

    assert!(
        out.contains("REPL"),
        "the banner names the loop, got:\n{out}"
    );
    assert!(
        out.contains("Type exit to leave"),
        "the banner says how to leave, got:\n{out}"
    );
    assert!(
        out.contains("one") && out.contains("two"),
        "each line's output is passed on, got:\n{out}"
    );
    assert_eq!(
        out.matches("> ").count(),
        4,
        "a prompt for each line and one more waiting on the input that ended, got:\n{out}"
    );
    assert!(
        err.contains("Error"),
        "the line that would not compile says why, got:\n{err}"
    );
}

/// Every declaration a file accepts is accepted at the prompt as well. The
/// line goes above the synthesised `main` rather than inside it, which is what
/// gives a method block a type to attach to and keeps a function declaration
/// out of a block.
#[test]
fn a_line_declares_everything_a_file_can() {
    let (out, err) = session(concat!(
        "struct Point { x: int }\n",
        "impl Point { fn twice(self) -> int { return self.x * 2; } }\n",
        "enum Colour { Red, Green }\n",
        "fn add(a: int, b: int) -> int { return a + b; }\n",
        "let p = Point{x: 21};\n",
        "print(p.twice());\n",
        "print(Colour::Red);\n",
        "print(add(2, 3));\n",
        "@cfg(nothing) fn add(a: int, b: int) -> int { return 0; }\n",
        "@cfg(nothing) print(\"dropped\");\n",
    ));

    assert!(
        out.contains("42"),
        "the method block declared a method to call, got:\n{out}\n{err}"
    );
    assert!(
        out.contains("Red"),
        "the enum declared a variant to name, got:\n{out}\n{err}"
    );
    assert!(
        out.contains('5'),
        "the function declared is callable, got:\n{out}\n{err}"
    );
    assert!(
        !out.contains("dropped") && err.is_empty(),
        "a line under a false @cfg is dropped, got:\n{out}\n{err}"
    );
}

/// A line the parser cannot read reports what it expected and is dropped, and
/// the prompt takes the next line as it would any other.
#[test]
fn a_line_the_parser_rejects_is_reported_and_dropped() {
    let (out, err) = session("impl {\nprint(\"still here\");\n");

    assert!(
        err.contains("Error"),
        "the line that would not parse says why, got:\n{err}"
    );
    assert!(
        out.contains("still here"),
        "the line after it runs, got:\n{out}"
    );
}

/// A line that is an expression shows its value, which is what the loop is
/// named for. A line the prompt reads as an expression and which turns out to
/// be a statement compiles as it was typed, so a call that yields nothing
/// prints what it prints and nothing more.
#[test]
fn an_expression_line_prints_its_value() {
    let (out, err) = session(concat!(
        "let x = 21;\n",
        "x\n",
        "x * 2\n",
        "\"hi\"\n",
        "print(\"once\")\n",
    ));

    assert!(
        out.contains("21"),
        "a bare name shows what it holds, got:\n{out}\n{err}"
    );
    assert!(
        out.contains("42"),
        "an operator shows its result, got:\n{out}\n{err}"
    );
    assert!(
        out.contains("hi"),
        "a literal shows itself, got:\n{out}\n{err}"
    );
    assert_eq!(
        out.matches("once").count(),
        1,
        "a call that prints and yields nothing prints once, got:\n{out}\n{err}"
    );
}

/// A name nothing declares is an unknown variable, reported against the line
/// as it was typed. The column says which: the prompt tries an expression
/// wrapped in a `print` first, and an error naming that wrapper would point at
/// text nobody wrote.
#[test]
fn an_unknown_name_line_is_reported_and_dropped() {
    let (out, err) = session("nosuchname\nprint(\"after\");\n");

    assert!(
        err.contains("Unknown variable") && err.contains("Cannot find variable"),
        "the unknown name says what it is, got:\n{err}"
    );
    assert!(
        err.contains(":2:1"),
        "the report points at the line as typed, got:\n{err}"
    );
    assert!(out.contains("after"), "the line after it runs, got:\n{out}");
}

/// `exit` leaves, with no failure to report. The built-in of that name ends
/// the process that compiles the session, so a session holding the line would
/// print nothing ever again; the prompt answers it itself and reads no further
/// line.
#[test]
fn exit_leaves_the_session() {
    let (out, err) = session("print(\"before\");\nexit\nprint(\"never\");\n");

    assert!(
        out.contains("before"),
        "the line before it ran, got:\n{out}\n{err}"
    );
    assert!(
        !out.contains("never"),
        "the lines after it are never read, got:\n{out}"
    );
    assert_eq!(
        out.matches("> ").count(),
        2,
        "one prompt for each of the two lines read, got:\n{out}"
    );
}

/// `exit` carrying a status leaves with it, the same way the bare form leaves
/// with none. The status used to reach the built-in in the process compiling
/// the session, where it read as a line that failed and was dropped without a
/// word.
#[test]
fn exit_with_a_status_leaves_the_session_with_it() {
    let (status, out, err) = session_status("print(\"before\");\nexit(2)\nprint(\"never\");\n");

    assert_eq!(
        status,
        Some(2),
        "the status typed is the status left with, got:\n{out}\n{err}"
    );
    assert!(
        out.contains("before"),
        "the line before it ran, got:\n{out}\n{err}"
    );
    assert!(
        !out.contains("never"),
        "the lines after it are never read, got:\n{out}"
    );
    assert!(
        err.is_empty(),
        "leaving is not a failure to report, got:\n{err}"
    );
}

/// A line that compiles and then fails at run time has already had whatever
/// effect it had. It is dropped, like a line the compiler refused, but it is
/// not run again: the prompt tries the line in a second form only when the
/// first did not compile.
#[test]
fn a_line_that_fails_at_run_time_runs_once() {
    let dir = scratch("runs_once");
    let counter = dir.join("counter");
    let (out, err) = session(&format!(
        "import \"std/fs\";\n\
         fn bump() {{ fs::append(\"{}\", \"x\"); return [1, 2][9]; }}\n\
         bump()\n\
         print(\"after\");\n",
        counter.display()
    ));

    let written = std::fs::read_to_string(&counter).unwrap_or_default();
    assert_eq!(
        written.len(),
        1,
        "the line ran once, got {written:?}\n{out}\n{err}"
    );
    assert!(
        err.contains("index"),
        "the run-time failure is reported, got:\n{err}"
    );
    assert!(out.contains("after"), "the line after it runs, got:\n{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `exit` takes any argument the session can work out, not only one written
/// out: the line is compiled in the session, so `exit(code)` leaves with what
/// the lines before it bound to `code`.
#[test]
fn exit_leaves_with_a_status_the_session_works_out() {
    let (status, out, err) = session_status("let code = 5;\nexit(code)\nprint(\"never\");\n");

    assert_eq!(status, Some(5), "stdout: {out}\nstderr: {err}");
    assert!(
        !out.contains("never"),
        "the lines after it are never read, got:\n{out}"
    );
}

/// Leaving through the prompt and leaving from a file are the same call, so
/// they leave with the same status. A status wider than a byte is where the
/// two used to part.
#[test]
fn the_prompt_leaves_the_way_a_program_leaves() {
    let (status, out, err) = session_status("exit(300)\n");
    assert_eq!(
        status,
        file_status("wide_status", "fn main() { let a = 1; exit(300); }\n"),
        "stdout: {out}\nstderr: {err}"
    );
}

/// An `exit` the compiler cannot make sense of is a line like any other: it is
/// reported, dropped, and the session goes on.
#[test]
fn an_exit_that_does_not_compile_is_dropped() {
    let (status, out, err) = session_status("exit(nosuchname)\nprint(\"still here\");\n");

    assert_eq!(status, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(
        err.contains("Unknown variable"),
        "the argument that names nothing says so, got:\n{err}"
    );
    assert!(
        out.contains("still here"),
        "the line after it runs, got:\n{out}"
    );
}

/// The declaration forms that reach outside the file are typed at the prompt
/// too: an `import` brings a library module in, and a `host` block declares
/// the functions an embedding host would bind. A `dylib` block naming a
/// library that cannot be loaded is reported and dropped, and the session
/// carries on.
#[test]
fn a_line_declares_what_reaches_outside_the_file() {
    let (out, err) = session(concat!(
        "import \"std/math\";\n",
        "print(math::abs(0 - 3));\n",
        "host \"app\" { int rows(string); }\n",
        "dylib \"no_such_library_here\" { float sqrt(float); }\n",
        "print(\"still here\");\n",
    ));

    assert!(
        out.contains('3'),
        "the imported module is reachable, got:\n{out}\n{err}"
    );
    assert!(
        err.contains("dynamic library"),
        "the library that cannot be loaded says so, got:\n{err}"
    );
    assert!(
        out.contains("still here"),
        "the lines after both run, got:\n{out}\n{err}"
    );
}

/// A line is read as an expression from its opening tokens, and the reading
/// stops where the arguments start: the `=` of a `let` inside a closure passed
/// as an argument belongs to the closure, not to the line.
#[test]
fn an_expression_line_holding_a_closure_prints_its_value() {
    let (out, err) = session(concat!(
        "fn apply(f, x) { return f(x); }\n",
        "apply(fn(n) { let y = n; return y * 3; }, 5)\n",
    ));

    assert!(
        out.contains("15"),
        "the line's value is shown, got:\n{out}\n{err}"
    );
}

/// A line opening with `{` is a block, which is what `parse_statement` reads
/// there, rather than a map literal.
#[test]
fn a_line_opening_with_a_brace_is_a_block() {
    let (out, err) = session("{ let z = 4; print(z * 2); }\n");

    assert!(
        out.contains('8'),
        "the block runs its statements, got:\n{out}\n{err}"
    );
}
