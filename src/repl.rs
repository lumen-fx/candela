use crate::errors::BLUE;
use crate::errors::RESET;
use crate::parser::LineKind;
use crate::parser::classify_line;
use std::io::{Write, stderr, stdin, stdout};
use std::path::Path;
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

/// Compiles and runs `contents` as a program, and hands back what it wrote.
///
/// The session runs in a process of its own, so a line the compiler rejects
/// reports its error there and leaves the prompt to read the next line.
fn evaluate(exe: &Path, tmp: &Path, contents: &str) -> Output {
    std::fs::write(tmp, contents).expect("{RED}[ERROR]{RESET} Cannot write to temporary file");
    let output = std::process::Command::new(exe)
        .arg(tmp)
        .output()
        .expect("{RED}[ERROR]{RESET} Failed to execute Candela");
    // The next line writes the file again, so a removal that does not happen
    // costs nothing beyond the file staying behind.
    let _ = std::fs::remove_file(tmp);
    output
}

#[cold]
#[inline(never)]
pub fn repl() {
    say!(
        "{BLUE}CANDELA {} REPL (read-eval-print-loop){RESET}\n",
        env!("CARGO_PKG_VERSION")
    );

    // Asking GitHub for the latest release runs on its own thread and the
    // answer is picked up between prompts, so nothing here holds the prompt
    // back.
    let mut update = crate::update::start();

    let exe = std::env::current_exe().expect("{RED}[ERROR]{RESET} Cannot find candela binary path");
    // One file per process: two prompts open at once each compile a session of
    // their own, and a shared path would hand one of them the other's program.
    let tmp = std::env::temp_dir().join(format!("candela_repl_{}.cdl", std::process::id()));

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
            .expect("{RED}[ERROR]{RESET} Incorrect input string");
        // End of input: a terminal sent the end-of-file keystroke, or a piped
        // script ran out of lines. There is no next line to prompt for, so
        // leave rather than loop on an empty read.
        if read == 0 {
            say!("\n");
            return;
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
        // Read before the semicolon goes on, so the line the grammar sees is
        // the line that was typed.
        let kind = classify_line(&s);
        if !s.ends_with(';') && !s.ends_with('}') {
            s.push(';');
        }
        if s.contains("exit()") && !s.contains('"') {
            say!(
                "{BLUE}[CANDELA TIP]{RESET} To exit, press Ctrl+D (Ctrl+Z then enter on Windows) or Ctrl+C\n"
            );
        }

        lines.push(Line {
            text: s,
            declaration: kind == LineKind::Declaration,
        });
        program(&lines, &mut contents);
        let output = evaluate(&exe, &tmp, &contents);

        let new_stdout = String::from_utf8_lossy(&output.stdout).to_string();

        if output.status.success() {
            if new_stdout.len() > prev_stdout.len() {
                say!("{}", &new_stdout[prev_stdout.len()..]);
            }
            prev_stdout = new_stdout;
        } else {
            let _ = write!(stderr(), "{}", String::from_utf8_lossy(&output.stderr));
            lines.pop();
        }
    }
}
