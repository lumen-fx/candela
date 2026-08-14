//! Runtime behaviour that only a whole program run shows: what the VM prints,
//! and which failures a `catch` can reach.
//!
//! Each test writes a `.cdl` file, runs it through the `candela` binary, and
//! reads stdout. The run is given a deadline and killed past it, because the
//! failure these guard against is a program that never finishes rather than one
//! that finishes wrongly.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const RUN_DEADLINE: Duration = Duration::from_secs(30);

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Runs `source` and returns its stdout as lines. Panics when the program
/// fails, or when it outruns the deadline.
fn run(name: &str, source: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("candela_rt_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch directory");
    let path = dir.join(format!("{name}.cdl"));
    std::fs::write(&path, source).expect("write program");

    let mut child = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg(&path)
        .env("CANDELA_LIB_PATH", repo().join("libs"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("candela binary runs");

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
    assert!(
        status.success(),
        "{name} exited with {:?}\nstdout: {}\nstderr: {}",
        status.code(),
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
    String::from_utf8_lossy(&stdout)
        .lines()
        .map(ToOwned::to_owned)
        .collect()
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
