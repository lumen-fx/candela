//! Quick fixes for the compiler diagnostics that have one obvious fix.
//!
//! The compiler's report says what to write in its help text, and the
//! editor hands back each diagnostic it shows with its code and message, so
//! a fix is read from them: the `import "...";` a missing module needs, the
//! `module::name` a call should spell, and the `int(...)` that makes a
//! truncation explicit. The tests compile real programs, so a change to the
//! wording these read breaks them here first.

/// One fix: a title and the edits it makes, as byte ranges of the buffer
/// and their replacements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<(usize, usize, String)>,
    /// The fix to apply when the editor asks for the preferred one.
    pub preferred: bool,
}

/// The fixes for a diagnostic of `code` with `message`, raised at
/// `start..end` of `text`.
#[must_use]
pub fn fixes(text: &str, code: &str, message: &str, start: usize, end: usize) -> Vec<Fix> {
    let end = end.min(text.len());
    let start = start.min(end);
    match code {
        "implicit_float_to_int" if start < end => vec![Fix {
            title: "Truncate explicitly with int(...)".to_owned(),
            edits: vec![(start, end, format!("int({})", &text[start..end]))],
            preferred: true,
        }],
        "unknown_function" | "unknown_namespace" | "unknown_type" => {
            let import = quoted_import(message);
            let call = spelled_call(message);
            let mut edits = Vec::new();
            if let Some(import) = &import
                && !text.contains(import.as_str())
            {
                let at = import_position(text);
                edits.push((at, at, format!("{import}\n")));
            }
            if let Some(call) = &call
                && start < end
                && &text[start..end] != call
            {
                edits.push((start, end, call.clone()));
            }
            if edits.is_empty() {
                return Vec::new();
            }
            let title = match (&import, &call) {
                (Some(i), Some(c)) => format!("Add `{i}` and call `{c}`"),
                (Some(i), None) => format!("Add `{i}`"),
                (None, Some(c)) => format!("Call `{c}`"),
                (None, None) => return Vec::new(),
            };
            vec![Fix {
                title,
                edits,
                preferred: true,
            }]
        }
        _ => Vec::new(),
    }
}

/// The `import "...";` statement a message tells the reader to write, when
/// it names one with no item list.
fn quoted_import(message: &str) -> Option<String> {
    let at = message.find("import \"")?;
    let rest = &message[at..];
    let end = rest.find("\";")? + 2;
    let statement = &rest[..end];
    (!statement.contains('{')).then(|| statement.to_owned())
}

/// The `module::name` a message says the call is spelled as now: after
/// "and call ", "Call " or "is now ".
fn spelled_call(message: &str) -> Option<String> {
    for lead in ["and call ", "Call ", "is now "] {
        if let Some(at) = message.find(lead) {
            let name: String = message[at + lead.len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            if name.contains("::") && !name.ends_with(':') {
                return Some(name);
            }
        }
    }
    None
}

/// Where a new import goes: after the last `import` line at the top of the
/// file, or else after the file's leading `//` comment lines.
fn import_position(text: &str) -> usize {
    let mut at = 0;
    let mut offset = 0;
    let mut seen_import = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") {
            seen_import = true;
            at = offset + line.len();
        } else if !seen_import && trimmed.starts_with("//") {
            at = offset + line.len();
        } else if !trimmed.is_empty() {
            break;
        }
        offset += line.len();
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use candela::compiler::imports::ImportResolver;
    use std::path::PathBuf;

    /// Applies `fix` to `text`, the last edit first.
    fn apply(text: &str, fix: &Fix) -> String {
        let mut out = text.to_owned();
        let mut edits = fix.edits.clone();
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (s, e, new) in edits {
            out.replace_range(s..e, &new);
        }
        out
    }

    fn lib() -> ImportResolver {
        let mut r = ImportResolver::new();
        r.set_lib_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../libs"));
        r
    }

    /// The text after the fix of the first diagnostic with `code`, and
    /// whether the result compiles without that diagnostic.
    fn fixed(text: &str, code: &str) -> String {
        let path = std::env::temp_dir().join("candela_lsp_actions.cdl");
        let path = path.to_str().unwrap();
        let out = analyze(text, path, &lib());
        let d = out
            .errors
            .iter()
            .chain(&out.warnings)
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("no {code} in {:?}", out.errors));
        let found = fixes(text, &d.code, &d.message, d.span.start, d.span.end);
        assert_eq!(found.len(), 1, "{found:?}");
        let after = apply(text, &found[0]);
        let again = analyze(&after, path, &lib());
        assert!(
            again
                .errors
                .iter()
                .chain(&again.warnings)
                .all(|d| d.code != code),
            "{after}\n{:?}",
            again.errors
        );
        after
    }

    #[test]
    fn a_std_function_gets_its_import_and_its_module() {
        let after = fixed(
            "fn main() {\n    print(sqrt(2.0));\n}\n",
            "unknown_function",
        );
        assert_eq!(
            after,
            "import \"std/math\";\nfn main() {\n    print(math::sqrt(2.0));\n}\n"
        );
    }

    #[test]
    fn a_module_gets_its_import_after_the_others() {
        let after = fixed(
            "//! A tool.\nimport \"std/math\";\n\nfn main() {\n    print(json::stringify([1]));\n}\n",
            "unknown_namespace",
        );
        assert_eq!(
            after,
            "//! A tool.\nimport \"std/math\";\nimport \"std/json\";\n\nfn main() {\n    print(json::stringify([1]));\n}\n"
        );
    }

    #[test]
    fn an_imported_item_is_called_through_its_module() {
        let after = fixed(
            "import \"std/json\";\nfn main() {\n    print(parse(\"1\"));\n}\n",
            "unknown_function",
        );
        assert!(after.contains("json::parse(\"1\")"), "{after}");
    }

    #[test]
    fn a_renamed_function_gets_its_new_spelling() {
        let after = fixed("fn main() {\n    assert_eq(1, 1);\n}\n", "unknown_function");
        assert!(after.starts_with("import \"std/assert\";\n"), "{after}");
        assert!(after.contains("assert::eq(1, 1)"), "{after}");
    }

    #[test]
    fn a_truncation_is_made_explicit() {
        let after = fixed(
            "fn main() {\n    let x: int = 2.5 * 2.0;\n    print(x);\n}\n",
            "implicit_float_to_int",
        );
        assert!(after.contains("let x: int = int(2.5 * 2.0);"), "{after}");
    }

    #[test]
    fn a_name_with_no_suggestion_has_no_fix() {
        assert!(
            fixes(
                "sqr(1)",
                "unknown_function",
                "Cannot find function sqr",
                0,
                3
            )
            .is_empty()
        );
        assert!(fixes("x", "type_mismatch", "anything", 0, 1).is_empty());
    }
}
