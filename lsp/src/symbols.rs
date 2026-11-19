//! The declarations a file makes, read from its tokens alone, for workspace
//! symbols: a search over every file of the workspace cannot compile each
//! one, and a declaration's name and place are plain in the text.

use crate::syntax::Tokens;
use tower_lsp::lsp_types::SymbolKind;

/// A top-level declaration, or a method in an `impl` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub name: String,
    pub kind: SymbolKind,
    /// The type an `impl` block attaches a method to.
    pub container: Option<String>,
    /// The name's byte range.
    pub start: usize,
    pub end: usize,
}

/// The functions, methods, constants, structs and enums `text` declares.
#[must_use]
pub fn declarations(text: &str) -> Vec<Declared> {
    let t = Tokens::new(text);
    let mut out = Vec::new();
    let mut depth = 0usize;
    // The type of the `impl` block open at depth 1, if one is.
    let mut impl_type: Option<String> = None;
    let mut pending_impl: Option<String> = None;
    let n = t.toks.len();
    let mut i = 0;
    while i < n {
        let s = t.s(i);
        match s {
            "{" => {
                depth += 1;
                if depth == 1 {
                    impl_type = pending_impl.take();
                }
            }
            "}" => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    impl_type = None;
                }
            }
            "impl" if depth == 0 && t.is_ident(i + 1) => {
                pending_impl = Some(t.s(i + 1).to_owned());
            }
            "fn" | "struct" | "enum" | "const" if t.is_ident(i + 1) => {
                let in_impl = depth == 1 && impl_type.is_some();
                let top = depth == 0 && (i == 0 || t.s(i - 1) != "=");
                if top || in_impl && s == "fn" {
                    let kind = match s {
                        "fn" if in_impl => SymbolKind::METHOD,
                        "fn" => SymbolKind::FUNCTION,
                        "struct" => SymbolKind::STRUCT,
                        "enum" => SymbolKind::ENUM,
                        _ => SymbolKind::CONSTANT,
                    };
                    let tok = t.toks[i + 1];
                    out.push(Declared {
                        name: t.s(i + 1).to_owned(),
                        kind,
                        container: if in_impl { impl_type.clone() } else { None },
                        start: tok.start,
                        end: tok.end,
                    });
                }
                i += 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Whether `query`'s characters appear in `name` in order, ignoring case:
/// `mkth` matches `make_thing`.
#[must_use]
pub fn matches(name: &str, query: &str) -> bool {
    let mut chars = name.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|q| chars.any(|c| c == q))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_of_a_file() {
        let text = "import \"std/json\";\n\
            const LIMIT: int = 3;\n\
            struct Point { x: int = 0 }\n\
            enum Mode { On, Off }\n\
            impl Point {\n    fn norm(self) -> int { let f = fn(x) => x; return 1; }\n}\n\
            fn main() { let s = Point { .. }; }\n\
            host \"kit\" { int add(int, int); }\n";
        let found: Vec<(String, SymbolKind, Option<String>)> = declarations(text)
            .into_iter()
            .map(|d| (d.name, d.kind, d.container))
            .collect();
        assert_eq!(
            found,
            vec![
                ("LIMIT".to_owned(), SymbolKind::CONSTANT, None),
                ("Point".to_owned(), SymbolKind::STRUCT, None),
                ("Mode".to_owned(), SymbolKind::ENUM, None),
                (
                    "norm".to_owned(),
                    SymbolKind::METHOD,
                    Some("Point".to_owned())
                ),
                ("main".to_owned(), SymbolKind::FUNCTION, None),
            ]
        );
    }

    #[test]
    fn a_query_matches_in_order() {
        assert!(matches("make_thing", "mkth"));
        assert!(matches("MakeThing", "make"));
        assert!(!matches("make_thing", "thmk"));
        assert!(matches("anything", ""));
    }
}
