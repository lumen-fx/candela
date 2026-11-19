//! Find references, rename and document highlight.
//!
//! A reference is a name in some file that go-to-definition takes to the
//! same declaration: each identifier spelled like the one under the cursor
//! is resolved the way `Ide::definition` resolves it, against a summary of
//! a program that file is the root of, and kept when it lands on the same
//! file and span. Nothing here keeps an index; the backend decides which
//! files to read (see `Inner::reference_files`).

use crate::analysis::ProgramSummary;
use crate::ide::Ide;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A declaration, the way references are matched against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    /// The declaring file, canonical.
    pub file: PathBuf,
    /// Where `Ide::definition` places the declaration in that file.
    pub start: usize,
    /// The name as written.
    pub name: String,
    /// Where the name is written in the declaration, when the summary that
    /// found it read the file's current text.
    pub name_range: Option<(usize, usize)>,
    /// A parameter or a local, which only its own file can name.
    pub local: bool,
    /// Declared in a standard library module.
    pub std: bool,
}

/// The canonical path of a file a summary read.
fn canonical(path: &str) -> PathBuf {
    Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
}

/// The file and start `definition` gives for the name at `offset`.
fn resolve(
    ide: &Ide<'_>,
    summary: &ProgramSummary,
    path: &Path,
    files: &mut HashMap<u16, PathBuf>,
) -> Option<(PathBuf, usize)> {
    match ide.definition()? {
        Ok(target) => {
            let file = if target.src_file == 0 {
                path.to_path_buf()
            } else {
                files
                    .entry(target.src_file)
                    .or_insert_with(|| {
                        summary
                            .sources
                            .get(usize::from(target.src_file))
                            .map_or_else(PathBuf::new, |s| canonical(&s.path))
                    })
                    .clone()
            };
            Some((file, target.span.start as usize))
        }
        Err(at) => Some((path.to_path_buf(), at)),
    }
}

/// The first `name`, as a whole word, at or after `from` in `text`.
fn word_at_or_after(text: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut at = from.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    while let Some(i) = text[at..].find(name) {
        let s = at + i;
        let e = s + name.len();
        let before = text[..s].chars().next_back().is_some_and(is_word);
        let after = text[e..].chars().next().is_some_and(is_word);
        if !before && !after {
            return Some((s, e));
        }
        at = e;
    }
    None
}

/// The declaration the identifier at `offset` names, and the identifier's
/// byte range. `path` is the canonical path of the file `text` is.
#[must_use]
pub fn declaration(
    text: &str,
    summary: &ProgramSummary,
    path: &Path,
    offset: usize,
) -> Option<(Decl, (usize, usize))> {
    let ide = Ide::new(text, summary, offset);
    let toks = ide.tokens();
    let i = toks.at(offset)?;
    if !toks.is_ident(i) {
        return None;
    }
    let under = (toks.toks[i].start, toks.toks[i].end);
    let name = toks.s(i).to_owned();
    let mut files = HashMap::new();
    let (file, start) = resolve(&ide, summary, path, &mut files)?;
    let local = matches!(ide.definition(), Some(Err(_)));
    let (decl_text, std) = if local {
        (Some(text), false)
    } else {
        let source = summary
            .sources
            .iter()
            .enumerate()
            .find(|(n, s)| {
                if *n == 0 {
                    file == path
                } else {
                    canonical(&s.path) == file
                }
            })
            .map(|(_, s)| s);
        // The buffer's own declarations are placed against the text the
        // summary read, which is the buffer's only when it compiled as is.
        let current = source
            .map(|s| s.text.as_str())
            .filter(|t| file != path || *t == text);
        (current, source.is_some_and(|s| s.is_std))
    };
    let name_range = decl_text.and_then(|t| word_at_or_after(t, start, &name));
    Some((
        Decl {
            file,
            start,
            name,
            name_range,
            local,
            std,
        },
        under,
    ))
}

/// Every place in `text` (the file at canonical `path`, analysed as the
/// root of `summary`) that names `decl`, in order, the declaration itself
/// included when it is in this file.
#[must_use]
pub fn occurrences(
    text: &str,
    summary: &ProgramSummary,
    path: &Path,
    decl: &Decl,
) -> Vec<(usize, usize)> {
    let mut ide = Ide::new(text, summary, 0);
    let toks = ide.tokens();
    let candidates: Vec<(usize, usize)> = (0..toks.toks.len())
        .filter(|&i| toks.is_ident(i) && toks.s(i) == decl.name)
        .map(|i| (toks.toks[i].start, toks.toks[i].end))
        .collect();
    let mut files = HashMap::new();
    let mut found = Vec::new();
    for (s, e) in candidates {
        ide.set_offset(s);
        if resolve(&ide, summary, path, &mut files) == Some((decl.file.clone(), decl.start)) {
            found.push((s, e));
        }
    }
    if decl.file == path
        && let Some(range) = decl.name_range
        && !found.contains(&range)
    {
        found.push(range);
        found.sort_unstable();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use candela::compiler::imports::ImportResolver;

    const SRC: &str = r#"
struct Point { x: int = 0, y: int = 0 }

enum Shape { Dot, Line(int) }

impl Point {
    fn norm(self) -> int { return self.x + self.y; }
}

fn twice(p: Point) -> int {
    let total = p.norm() + p.x;
    return total * 2;
}

fn main() {
    let p = Point { x: 1, .. };
    let s = Shape::Dot;
    let total = twice(p);
    print(total + p.x);
    match s {
        Shape::Dot => print(0),
        Shape::Line(n) => print(n),
    }
}
"#;

    fn summary() -> ProgramSummary {
        let path = std::env::temp_dir().join("candela_lsp_refs.cdl");
        let out = analyze(SRC, path.to_str().unwrap(), &ImportResolver::new());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        out.summary.expect("summary")
    }

    fn path() -> PathBuf {
        std::env::temp_dir().join("candela_lsp_refs.cdl")
    }

    /// The text of every reference to what the `nth` `word` names, with the
    /// line each is on.
    fn refs(word: &str, nth: usize) -> Vec<(usize, String)> {
        let s = summary();
        let offset = SRC.match_indices(word).nth(nth).unwrap().0;
        let (decl, _) = declaration(SRC, &s, &path(), offset).expect("a declaration");
        occurrences(SRC, &s, &path(), &decl)
            .into_iter()
            .map(|(a, b)| (SRC[..a].lines().count(), SRC[a..b].to_owned()))
            .collect()
    }

    #[test]
    fn a_function_from_its_call_and_its_declaration() {
        let from_call = refs("twice", 1);
        assert_eq!(from_call.len(), 2);
        assert_eq!(refs("twice", 0), from_call);
    }

    #[test]
    fn fields_methods_and_variants() {
        // `x`: the declaration, the literal's field, and three reads.
        assert_eq!(refs("x:", 0).len(), 5, "{:?}", refs("x:", 0));
        assert_eq!(refs("norm", 0).len(), 2);
        // A variant: its declaration, the value and the pattern.
        assert_eq!(refs("Dot", 0).len(), 3, "{:?}", refs("Dot", 0));
    }

    #[test]
    fn a_local_is_told_from_one_of_the_same_name() {
        let in_twice = refs("total", 0);
        assert_eq!(in_twice.len(), 2);
        let in_main = refs("total", 2);
        assert_eq!(in_main.len(), 2);
        assert_ne!(in_twice, in_main);
    }

    #[test]
    fn a_builtin_has_no_declaration() {
        let s = summary();
        let offset = SRC.find("print").unwrap();
        assert!(declaration(SRC, &s, &path(), offset).is_none());
    }
}
