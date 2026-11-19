//! Inlay hints: the type a `let` takes from its value, and the name of the
//! parameter each argument of a call lands on.
//!
//! A `let`'s type is followed through the text the way completion follows
//! it (`syntax::Infer`), and a hint shows only where that type is known and
//! the value does not already spell it: a literal, or a value built by
//! naming its own struct or enum. An argument's parameter comes from the
//! function the call resolves to (`Ide::called`); a hint is left out where
//! the argument already reads as the name.

use crate::analysis::ProgramSummary;
use crate::ide::{Called, Ide};
use crate::line_index::LineIndex;
use crate::syntax::{Infer, Kind, Ty, scope_at};
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel};

/// The hints for the names and calls that start in `start..end`.
#[must_use]
pub fn hints(text: &str, summary: &ProgramSummary, start: usize, end: usize) -> Vec<InlayHint> {
    let ide = Ide::new(text, summary, 0);
    let t = ide.tokens();
    let index = LineIndex::new(text);
    let mut out = Vec::new();
    let n = t.toks.len();
    for i in 0..n {
        let at = t.toks[i].start;
        if at < start {
            continue;
        }
        if at >= end {
            break;
        }
        match t.s(i) {
            "let" => let_hints(&ide, summary, i, &index, &mut out),
            "(" => argument_hints(&ide, i, &index, &mut out),
            _ => {}
        }
    }
    out
}

/// The type of each name the `let` at token `i` binds without writing one.
fn let_hints(
    ide: &Ide<'_>,
    summary: &ProgramSummary,
    i: usize,
    index: &LineIndex<'_>,
    out: &mut Vec<InlayHint>,
) {
    let t = ide.tokens();
    let n = t.toks.len();
    let Some(eq) = (i..n)
        .take_while(|&k| t.s(k) != ";")
        .find(|&k| t.s(k) == "=")
    else {
        return;
    };
    // `let x: T = ...` says it already.
    if (i..eq).any(|k| t.s(k) == ":") {
        return;
    }
    if spells_its_type(ide, summary, eq + 1) {
        return;
    }
    let names: Vec<usize> = (i + 1..eq).filter(|&k| t.is_ident(k)).collect();
    if names.is_empty() {
        return;
    }
    let semi = (eq..n).find(|&k| t.s(k) == ";").unwrap_or(n);
    let scope = scope_at(t, semi.min(n));
    let infer = Infer::new(t, summary, &scope);
    for k in names {
        let Some(b) = scope.bindings.iter().find(|b| b.at == t.toks[k].start) else {
            continue;
        };
        let ty = infer.binding(b);
        if !known(&ty, summary) {
            continue;
        }
        out.push(InlayHint {
            position: index.position(t.toks[k].end),
            label: InlayHintLabel::String(format!(": {}", ty.render())),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: None,
            data: None,
        });
    }
}

/// Whether the value starting at token `v` already names its type: a lone
/// literal, or a struct literal, a variant or a type's own function
/// (`Point { .. }`, `Mode::On`, `Point::origin()`).
fn spells_its_type(ide: &Ide<'_>, summary: &ProgramSummary, v: usize) -> bool {
    let t = ide.tokens();
    let follows = matches!(t.s(v + 1), ";" | "");
    if matches!(t.kind(v), Some(Kind::Number | Kind::Str)) && follows {
        return true;
    }
    if matches!(t.s(v), "true" | "false") && follows {
        return true;
    }
    if t.is_ident(v) && matches!(t.s(v + 1), "{" | "::" | "<") {
        let name = t.s(v);
        return summary.struct_named(name).is_some()
            || summary.enum_named(name).is_some()
            || summary.lookup(name).is_some_and(|k| {
                matches!(
                    k,
                    crate::analysis::ScopeKind::Struct(_) | crate::analysis::ScopeKind::Enum(_)
                )
            });
    }
    false
}

/// Whether a type is worth showing: every name in it a builtin type other
/// than `any`, or a struct or an enum the program declares. A generic
/// function's type parameter (`T`) is not one; the text does not say what
/// a call fills it with.
fn known(ty: &Ty, summary: &ProgramSummary) -> bool {
    match ty {
        Ty::Named(n) => {
            (n != "any" && crate::ide::BUILTIN_TYPES.contains(&n.as_str()))
                || summary.struct_named(n).is_some()
                || summary.enum_named(n).is_some()
        }
        Ty::List(t) => known(t, summary),
        Ty::Map(k, v) => known(k, summary) && known(v, summary),
        Ty::Tuple(ts) => ts.iter().all(|t| known(t, summary)),
        Ty::Unknown => false,
    }
}

/// The parameter name in front of each argument of the call whose `(` is
/// token `paren`.
fn argument_hints(ide: &Ide<'_>, paren: usize, index: &LineIndex<'_>, out: &mut Vec<InlayHint>) {
    let Some(Called::Function(f)) = ide.called(paren) else {
        return;
    };
    let t = ide.tokens();
    let callee = t.s(paren - 1);
    let close = t.closing(paren);
    let params: Vec<&str> = f.call_params().map(|p| p.name.as_str()).collect();
    for ((s, e), name) in t.split_commas(paren + 1, close).into_iter().zip(params) {
        if s >= e || name.starts_with('_') {
            continue;
        }
        // The argument already reads as the name: `area(width)`,
        // `area(box.width)`, or the function is named for it: `set_width(w)`.
        let last = t.s(e - 1);
        let reads_as_name = last == name && (e - s == 1 || t.s(e - 2) == ".");
        if reads_as_name || callee == name || callee.ends_with(&format!("_{name}")) {
            continue;
        }
        out.push(InlayHint {
            position: index.position(t.toks[s].start),
            label: InlayHintLabel::String(format!("{name}:")),
            kind: Some(InlayHintKind::PARAMETER),
            text_edits: None,
            tooltip: None,
            padding_left: None,
            padding_right: Some(true),
            data: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use candela::compiler::imports::ImportResolver;

    const SRC: &str = r#"struct Point { x: int = 0, y: int = 0 }

fn make(x: int, height: int) -> Point { return Point { x: x, y: height }; }

fn set_x(p: Point, x: int) -> Point { return Point { x: x, y: p.y }; }

fn first<T>(xs: T[]) -> T { return xs[0]; }

fn main() {
    let a = make(1, 2);
    let b = Point { .. };
    let n = 3;
    let height = 4;
    let total = a.x + n;
    let names = ["a", "b"];
    let c = make(height, a.x);
    let d = set_x(c, 5);
    let f = first<int>([1]);
    print(d.x + b.y + total + names.len() + f);
}
"#;

    fn all() -> Vec<(String, String)> {
        let path = std::env::temp_dir().join("candela_lsp_hints.cdl");
        let out = analyze(SRC, path.to_str().unwrap(), &ImportResolver::new());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let summary = out.summary.expect("summary");
        let index = LineIndex::new(SRC);
        hints(SRC, &summary, 0, SRC.len())
            .into_iter()
            .map(|h| {
                let at = index.offset(h.position);
                let word: String = SRC[at..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let before: String = SRC[..at]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let InlayHintLabel::String(label) = h.label else {
                    panic!("a plain label");
                };
                // A type hint follows its name; a parameter hint precedes
                // its argument.
                let anchor = if label.starts_with(':') { before } else { word };
                (anchor, label)
            })
            .collect()
    }

    #[test]
    fn types_of_lets_that_do_not_spell_them() {
        let found = all();
        let types: Vec<&(String, String)> =
            found.iter().filter(|(_, l)| l.starts_with(':')).collect();
        assert_eq!(
            types,
            vec![
                &("a".to_owned(), ": Point".to_owned()),
                &("total".to_owned(), ": int".to_owned()),
                &("names".to_owned(), ": string[]".to_owned()),
                &("c".to_owned(), ": Point".to_owned()),
                &("d".to_owned(), ": Point".to_owned()),
                &("f".to_owned(), ": int".to_owned()),
            ],
            "{found:?}"
        );
    }

    #[test]
    fn parameter_names_where_the_argument_does_not_say_them() {
        let found = all();
        let params: Vec<&(String, String)> =
            found.iter().filter(|(_, l)| l.ends_with(':')).collect();
        // `make(1, 2)` names both, and so does `make(height, a.x)`, whose
        // arguments are the other way round; `set_x(c, 5)` names only `p`,
        // the function being named for `x`. `print(...)` is a builtin.
        assert_eq!(
            params,
            vec![
                &("1".to_owned(), "x:".to_owned()),
                &("2".to_owned(), "height:".to_owned()),
                &("height".to_owned(), "x:".to_owned()),
                &("a".to_owned(), "height:".to_owned()),
                &("c".to_owned(), "p:".to_owned()),
            ],
            "{found:?}"
        );
    }
}
