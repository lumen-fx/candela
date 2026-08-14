//! Deadline-bounded process runs for the integration tests.
//!
//! `Command::output()` blocks until every holder of the child's pipe write
//! ends closes them. Tests in one binary run on threads, their children
//! inherit each other's pipe handles across spawns, and one slow sibling can
//! keep another test's `output()` blocked long after its own child exited.
//! Draining the pipes on threads and polling the child against a deadline
//! turns that interleaving into a named failure instead of a hung CI job.

use std::io::Read;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const RUN_DEADLINE: Duration = Duration::from_mins(2);

/// Spawns the command and collects its output like `Command::output`, but
/// kills the child and panics past the deadline.
pub fn output_with_deadline(command: &mut Command, label: &str) -> Output {
    try_output_with_deadline(command, label)
        .unwrap_or_else(|e| panic!("{label}: spawn failed: {e}"))
}

/// Like `output_with_deadline`, but a spawn failure comes back as the error
/// instead of a panic, for callers that retry on "text file busy".
pub fn try_output_with_deadline(command: &mut Command, label: &str) -> std::io::Result<Output> {
    let mut child: Child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

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
        match child.try_wait().expect("poll the child") {
            Some(status) => break status,
            None if started.elapsed() > RUN_DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{label}: still running after {RUN_DEADLINE:?}; killed");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    Ok(Output {
        status,
        stdout: stdout.join().expect("read stdout"),
        stderr: stderr.join().expect("read stderr"),
    })
}
