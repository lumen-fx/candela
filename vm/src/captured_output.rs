//! Where program output and error reports go.
//!
//! By default they go to the process's stdout and stderr. A host that embeds
//! the runtime can redirect both into a per-thread buffer instead, run some
//! candela code, and read the buffer back. Redirection is a runtime setting, so
//! a single build serves both the command-line toolchain and an embedding host.
//! wasm builds always redirect, since there are no process streams to write to.

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
