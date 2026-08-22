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
