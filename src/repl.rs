use crate::errors::BLUE;
use crate::errors::RESET;
use std::io::{Write, stderr, stdin, stdout};

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
    let tmp = std::env::temp_dir().join("candela_repl_tmp.cdl");

    let mut all_lines: Vec<String> = Vec::with_capacity(1);
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
        if !s.ends_with(';') && !s.ends_with('}') {
            s.push(';');
        }
        if s.contains("exit()") && !s.contains('"') {
            say!(
                "{BLUE}[CANDELA TIP]{RESET} To exit, press Ctrl+D (Ctrl+Z then enter on Windows) or Ctrl+C\n"
            );
        }

        all_lines.push(s);

        contents.clear();
        for x in all_lines.iter().filter(|x| x.starts_with("import")) {
            contents.push_str(x);
            contents.push('\n');
        }
        contents.push_str("fn main() {\n");
        for x in all_lines.iter().filter(|x| !x.starts_with("import")) {
            contents.push_str(x);
            contents.push('\n');
        }
        contents.push('\n');
        contents.push('}');

        std::fs::write(&tmp, &contents)
            .expect("{RED}[ERROR]{RESET} Cannot write to temporary file");

        let output = std::process::Command::new(&exe)
            .arg(tmp.to_str().unwrap())
            .output()
            .expect("{RED}[ERROR]{RESET} Failed to execute Candela");

        std::fs::remove_file(&tmp).unwrap();

        let new_stdout = String::from_utf8_lossy(&output.stdout).to_string();

        if output.status.success() {
            if new_stdout.len() > prev_stdout.len() {
                say!("{}", &new_stdout[prev_stdout.len()..]);
            }
            prev_stdout = new_stdout;
        } else {
            let _ = write!(stderr(), "{}", String::from_utf8_lossy(&output.stderr));
            all_lines.pop();
        }
    }
}
