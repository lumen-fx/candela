//! Behaviour that only a whole program run shows: what the VM prints, which
//! failures a `catch` can reach, and what a run that refuses to start says.
//!
//! Each test writes a `.cdl` file, runs it through the `candela` binary, and
//! reads what came back. The run is given a deadline and killed past it,
//! because the failure these guard against is a program that never finishes
//! rather than one that finishes wrongly.

use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_DEADLINE: Duration = Duration::from_secs(30);

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// What a run left behind.
struct Finished {
    succeeded: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs `source` with `input` on its standard input, and waits for it.
fn finish(name: &str, source: &str, input: &str) -> Finished {
    let dir = std::env::temp_dir().join(format!("candela_rt_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let path = dir.join(format!("{name}.cdl"));
    std::fs::write(&path, source).expect("write program");

    let mut child = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(&path)
        .env("CANDELA_LIB_PATH", repo().join("libs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("candela binary runs");

    // Handing over the input and closing the pipe is what lets a program that
    // reads to the end of its input finish at all.
    let mut child_stdin = child.stdin.take().expect("stdin is piped");
    child_stdin
        .write_all(input.as_bytes())
        .expect("hand over the input");
    drop(child_stdin);

    // The pipes are drained on their own threads. Polling the child while it
    // waits for room in a full pipe would read as a hang and blame the program.
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
        match child.try_wait().expect("poll the run") {
            Some(status) => break status,
            None if started.elapsed() > RUN_DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&dir);
                panic!("{name} never finished; it is stuck rather than raising");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    let stdout = stdout.join().expect("read stdout");
    let stderr = stderr.join().expect("read stderr");
    let _ = std::fs::remove_dir_all(&dir);
    Finished {
        succeeded: status.success(),
        code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    }
}

/// Runs `source` and returns its stdout as lines. Panics when the program
/// fails, or when it outruns the deadline.
fn run(name: &str, source: &str) -> Vec<String> {
    run_reading(name, source, "")
}

/// [`run`] for a program that reads its standard input.
fn run_reading(name: &str, source: &str, input: &str) -> Vec<String> {
    let done = finish(name, source, input);
    assert!(
        done.succeeded,
        "{name} exited with {:?}\nstdout: {}\nstderr: {}",
        done.code, done.stdout, done.stderr,
    );
    done.stdout.lines().map(ToOwned::to_owned).collect()
}

/// Runs `source` expecting it not to get through, and returns what it said on
/// stderr with the terminal colouring taken out.
fn refuse(name: &str, source: &str) -> String {
    let done = finish(name, source, "");
    assert!(
        !done.succeeded,
        "{name} was expected to stop, and it ran to the end\nstdout: {}",
        done.stdout,
    );
    strip_ansi(&done.stderr)
}

/// Drops the SGR escapes ariadne colours a report with, so an assertion can
/// read the text the way a person does.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        for escape in chars.by_ref() {
            if escape.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// A function body is compiled inline at its first call site and records an
/// absolute entry address, so a body first compiled inside a `try` block used to
/// be entered at the wrong address and the program never came back.
#[test]
fn a_call_inside_try_returns() {
    let out = run(
        "call_in_try",
        r#"
fn add(a, b) {
    return a + b;
}

fn main() {
    try {
        print(add(1, 2));
    } catch e {
        print("caught " + e);
    }
    print("after");
}
"#,
    );
    assert_eq!(out, ["3", "after"]);
}

/// A recursive call saves the registers the caller still needs afterwards, and
/// the return puts them back. A function that ends without a value returns
/// through a different instruction, which used to skip the restore, so every
/// level read the deepest call's `n` and printed `up -1` four times over.
#[test]
fn a_void_recursive_call_gives_the_caller_its_locals_back() {
    let out = run(
        "void_recursion",
        r#"
fn countdown(n) {
    if n < 0 {
        return;
    }
    countdown(n - 1);
    print("up " + str(n));
}

fn main() {
    let tally = 11;
    countdown(3);
    print("tally " + str(tally));
}
"#,
    );
    assert_eq!(out, ["up 0", "up 1", "up 2", "up 3", "tally 11"]);
}

/// The same restore has to happen when the recursion ends by falling off the
/// end of the body rather than at a `return`, and when two functions call each
/// other rather than one calling itself.
#[test]
fn mutual_void_recursion_unwinds_in_order() {
    let out = run(
        "mutual_void_recursion",
        r#"
fn ping(n) {
    if n > 0 {
        let mine = "ping " + str(n);
        pong(n - 1);
        print(mine);
    }
}

fn pong(n) {
    if n > 0 {
        let mine = "pong " + str(n);
        ping(n - 1);
        print(mine);
    }
}

fn main() {
    ping(4);
}
"#,
    );
    assert_eq!(out, ["pong 1", "ping 2", "pong 3", "ping 4"]);
}

#[test]
fn a_throw_below_the_block_reaches_the_catch() {
    let out = run(
        "throw_below",
        r#"
fn refuse(amount) {
    if amount > 10 {
        throw("too_much");
    }
    return amount;
}

fn spend(amount) {
    return refuse(amount);
}

fn main() {
    try {
        print(spend(50));
    } catch "too_much" {
        print("declined");
    }
    print(spend(5));
}
"#,
    );
    assert_eq!(out, ["declined", "5"]);
}

/// A call made inside a `try` saves the caller's live registers, and a throw
/// out of that call never reaches the return that would put them back. The
/// catch does it instead. It used to leave the save behind, so every level
/// above the catch read one call too deep and `dig` printed 10, 10, 20 on its
/// way out instead of counting up to 30.
#[test]
fn a_catch_hands_the_resuming_call_its_registers_back() {
    let out = run(
        "catch_restores_registers",
        r#"
fn dig(n) {
    if n <= 0 {
        throw("bottom");
    }
    let mine = n * 10;
    try {
        dig(n - 1);
    } catch e {
        print("caught at " + str(mine));
    }
    print("after " + str(mine));
}

fn main() {
    dig(3);
}
"#,
    );
    assert_eq!(out, ["caught at 10", "after 10", "after 20", "after 30"]);
}

/// A recursion that returns a value leaves through a different instruction than
/// one that returns nothing, and both restore the caller's registers, so a
/// catch has to unwind to the same place for either.
#[test]
fn a_catch_in_a_recursion_that_returns_a_value_unwinds_the_same() {
    let out = run(
        "catch_value_recursion",
        r#"
fn sum(n) {
    if n <= 0 {
        throw("bottom");
    }
    let mine = n * 10;
    let got = 0;
    try {
        got = sum(n - 1);
    } catch e {
        got = 1;
    }
    print("level " + str(mine) + " got " + str(got));
    return mine + got;
}

fn main() {
    print("total " + str(sum(3)));
}
"#,
    );
    assert_eq!(
        out,
        [
            "level 10 got 1",
            "level 20 got 11",
            "level 30 got 31",
            "total 61"
        ]
    );
}

/// One catch can end several calls at once. The frame that resumes takes back
/// what its own call saved; the levels below it are gone and what they saved
/// goes with them.
#[test]
fn a_throw_across_several_calls_leaves_one_level_to_restore() {
    let out = run(
        "catch_across_levels",
        r#"
fn dig(n) {
    if n <= 0 {
        throw("bottom");
    }
    let mine = n * 10;
    if n == 3 {
        try {
            dig(n - 1);
        } catch e {
            print("caught at " + str(mine));
        }
    } else {
        dig(n - 1);
    }
    print("after " + str(mine));
}

fn main() {
    dig(5);
}
"#,
    );
    assert_eq!(out, ["caught at 30", "after 30", "after 40", "after 50"]);
}

/// A catch that throws again is a throw from a frame that has already been
/// unwound once, and the next catch up unwinds from there.
#[test]
fn a_throw_from_inside_a_catch_unwinds_the_frames_above_it() {
    let out = run(
        "rethrow_from_catch",
        r#"
fn dig(n) {
    if n <= 0 {
        throw("bottom");
    }
    let mine = n * 10;
    try {
        dig(n - 1);
    } catch e {
        if n < 3 {
            throw("rethrown at " + str(mine));
        }
        print("caught " + e + " at " + str(mine));
    }
    print("after " + str(mine));
}

fn main() {
    dig(4);
}
"#,
    );
    assert_eq!(out, ["caught rethrown at 20 at 30", "after 30", "after 40"]);
}

/// Two `try` blocks in one frame start at the same depth, so the second catch
/// has nothing left to unwind and has to leave the registers the first one
/// restored alone.
#[test]
fn nested_catches_in_one_frame_restore_once() {
    let out = run(
        "nested_catch_one_frame",
        r#"
fn dig(n) {
    if n <= 0 {
        throw("bottom");
    }
    let mine = n * 10;
    try {
        try {
            dig(n - 1);
        } catch inner {
            print("inner at " + str(mine));
            throw("again");
        }
    } catch outer {
        print("outer at " + str(mine));
    }
    print("after " + str(mine));
}

fn main() {
    dig(3);
}
"#,
    );
    assert_eq!(
        out,
        [
            "inner at 10",
            "outer at 10",
            "after 10",
            "after 20",
            "after 30"
        ]
    );
}

/// An invalid operation answers with a quiet NaN whose sign bit is set, which is
/// the bit pattern the enum tag uses. Printing one used to walk the enum table
/// and abort the process.
#[test]
fn not_a_number_and_infinities_print() {
    let out = run(
        "nan_print",
        r"
fn main() {
    let neg = 0.0 - 1.0;
    let nan = neg.sqrt();
    print(nan);
    print(type(nan));
    print(1.0 / 0.0);
    print(neg / 0.0);
    print([nan]);
}
",
    );
    assert_eq!(out, ["NaN", "float", "inf", "-inf", "[NaN]"]);
}

/// Each type has its own printing arm, and what each one writes is part of the
/// language: `print` is how a program says anything at all, and the shapes are
/// documented alongside it.
#[test]
fn every_type_prints_in_its_documented_shape() {
    let out = run(
        "print_shapes",
        r#"
struct Point {
    x: int,
    y: int,
}

enum Shape {
    Dot,
    Line(int, int),
}

fn main() {
    print("text");
    print(9);
    print(2.5);
    print(3.0);
    print(false);
    print([1, 2, 3]);
    print([[1, 2], [3]]);
    print(Point { x: 1, y: 2 });
    print(Shape::Dot);
    print(Shape::Line(4, 5));
    print({"only": 1});
}
"#,
    );
    assert_eq!(
        out,
        [
            "text",
            "9",
            "2.5",
            "3",
            "false",
            "[1,2,3]",
            "[[1,2],[3]]",
            "Point {x:1,y:2}",
            "Dot",
            "Line(4,5)",
            "{\"only\":1}",
        ]
    );
}

/// A map with more than one entry separates them with a comma. The order they
/// come out in is the map's, which is not the order they went in, so the test
/// asks for the entries rather than for a line.
#[test]
fn a_map_prints_every_entry_separated_by_commas() {
    let out = run(
        "print_map",
        r#"
fn main() {
    print({"a": 1, "b": 2, "c": 3});
}
"#,
    );
    let [line] = out.as_slice() else {
        panic!("one line of output, got {out:?}");
    };
    let inside = line
        .strip_prefix('{')
        .and_then(|l| l.strip_suffix('}'))
        .unwrap_or_else(|| panic!("a map prints inside braces, got {line}"));
    let mut entries: Vec<&str> = inside.split(',').collect();
    entries.sort_unstable();
    assert_eq!(entries, ["\"a\":1", "\"b\":2", "\"c\":3"]);
}

/// `input` writes its prompt without a newline and flushes it, so the prompt is
/// on screen before the program blocks on the answer.
#[test]
fn input_writes_its_prompt_then_takes_the_line() {
    let out = run_reading(
        "input_prompt",
        r#"
fn main() {
    let name = input("name? ");
    print("hi " + name);
}
"#,
        "ada\n",
    );
    assert_eq!(out, ["name? hi ada"]);
}

/// The parser rejects a negative literal exponent, so only a value computed at
/// run time reaches the instruction. It used to cast to an unsigned exponent and
/// return a wrong number.
#[test]
fn a_negative_exponent_raises_at_run_time() {
    let out = run(
        "negative_exponent",
        r#"
fn main() {
    let exponent = 0 - 2;
    try {
        print(2 ^ exponent);
    } catch "negative_exponent" {
        print("raised");
    }
    print(2 ^ 10);
    print(2.0 ^ (0.0 - 2.0));
}
"#,
    );
    assert_eq!(out, ["raised", "1024", "0.25"]);
}

/// A run that does not get past the parser still has something to say, and the
/// report is written to stderr rather than raised there: the reader has to see
/// the error in their source, not one about the stream it was going to.
#[test]
fn a_syntax_error_prints_a_report_and_stops() {
    let report = refuse(
        "missing_semicolon",
        r#"
fn main() {
    print("hi")
}
"#,
    );
    assert!(
        report.contains("Missing semicolon"),
        "the report names the error, got:\n{report}"
    );
    assert!(
        report.contains("Add a ; here"),
        "the report says what to do about it, got:\n{report}"
    );
    assert!(
        report.contains("missing_semicolon.cdl:3:16"),
        "the report points at the source, got:\n{report}"
    );
}

/// An unclosed delimiter is reported against the opener, not the end of the
/// file, so a long body does not send the reader looking at the wrong line.
#[test]
fn an_unclosed_delimiter_is_reported_against_its_opener() {
    let report = refuse(
        "unclosed",
        r#"fn main() {
    print("hi");
"#,
    );
    assert!(
        report.contains("Unclosed delimiter"),
        "the report names the error, got:\n{report}"
    );
    assert!(
        report.contains("This '{' is never closed"),
        "the report points at the opener, got:\n{report}"
    );
    assert!(
        report.contains("unclosed.cdl:1:11"),
        "the report points at the opening line, got:\n{report}"
    );
}

/// `program | head -1` leaves a pipe with no reader, and every `print` after
/// that fails. The write used to be unwrapped, so the program died on the first
/// line nobody was there to take. Losing the line is the price; losing the run
/// is not, and an embedding host runs candela on a thread it needs back.
#[test]
fn output_a_reader_left_behind_costs_the_line_not_the_run() {
    let dir = std::env::temp_dir().join(format!("candela_pipe_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let path = dir.join("closed_pipe.cdl");
    // More output than a pipe holds, so the program is still writing when the
    // reader goes away rather than already finished.
    std::fs::write(
        &path,
        r#"
fn main() {
    let i = 0;
    while i < 20000 {
        print("a line of program output");
        i = i + 1;
    }
}
"#,
    )
    .expect("write program");

    let mut child = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(&path)
        .env("CANDELA_LIB_PATH", repo().join("libs"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("candela binary runs");

    let mut reader = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut first = String::new();
    reader.read_line(&mut first).expect("read the first line");
    assert_eq!(first.trim_end(), "a line of program output");
    drop(reader);

    let mut child_stderr = child.stderr.take().expect("stderr is piped");
    let stderr = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = child_stderr.read_to_end(&mut buffer);
        buffer
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait().expect("poll the run") {
            Some(status) => break status,
            None if started.elapsed() > RUN_DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&dir);
                panic!("the run never finished after its reader left");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    let stderr = stderr.join().expect("read stderr");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        status.success(),
        "the run exited with {:?}\nstderr: {}",
        status.code(),
        String::from_utf8_lossy(&stderr),
    );
}

#[test]
fn fs_append_creates_a_missing_file() {
    let target = std::env::temp_dir().join(format!("candela_append_{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&target);
    let path = target.to_string_lossy().replace('\\', "\\\\");

    let source = format!(
        r#"
fn main() {{
    fs::append("{path}", "one\n");
    fs::append("{path}", "two\n");
    print(fs::read("{path}"));
}}
"#
    );
    let out = run("fs_append", &source);
    let _ = std::fs::remove_file(&target);
    // The file ends in a newline, so the captured output carries a final
    // empty line.
    assert_eq!(out, ["one", "two", ""]);
}

/// A slice runs from start to end with `0 <= start <= end <= len`, so a start
/// sitting on the end of the value is in range and produces an empty one.
#[test]
fn a_slice_starting_at_the_end_is_empty() {
    let out = run(
        "slice_at_end",
        r#"
fn main() {
    let xs = [1, 2, 3];
    print(xs[3..3]);
    print([][0..0]);
    print("abc"[3..3] == "");
    try {
        print(xs[4..4]);
    } catch "slice_out_of_bounds" {
        print("past the end raises");
    }
}
"#,
    );
    assert_eq!(out, ["[]", "[]", "true", "past the end raises"]);
}

/// json values are read by recursive descent, so a long run of openers used to
/// exhaust the native stack and kill the process.
#[test]
fn deeply_nested_json_raises_rather_than_dying() {
    let source = format!(
        r#"
fn main() {{
    try {{
        print(json_parse("{}"));
    }} catch "json_parse_error" {{
        print("raised");
    }}
}}
"#,
        "[".repeat(200_000)
    );
    assert_eq!(run("deep_json", &source), ["raised"]);
}

/// A parsed document is the one thing that puts lists, maps and pooled strings
/// in each other, and holds them across thousands of later allocations. Every
/// collection past the first garbage collection used to walk off the end of a
/// mark vector and kill the process, and a string the document still held
/// could be handed out again to a later value.
#[test]
fn a_parsed_document_survives_later_allocation() {
    let entries: Vec<String> = (0..200)
        .map(|i| {
            format!(
                "{{\\\"id\\\": {i}, \\\"name\\\": \\\"a-fairly-long-entry-name-{i}\\\", \
                 \\\"tags\\\": [\\\"alpha-tag\\\", \\\"beta-tag\\\"]}}"
            )
        })
        .collect();
    let document = format!("[{}]", entries.join(", "));
    let source = format!(
        r#"
fn main() {{
    let doc = as_list(json_parse("{document}"));
    let n = 0;
    let text = "";
    while n < 5000 {{
        let a_map = {{"key-number-here": n}};
        let a_list = [n, n + 1, n + 2];
        text = "a-string-built-at-run-time-" + str(n);
        n = n + 1;
    }}
    print(doc.len());
    let last = as_map(doc[199]);
    print(as_int(last.get("id")));
    print(as_str(last.get("name")));
    print(as_str(as_list(last.get("tags"))[1]));
}}
"#
    );
    assert_eq!(
        run("parsed_document_survives", &source),
        ["200", "199", "a-fairly-long-entry-name-199", "beta-tag"]
    );
}

/// `return` with no value used to compile to nothing at all, so a guard clause
/// fell straight through into the body it was written to skip.
#[test]
fn a_bare_return_leaves_the_function() {
    let out = run(
        "bare_return",
        r#"
fn guard(x) {
    if x == 1 {
        return;
    }
    print("body ran");
}

fn main() {
    guard(1);
    print("next");
    guard(2);
}
"#,
    );
    assert_eq!(out, ["next", "body ran"]);
}

/// A bare `return` leaves the whole function, not just the block it sits in.
#[test]
fn a_bare_return_in_a_nested_if_leaves_the_function() {
    let out = run(
        "bare_return_nested",
        r#"
fn guard(x, y) {
    if x == 1 {
        if y == 1 {
            return;
        }
        print("inner body");
    }
    print("outer body");
}

fn main() {
    guard(1, 1);
    print("next");
    guard(1, 2);
}
"#,
    );
    assert_eq!(out, ["next", "inner body", "outer body"]);
}

/// The same in an `else`, which lowers through its own jump.
#[test]
fn a_bare_return_in_an_else_branch_leaves_the_function() {
    let out = run(
        "bare_return_else",
        r#"
fn guard(x) {
    if x == 1 {
        print("then");
    } else {
        return;
    }
    print("after the if");
}

fn main() {
    guard(2);
    print("next");
    guard(1);
}
"#,
    );
    assert_eq!(out, ["next", "then", "after the if"]);
}

/// A bare `return` in a loop body ends the function rather than the loop, so it
/// is not a `break`: the statements after the loop are skipped too.
#[test]
fn a_bare_return_in_a_loop_body_leaves_the_function() {
    let out = run(
        "bare_return_loop",
        r#"
fn count_while(limit) {
    let i = 0;
    while i < limit {
        if i == 2 {
            return;
        }
        print("while " + str(i));
        i += 1;
    }
    print("after the while");
}

fn count_for(limit) {
    for i in 0..limit {
        if i == 2 {
            return;
        }
        print("for " + str(i));
    }
    print("after the for");
}

fn main() {
    count_while(5);
    count_for(5);
    print("done");
}
"#,
    );
    assert_eq!(out, ["while 0", "while 1", "for 0", "for 1", "done"]);
}

/// A bare `return` as the first statement of a body skips all of it.
#[test]
fn a_bare_return_at_the_top_of_a_body_skips_the_rest() {
    let out = run(
        "bare_return_first",
        r#"
fn nothing() {
    return;
    print("body ran");
}

fn main() {
    nothing();
    print("done");
}
"#,
    );
    assert_eq!(out, ["done"]);
}

/// A bare `return` where the body would have ended anyway keeps working: the
/// body runs, and the extra return changes nothing.
#[test]
fn a_bare_return_as_the_last_statement_still_runs_the_body() {
    let out = run(
        "bare_return_last",
        r#"
fn greet(name) {
    print("hello " + name);
    return;
}

fn main() {
    greet("world");
    print("done");
}
"#,
    );
    assert_eq!(out, ["hello world", "done"]);
}

/// `main` is compiled inline at the program's top level, where there is no call
/// frame to pop, so a `return` there ends the program. Returning a value does
/// the same, and the value is dropped the way the value of `main` is.
#[test]
fn a_return_at_the_program_top_level_ends_the_program() {
    let out = run(
        "top_level_return",
        r#"
fn main() {
    print("before");
    return;
    print("after");
}
"#,
    );
    assert_eq!(out, ["before"]);

    let out = run(
        "top_level_return_value",
        r#"
fn main() {
    let i = 0;
    while i < 5 {
        if i == 2 {
            return 7;
        }
        print("while " + str(i));
        i += 1;
    }
    print("after the while");
}
"#,
    );
    assert_eq!(out, ["while 0", "while 1"]);
}
