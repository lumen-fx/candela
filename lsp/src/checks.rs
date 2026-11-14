//! The `check` command a package names in its `candela.toml`.
//!
//! A package can know about mistakes the compiler cannot see: an engine that
//! reads attributes, settings files and asset paths when it loads a program.
//! It says how to ask it with a command, run with the file being checked:
//!
//! ```toml
//! [package]
//! check = ["engine", "check", "--json", "{file}"]
//! ```
//!
//! The command prints its findings as JSON on stdout: a list, or an object
//! whose `diagnostics` key holds one. Each finding is
//!
//! ```json
//! { "file": "/game/main.cdl", "line": 3, "column": 5, "end_line": 3, "end_column": 9,
//!   "severity": "error", "code": "unknown_action", "message": "...",
//!   "related": [{ "file": "...", "line": 1, "column": 1, "message": "..." }] }
//! ```
//!
//! `file` is absolute or relative to the directory of the file being
//! checked, which the command runs in. Lines and columns count from 1, and a
//! column counts characters. Only `file` and `message` are required: a
//! finding with no `line` covers the first line, one with no `column` its
//! whole line, and one with no end runs to the end of its line. `severity` is
//! `error` (the default), `warning`, `information` or `hint`.

use candela::PackageCheck;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tower_lsp::lsp_types::{DiagnosticSeverity, Position, Range};

/// How long a check may run before it is stopped and its findings dropped.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A place in a file a check pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub file: PathBuf,
    /// From 1.
    pub line: u32,
    /// From 1, in characters. `None` covers the whole line.
    pub column: Option<u32>,
    pub end: Option<(u32, u32)>,
}

/// One finding a check reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub place: Place,
    pub severity: DiagnosticSeverity,
    pub code: Option<String>,
    pub message: String,
    pub related: Vec<Related>,
}

/// Another place a finding is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    pub place: Place,
    pub message: String,
}

impl Found {
    /// The finding's range in the editor, against the file's text when it
    /// is known.
    #[must_use]
    pub fn range(&self, text: Option<&str>) -> Range {
        self.place.range(text)
    }
}

impl Related {
    #[must_use]
    pub fn range(&self, text: Option<&str>) -> Range {
        self.place.range(text)
    }
}

impl Place {
    #[must_use]
    pub fn range(&self, text: Option<&str>) -> Range {
        let line_text = |line: u32| -> Option<&str> {
            text.and_then(|t| t.split('\n').nth(line.saturating_sub(1) as usize))
                .map(|l| l.strip_suffix('\r').unwrap_or(l))
        };
        // A column counts characters; the editor counts UTF-16 units.
        let utf16 = |line: u32, column: u32| -> u32 {
            let skip = column.saturating_sub(1) as usize;
            match line_text(line) {
                Some(l) => l.chars().take(skip).map(char::len_utf16).sum::<usize>() as u32,
                None => skip as u32,
            }
        };
        let line_end =
            |line: u32| -> u32 { line_text(line).map_or(0, |l| l.encode_utf16().count() as u32) };
        let line = self.line.max(1);
        let start = Position::new(line - 1, self.column.map_or(0, |c| utf16(line, c)));
        let end = match (self.column, self.end) {
            (_, Some((end_line, end_column))) => {
                let end_line = end_line.max(line);
                Position::new(end_line - 1, utf16(end_line, end_column))
            }
            _ => Position::new(line - 1, line_end(line).max(start.character)),
        };
        Range::new(start, end)
    }
}

/// Runs a package's check from `cwd`, and reads what it found.
///
/// # Errors
///
/// When the command cannot start, runs past its time, or prints something
/// that is not the findings JSON; the reason, for the editor's log.
pub async fn run(check: &PackageCheck, cwd: Option<&Path>) -> Result<Vec<Found>, String> {
    let mut command = tokio::process::Command::new(&check.program);
    command
        .args(&check.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = tokio::time::timeout(TIMEOUT, command.output())
        .await
        .map_err(|_| {
            format!(
                "{} ran longer than {} seconds",
                check.program.display(),
                TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| format!("{} could not start: {e}", check.program.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{} failed ({}): {}",
            check.program.display(),
            output.status,
            stderr.lines().take(5).collect::<Vec<_>>().join(" / ")
        ));
    }
    parse(&stdout, cwd.unwrap_or_else(|| Path::new(".")))
}

/// Reads a check's output, resolving relative file names against `cwd`.
///
/// # Errors
///
/// When the output is not JSON in the shape the module docs describe.
pub fn parse(stdout: &str, cwd: &Path) -> Result<Vec<Found>, String> {
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value =
        serde_json::from_str(stdout).map_err(|e| format!("its output is not JSON: {e}"))?;
    let list = match &value {
        Value::Array(list) => list,
        Value::Object(map) => match map.get("diagnostics") {
            Some(Value::Array(list)) => list,
            _ => return Err(String::from("its output has no diagnostics list")),
        },
        _ => return Err(String::from("its output is not a list of diagnostics")),
    };
    Ok(list.iter().filter_map(|item| found(item, cwd)).collect())
}

fn place(item: &Value, cwd: &Path) -> Option<Place> {
    let file = item.get("file").and_then(Value::as_str)?;
    let number = |key: &str| {
        item.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let line = number("line").unwrap_or(1);
    let end = number("end_line")
        .zip(number("end_column"))
        .or_else(|| number("end_column").map(|c| (line, c)));
    Some(Place {
        file: cwd.join(file),
        line,
        column: number("column"),
        end,
    })
}

fn found(item: &Value, cwd: &Path) -> Option<Found> {
    let at = place(item, cwd)?;
    let message = item.get("message").and_then(Value::as_str)?.to_owned();
    let severity = match item.get("severity").and_then(Value::as_str) {
        Some("warning") => DiagnosticSeverity::WARNING,
        Some("information" | "info") => DiagnosticSeverity::INFORMATION,
        Some("hint") => DiagnosticSeverity::HINT,
        _ => DiagnosticSeverity::ERROR,
    };
    let code = match item.get("code") {
        Some(Value::String(code)) => Some(code.clone()),
        Some(Value::Number(code)) => Some(code.to_string()),
        _ => None,
    };
    let related = item
        .get("related")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|r| {
                    Some(Related {
                        place: place(r, cwd)?,
                        message: r
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Found {
        place: at,
        severity,
        code,
        message,
        related,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_and_an_object_both_read() {
        let one = r#"[{"file": "main.cdl", "line": 2, "column": 3, "end_column": 6, "message": "m", "code": "c"}]"#;
        let found = parse(one, Path::new("/game")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].place.file, PathBuf::from("/game/main.cdl"));
        assert_eq!(found[0].severity, DiagnosticSeverity::ERROR);
        assert_eq!(found[0].code.as_deref(), Some("c"));
        let range = found[0].range(Some("first\nlet abc = 1;\n"));
        assert_eq!(range, Range::new(Position::new(1, 2), Position::new(1, 5)));

        let wrapped = r#"{"diagnostics": [{"file": "/abs/drift.toml", "line": 4, "message": "m", "severity": "warning"}]}"#;
        let found = parse(wrapped, Path::new("/game")).unwrap();
        assert_eq!(found[0].place.file, PathBuf::from("/abs/drift.toml"));
        assert_eq!(found[0].severity, DiagnosticSeverity::WARNING);
    }

    #[test]
    fn a_finding_with_no_column_covers_its_line() {
        let found = parse(
            r#"[{"file": "a.toml", "line": 2, "message": "m"}]"#,
            Path::new("/g"),
        )
        .unwrap();
        let range = found[0].range(Some("[a]\ngravity = 900\n"));
        assert_eq!(range, Range::new(Position::new(1, 0), Position::new(1, 13)));
    }

    #[test]
    fn columns_count_characters() {
        let found = parse(
            r#"[{"file": "a.cdl", "line": 1, "column": 3, "end_line": 1, "end_column": 4, "message": "m"}]"#,
            Path::new("/g"),
        )
        .unwrap();
        // An e with an acute accent is one character and one UTF-16 unit, an
        // emoji one character and two.
        let range = found[0].range(Some("\u{1F600}\u{e9}x"));
        assert_eq!(range, Range::new(Position::new(0, 3), Position::new(0, 4)));
    }

    #[test]
    fn related_places_and_bad_output() {
        let found = parse(
            r#"[{"file": "a.cdl", "message": "m", "related": [{"file": "b.cdl", "line": 5, "message": "here"}]}]"#,
            Path::new("/g"),
        )
        .unwrap();
        assert_eq!(found[0].related[0].place.file, PathBuf::from("/g/b.cdl"));
        assert_eq!(found[0].related[0].message, "here");
        assert!(parse("not json", Path::new("/g")).is_err());
        assert!(parse("{\"x\": 1}", Path::new("/g")).is_err());
        assert!(parse("", Path::new("/g")).unwrap().is_empty());
    }
}
