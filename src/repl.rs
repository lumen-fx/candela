// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::compiler::imports::ImportResolver;
use crate::errors::BLUE;
use crate::errors::RESET;
use crate::parser::ExitLine;
use crate::parser::LineKind;
use crate::parser::classify_line;
use crate::parser::exit_line;
use std::io::{Write, stderr, stdin, stdout};
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::process::Output;

/// Writes to the REPL's stdout, and drops the text when the write fails.
///
/// The prompt and the output of every line evaluated go through here. A stdout
/// whose reader has exited fails every write, and the session ending on the
/// first of those would take the loop down over text nobody is reading.
macro_rules! say {
    ($($arg:tt)*) => {{
        let _ = write!(stdout(), $($arg)*);
    }};
}

/// One accepted line of the session, in the form that compiled.
struct Line {
    text: String,
    /// Whether the line declares something at the top level, which goes above
    /// the synthesised `main` rather than inside it.
    declaration: bool,
}

/// Writes the session out as a program: the declarations first, then a `main`
/// holding everything else, each group in the order the lines were typed.
fn program(lines: &[Line], contents: &mut String) {
    contents.clear();
    for line in lines.iter().filter(|line| line.declaration) {
        contents.push_str(&line.text);
        contents.push('\n');
    }
    contents.push_str("fn main() {\n");
    for line in lines.iter().filter(|line| !line.declaration) {
        contents.push_str(&line.text);
        contents.push('\n');
    }
    contents.push_str("\n}");
}

/// The forms a typed line is tried in, best first.
///
/// An expression is shown as `print(line)`, because showing a value is what a
/// read-eval-print loop is for. Whether the reading holds is settled by
/// compiling it: a line that turns out to be a statement compiles in the
/// second form, as it was typed.
fn forms(line: &str, kind: LineKind) -> Vec<String> {
    let mut forms = Vec::with_capacity(2);
    if kind == LineKind::Expression {
        forms.push(format!("print({});", line.trim_end().trim_end_matches(';')));
    }
    forms.push(as_typed(line));
    forms
}

/// The line as it was typed, ended with a semicolon where it needs one.
fn as_typed(line: &str) -> String {
    let mut typed = line.to_owned();
    // A line left open on a '{' has nothing to end with a semicolon, and one
    // placed after it turns up in the middle of the error the line earns.
    if !typed.ends_with([';', '}', '{']) {
        typed.push(';');
    }
    typed
}

/// Compiles and runs `contents` as a program, and hands back what it wrote.
///
/// The session runs in a process of its own, so a line the compiler rejects
/// reports its error there and leaves the prompt to read the next line.
fn evaluate(exe: &Path, tmp: &Path, contents: &str) -> Output {
    std::fs::write(tmp, contents)
        .unwrap_or_else(|e| panic!("[ERROR] Cannot write the session file: {e}"));
    let output = std::process::Command::new(exe)
        .arg(tmp)
        .output()
        .unwrap_or_else(|e| panic!("[ERROR] Cannot run candela: {e}"));
    // The next line writes the file again, so a removal that does not happen
    // costs nothing beyond the file staying behind.
    let _ = std::fs::remove_file(tmp);
    output
}

/// Whether `contents` compiles, without running any of it.
///
/// This is what tells a line the compiler refused from a line that ran and
/// then failed. The first never had an effect, so the prompt is free to try
/// the line in another form; the second already had whatever effect it had,
/// and running it again would repeat it.
///
/// The compile runs here rather than in a process of its own: it is the same
/// compile the run does, and a diagnostic comes back as a value instead of
/// ending the prompt.
fn compiles(tmp: &Path, contents: &str) -> bool {
    crate::collect_diagnostic(|| {
        crate::compiler::compile(
            String::from(contents),
            &tmp.to_string_lossy(),
            false,
            &ImportResolver::new(),
        )
    })
    .is_ok()
}

/// A file in the temporary directory nothing else holds, for the session to be
/// compiled from.
///
/// Created rather than named: a path worked out from the process id is one
/// another program can guess and leave a symbolic link at, and writing the
/// session would then follow that link and write wherever it pointed.
fn session_file() -> PathBuf {
    let dir = std::env::temp_dir();
    for attempt in 0..u16::MAX {
        let path = dir.join(format!("candela_repl_{}_{attempt}.cdl", std::process::id()));
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .is_ok()
        {
            return path;
        }
    }
    panic!("[ERROR] Cannot make a session file in {}", dir.display());
}

#[cold]
#[inline(never)]
pub fn repl() -> ExitCode {
    say!(
        "{BLUE}CANDELA {} REPL (read-eval-print-loop){RESET}\n",
        env!("CARGO_PKG_VERSION")
    );
    say!("{BLUE}Type exit to leave, or press Ctrl+D (Ctrl+Z then enter on Windows).{RESET}\n");

    // Asking GitHub for the latest release runs on its own thread and the
    // answer is picked up between prompts, so nothing here holds the prompt
    // back.
    let mut update = crate::update::start();

    let exe = std::env::current_exe()
        .unwrap_or_else(|e| panic!("[ERROR] Cannot find the candela binary: {e}"));
    // One file per prompt: two open at once each compile a session of their
    // own, and a shared path would hand one of them the other's program.
    let tmp = session_file();

    let mut lines: Vec<Line> = Vec::with_capacity(1);
    let mut prev_stdout = String::with_capacity(1);
    let mut contents = String::with_capacity(20);

    loop {
        update = update.and_then(crate::update::poll);

        let mut s = String::new();
        say!("> ");
        let _ = stdout().flush();
        let read = stdin()
            .read_line(&mut s)
            .unwrap_or_else(|e| panic!("[ERROR] Cannot read the line: {e}"));
        // End of input: a terminal sent the end-of-file keystroke, or a piped
        // script ran out of lines. There is no next line to prompt for, so
        // leave rather than loop on an empty read.
        if read == 0 {
            say!("\n");
            return ExitCode::SUCCESS;
        }
        if s.ends_with('\n') {
            s.pop();
        }
        if s.ends_with('\r') {
            s.pop();
        }
        if s.is_empty() {
            continue;
        }
        match exit_line(&s) {
            // The name on its own names no status and is not a call, so there
            // is nothing to compile: the session leaves as a program that ran
            // to its end does.
            Some(ExitLine::Bare) => return ExitCode::SUCCESS,
            // A call leaves with what it names, which the session works out:
            // `exit(code)` reads the `code` the lines before it bound. The
            // line runs once, in the process the session is compiled in, and
            // the prompt leaves with the status that process left with, so
            // leaving through the prompt and leaving from a file are the same
            // call with the same result.
            Some(ExitLine::Call) => {
                lines.push(Line {
                    text: as_typed(&s),
                    declaration: false,
                });
                program(&lines, &mut contents);
                let output = evaluate(&exe, &tmp, &contents);
                // A line the compiler refused is dropped like any other, and
                // the session goes on: nothing ran, so nothing left.
                if !output.status.success() && !compiles(&tmp, &contents) {
                    lines.pop();
                    let _ = write!(stderr(), "{}", String::from_utf8_lossy(&output.stderr));
                    continue;
                }
                let new_stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if new_stdout.len() > prev_stdout.len() {
                    say!("{}", &new_stdout[prev_stdout.len()..]);
                }
                let _ = write!(stderr(), "{}", String::from_utf8_lossy(&output.stderr));
                let _ = stdout().flush();
                // Straight through with the status the run left with, which a
                // `u8` would take the low byte of and a platform with a wider
                // status would then report short.
                std::process::exit(output.status.code().unwrap_or(1));
            }
            None => {}
        }
        let kind = classify_line(&s);

        let mut rejected = None;
        for form in forms(&s, kind) {
            lines.push(Line {
                text: form,
                declaration: kind == LineKind::Declaration,
            });
            program(&lines, &mut contents);
            let output = evaluate(&exe, &tmp, &contents);
            if output.status.success() {
                let new_stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if new_stdout.len() > prev_stdout.len() {
                    say!("{}", &new_stdout[prev_stdout.len()..]);
                }
                prev_stdout = new_stdout;
                rejected = None;
                break;
            }
            // A form the compiler refused never ran, so the line is free to be
            // tried in another one. One that compiled and then failed at run
            // time has already had whatever effect it had, and running the line
            // again would repeat it.
            let refused_by_the_compiler = !compiles(&tmp, &contents);
            // A line that does not compile is dropped, so the lines after it
            // still run.
            lines.pop();
            rejected = Some(output.stderr);
            if !refused_by_the_compiler {
                break;
            }
        }
        // The error reported is the last form's, which is the line as it was
        // typed: an error against a `print` the prompt wrapped around it would
        // name text nobody wrote.
        if let Some(stderr_bytes) = rejected {
            let _ = write!(stderr(), "{}", String::from_utf8_lossy(&stderr_bytes));
        }
    }
}
