//! Where program output and error reports go.
//!
//! By default they go to the process's stdout and stderr. A host that embeds
//! the runtime can redirect both into a per-thread buffer instead, run some
//! candela code, and read the buffer back. Redirection is a runtime setting, so
//! a single build serves both the command-line toolchain and an embedding host.
//! wasm builds always redirect, since there are no process streams to write to.
//!
//! A stream that refuses a write costs the text, never the run: write through
//! [`out!`] and [`outln!`] rather than unwrapping.

use std::cell::Cell;
use std::cell::RefCell;
use std::io::Write;

thread_local! {
    pub static CAPTURED_OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
    static CAPTURING: Cell<bool> = const { Cell::new(false) };
}

/// Whether output on this thread is redirected into [`CAPTURED_OUTPUT`].
#[inline]
pub fn is_capturing() -> bool {
    cfg!(target_arch = "wasm32") || CAPTURING.with(Cell::get)
}

/// Starts or stops redirecting output on this thread, and returns the setting
/// that was in effect, so a caller can put it back.
pub fn set_capturing(on: bool) -> bool {
    CAPTURING.replace(on)
}

/// Appends to this thread's capture buffer, whether or not redirection is on.
pub fn print(s: &str) {
    CAPTURED_OUTPUT.with(|o| o.borrow_mut().push_str(s));
}

/// The sink for program output (`print` and friends).
pub fn stdout() -> OutputHandle {
    if is_capturing() {
        OutputHandle::Captured
    } else {
        OutputHandle::Stdout(std::io::stdout().lock())
    }
}

/// The sink for error reports.
pub fn stderr() -> OutputHandle {
    if is_capturing() {
        OutputHandle::Captured
    } else {
        OutputHandle::Stderr(std::io::stderr())
    }
}

/// Writes to an [`OutputHandle`], and drops the text when the write fails.
///
/// A pipe whose reader has already exited fails every write it is given, and
/// the same goes for a stream a supervisor closed. Raising there ends the run
/// over output nobody reads: it kills a piped command mid-program, and it kills
/// the thread an embedding host renders on. The line is worth less than the
/// run, so it goes nowhere and execution carries on.
macro_rules! out {
    ($handle:expr, $($arg:tt)*) => {{
        let _ = std::io::Write::write_fmt(&mut $handle, format_args!($($arg)*));
    }};
}

/// [`out!`] with a trailing newline, for a whole line of program output.
macro_rules! outln {
    ($handle:expr, $($arg:tt)*) => {{
        let _ = std::io::Write::write_fmt(
            &mut $handle,
            format_args!("{}\n", format_args!($($arg)*)),
        );
    }};
}

pub(crate) use out;
pub(crate) use outln;

/// A writer that resolves to a process stream or to the capture buffer,
/// whichever is in effect when it is created.
pub enum OutputHandle {
    Stdout(std::io::StdoutLock<'static>),
    Stderr(std::io::Stderr),
    Captured,
}

impl Write for OutputHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Stdout(out) => out.write(buf),
            Self::Stderr(out) => out.write(buf),
            Self::Captured => {
                if let Ok(s) = std::str::from_utf8(buf) {
                    print(s);
                }
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Stdout(out) => out.flush(),
            Self::Stderr(out) => out.flush(),
            Self::Captured => Ok(()),
        }
    }

    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        match self {
            Self::Stdout(out) => out.write_fmt(fmt),
            Self::Stderr(out) => out.write_fmt(fmt),
            Self::Captured => {
                print(&fmt.to_string());
                Ok(())
            }
        }
    }
}
