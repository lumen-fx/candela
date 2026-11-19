//! Semantic tokens: what each name in a buffer is, so an editor colors a
//! type, a function, a variant or an attribute the same way whatever its
//! grammar guessed from the spelling.
//!
//! One pass over the buffer's tokens tracks the bindings each block opens
//! (parameters, `let`s, `for` patterns, generic parameters) the way
//! `syntax::scope_at` does, and the rest of a name's meaning comes from what
//! is written around it and from the program's declarations
//! (`ProgramSummary`). Keywords, literals and comments are left to the
//! grammar.

use crate::analysis::{ProgramSummary, ScopeKind};
use crate::builtins;
use crate::line_index::LineIndex;
use crate::syntax::Tokens;
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType};

/// The token types, in the order of their indexes in the legend.
pub const TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::TYPE,
    SemanticTokenType::STRUCT,
    SemanticTokenType::ENUM,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::METHOD,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::DECORATOR,
    SemanticTokenType::MACRO,
    SemanticTokenType::TYPE_PARAMETER,
];

/// The modifiers, by bit.
pub const MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION,
    SemanticTokenModifier::DEFAULT_LIBRARY,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Namespace,
    Type,
    Struct,
    Enum,
    Variant,
    Function,
    Method,
    Property,
    Parameter,
    Variable,
    Decorator,
    Macro,
    TypeParameter,
}

const DECLARATION: u32 = 1;
const DEFAULT_LIBRARY: u32 = 2;

/// The words the grammar colors as keywords; never a name.
const KEYWORDS: &[&str] = &[
    "let",
    "if",
    "else",
    "match",
    "while",
    "for",
    "in",
    "loop",
    "return",
    "break",
    "continue",
    "try",
    "catch",
    "throw",
    "true",
    "false",
    "null",
    "fn",
    "as",
    "is",
    "struct",
    "enum",
    "impl",
    "import",
    "host",
    "dylib",
    "attribute",
    "on",
    "const",
    "export",
];

/// What a block's bindings are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Binding {
    Parameter,
    Variable,
    TypeParameter,
}

#[derive(Default)]
struct Frame {
    names: Vec<(String, Binding)>,
}

/// What a bracket opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    /// A function's parameter list.
    Params,
    /// A struct's or an enum's declaration body.
    Decl,
    Brace,
    Other,
}

/// The semantic tokens of `text`, encoded relative to each other as LSP
/// sends them.
#[must_use]
pub fn tokens(text: &str, summary: &ProgramSummary) -> Vec<SemanticToken> {
    let t = Tokens::new(text);
    let index = LineIndex::new(text);
    let mut out = Vec::new();
    let (mut line, mut col) = (0u32, 0u32);
    let mut push = |start: usize, end: usize, class: Class, modifiers: u32| {
        let at = index.position(start);
        let len = u32::try_from(text[start..end].encode_utf16().count()).unwrap_or(0);
        let delta_line = at.line - line;
        let delta_start = if delta_line == 0 {
            at.character - col
        } else {
            at.character
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: len,
            token_type: class as u32,
            token_modifiers_bitset: modifiers,
        });
        line = at.line;
        col = at.character;
    };

    let mut frames: Vec<Frame> = vec![Frame::default()];
    // Bindings waiting for the brace that opens their block.
    let mut pending: Vec<(String, Binding)> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    // The bracket a `fn`, `struct` or `enum` declaration will open next.
    let mut next_open: Option<Open> = None;
    let n = t.toks.len();
    for i in 0..n {
        let s = t.s(i);
        match s {
            "(" | "[" | "{" => {
                let kind = match (s, next_open) {
                    ("(", Some(Open::Params)) => {
                        next_open = None;
                        Open::Params
                    }
                    ("{", Some(Open::Decl)) => {
                        next_open = None;
                        Open::Decl
                    }
                    ("{", _) => {
                        next_open = None;
                        Open::Brace
                    }
                    _ => Open::Other,
                };
                if s == "{" {
                    frames.push(Frame {
                        names: std::mem::take(&mut pending),
                    });
                }
                open.push(kind);
                continue;
            }
            ")" | "]" | "}" => {
                if s == "}" && frames.len() > 1 {
                    frames.pop();
                }
                open.pop();
                continue;
            }
            ";" => {
                pending.clear();
                continue;
            }
            "fn" => {
                next_open = Some(Open::Params);
                pending.clear();
                continue;
            }
            "struct" | "enum" => {
                next_open = Some(Open::Decl);
                continue;
            }
            _ => {}
        }
        if !t.is_ident(i) || KEYWORDS.contains(&s) {
            continue;
        }
        let tok = t.toks[i];
        let prev = if i > 0 { t.s(i - 1) } else { "" };
        let next = t.s(i + 1);
        let inner = open.last().copied();

        // Declarations.
        if matches!(prev, "fn" | "struct" | "enum") {
            let class = match prev {
                "struct" => Class::Struct,
                "enum" => Class::Enum,
                _ if frames.len() > 1 => Class::Method,
                _ => Class::Function,
            };
            push(tok.start, tok.end, class, DECLARATION);
            // `<T, U>` declares type parameters for what follows.
            if next == "<" {
                let mut j = i + 2;
                while j < n && t.s(j) != ">" {
                    if t.is_ident(j) && matches!(t.s(j - 1), "<" | ",") {
                        pending.push((t.s(j).to_owned(), Binding::TypeParameter));
                    }
                    j += 1;
                }
            }
            continue;
        }
        if prev == "impl" && next == "<" {
            let mut j = i + 2;
            while j < n && t.s(j) != ">" {
                if t.is_ident(j) && summary.lookup(t.s(j)).is_none() {
                    pending.push((t.s(j).to_owned(), Binding::TypeParameter));
                }
                j += 1;
            }
        }
        let owner = if matches!(prev, "let" | "for" | "const") {
            Some(prev)
        } else if matches!(prev, "(" | ",") {
            pattern_owner(&t, i)
        } else {
            None
        };
        if let Some(owner) = owner {
            let binding = (s.to_owned(), Binding::Variable);
            if owner == "for" {
                // A `for` binding belongs to the loop's block.
                pending.push(binding);
            } else {
                frames
                    .last_mut()
                    .expect("the file frame stays")
                    .names
                    .push(binding);
            }
            push(tok.start, tok.end, Class::Variable, DECLARATION);
            continue;
        }
        // A parameter, unless the list is a function type's: `fn(int) -> int`.
        if inner == Some(Open::Params)
            && matches!(prev, "(" | ",")
            && !crate::ide::BUILTIN_TYPES.contains(&s)
            && !matches!(
                summary.lookup(s),
                Some(ScopeKind::Struct(_) | ScopeKind::Enum(_))
            )
        {
            pending.push((s.to_owned(), Binding::Parameter));
            push(tok.start, tok.end, Class::Parameter, DECLARATION);
            continue;
        }
        if inner == Some(Open::Decl) && matches!(prev, "{" | ",") {
            let class = if next == ":" {
                Class::Property
            } else {
                Class::Variant
            };
            push(tok.start, tok.end, class, DECLARATION);
            continue;
        }

        // Attributes and macros.
        if prev == "@" || (prev == "::" && path_after_at(&t, i)) {
            let class = if next == "::" {
                Class::Namespace
            } else {
                Class::Decorator
            };
            push(tok.start, tok.end, class, 0);
            continue;
        }
        if next == "!" && t.s(i + 2) == "(" {
            push(tok.start, tok.end, Class::Macro, 0);
            continue;
        }

        // After a dot: a method when called, a field otherwise.
        if prev == "." {
            let class = if next == "(" || next == "<" {
                Class::Method
            } else {
                Class::Property
            };
            push(tok.start, tok.end, class, 0);
            continue;
        }
        // A field name in a struct literal.
        if next == ":" && inner == Some(Open::Brace) && t.s(i + 2) != ":" && prev != "let" {
            let literal = matches!(prev, "{" | ",");
            if literal {
                push(tok.start, tok.end, Class::Property, 0);
                continue;
            }
        }

        // `Owner::name`.
        if prev == "::" && i >= 2 {
            let owner = t.s(i - 2);
            let class = if let Some(e) = summary.enum_named(owner)
                && e.variants.iter().any(|v| v.name == s)
            {
                Some(Class::Variant)
            } else if summary.method(owner, s).is_some()
                || summary.struct_named(owner).is_some()
                || summary.enum_named(owner).is_some()
            {
                Some(Class::Function)
            } else {
                let label = format!("{owner}::{s}");
                item_class(summary, &label).or_else(|| {
                    // An enum reached through a module: `geo::Shape::Dot`.
                    let qualified = if i >= 4 && t.s(i - 3) == "::" {
                        format!("{}::{owner}", t.s(i - 4))
                    } else {
                        owner.to_owned()
                    };
                    summary
                        .enums
                        .iter()
                        .any(|e| {
                            (e.name == qualified || e.name.ends_with(&format!("::{owner}")))
                                && e.variants.iter().any(|v| v.name == s)
                        })
                        .then_some(Class::Variant)
                })
            };
            if let Some(class) = class {
                let class = if next == "::" && class == Class::Function {
                    Class::Namespace
                } else {
                    class
                };
                push(tok.start, tok.end, class, 0);
            } else if next == "(" {
                push(tok.start, tok.end, Class::Function, 0);
            }
            continue;
        }
        // `name::` with nothing known by that name: a module or a namespace.
        if next == "::" {
            let class = match summary.lookup(s) {
                Some(ScopeKind::Enum(_)) => Class::Enum,
                Some(ScopeKind::Struct(_)) => Class::Struct,
                _ if summary.enum_named(s).is_some() => Class::Enum,
                _ if summary.struct_named(s).is_some() => Class::Struct,
                _ => Class::Namespace,
            };
            push(tok.start, tok.end, class, 0);
            continue;
        }

        // A binding of an enclosing block.
        if let Some(b) = bound(&frames, &pending, s) {
            let class = match b {
                Binding::Parameter => Class::Parameter,
                Binding::Variable => Class::Variable,
                Binding::TypeParameter => Class::TypeParameter,
            };
            push(tok.start, tok.end, class, 0);
            continue;
        }
        if let Some(class) = item_class(summary, s) {
            push(tok.start, tok.end, class, 0);
            continue;
        }
        if crate::ide::BUILTIN_TYPES.contains(&s) {
            push(tok.start, tok.end, Class::Type, DEFAULT_LIBRARY);
            continue;
        }
        if builtins::BUILTIN_FUNCTIONS.iter().any(|(b, _)| *b == s) && next == "(" {
            push(tok.start, tok.end, Class::Function, DEFAULT_LIBRARY);
        }
    }
    out
}

/// What the innermost binding named `name` is, if a block binds it.
fn bound(frames: &[Frame], pending: &[(String, Binding)], name: &str) -> Option<Binding> {
    pending
        .iter()
        .rev()
        .chain(frames.iter().rev().flat_map(|f| f.names.iter().rev()))
        .find(|(n, _)| n == name)
        .map(|(_, b)| *b)
}

/// What the program declares under `label`, as a class.
fn item_class(summary: &ProgramSummary, label: &str) -> Option<Class> {
    Some(match summary.lookup(label)? {
        ScopeKind::Function(id) => {
            if summary.functions.get(id).is_some_and(|f| f.is_const) {
                Class::Variable
            } else {
                Class::Function
            }
        }
        ScopeKind::Struct(_) => Class::Struct,
        ScopeKind::Enum(_) => Class::Enum,
        ScopeKind::Attribute(_) => Class::Decorator,
    })
}

/// The `let` or `for` whose tuple pattern the name at `i` is in:
/// `let (a, b) = t;`, `for (a, b) in q`.
fn pattern_owner(t: &Tokens<'_>, i: usize) -> Option<&'static str> {
    let mut j = i;
    while j > 0 {
        let p = t.s(j - 1);
        if p == "(" {
            return match t.s(j.checked_sub(2)?) {
                "let" => Some("let"),
                "for" => Some("for"),
                _ => None,
            };
        }
        if p == "," || t.is_ident(j - 1) {
            j -= 1;
        } else {
            return None;
        }
    }
    None
}

/// Whether the name at `i` ends an `@a::b` path.
fn path_after_at(t: &Tokens<'_>, i: usize) -> bool {
    let mut j = i;
    while j >= 2 && t.s(j - 1) == "::" && t.is_ident(j - 2) {
        j -= 2;
    }
    j > 0 && t.s(j - 1) == "@"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::analyze;
    use candela::compiler::imports::ImportResolver;

    const SRC: &str = r#"host "kit" {
    attribute tag on fn;
}

struct Cell<T> { value: T }

enum Mode { On, Off }

impl Cell<T> {
    fn get(self) -> T { return self.value; }
}

@tag
fn run(n: int) -> int {
    let c = Cell<int> { value: n };
    let m = Mode::On;
    for (i, x) in [(1, 2)] {
        print(i + x);
    }
    return c.get() + run(n - 1);
}
"#;

    /// Each named token as `(text, type name, modifier bits)`.
    fn classes() -> Vec<(String, String, u32)> {
        let path = std::env::temp_dir().join("candela_lsp_semantic.cdl");
        let out = analyze(SRC, path.to_str().unwrap(), &ImportResolver::new());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        let summary = out.summary.expect("summary");
        let index = LineIndex::new(SRC);
        let (mut line, mut col) = (0u32, 0u32);
        tokens(SRC, &summary)
            .into_iter()
            .map(|tok| {
                line += tok.delta_line;
                col = if tok.delta_line == 0 {
                    col + tok.delta_start
                } else {
                    tok.delta_start
                };
                let start = index.offset(tower_lsp::lsp_types::Position::new(line, col));
                let text = SRC[start..start + tok.length as usize].to_owned();
                let ty = TYPES[tok.token_type as usize].as_str().to_owned();
                (text, ty, tok.token_modifiers_bitset)
            })
            .collect()
    }

    fn class_of(name: &str, nth: usize) -> (String, u32) {
        let all = classes();
        let (_, ty, m) = all
            .iter()
            .filter(|(t, _, _)| t == name)
            .nth(nth)
            .unwrap_or_else(|| panic!("no {nth}th {name} in {all:?}"));
        (ty.clone(), *m)
    }

    #[test]
    fn declarations_and_uses_are_classed() {
        assert_eq!(class_of("Cell", 0), ("struct".to_owned(), 1));
        assert_eq!(class_of("T", 0), ("typeParameter".to_owned(), 0));
        assert_eq!(class_of("value", 0), ("property".to_owned(), 1));
        assert_eq!(class_of("On", 0), ("enumMember".to_owned(), 1));
        assert_eq!(class_of("get", 0), ("method".to_owned(), 1));
        assert_eq!(class_of("run", 0), ("function".to_owned(), 1));
        assert_eq!(class_of("n", 0), ("parameter".to_owned(), 1));
        assert_eq!(class_of("tag", 1), ("decorator".to_owned(), 0));
        assert_eq!(class_of("int", 0), ("type".to_owned(), 2));
    }

    #[test]
    fn uses_follow_what_they_name() {
        assert_eq!(class_of("value", 2), ("property".to_owned(), 0));
        assert_eq!(class_of("n", 1), ("parameter".to_owned(), 0));
        assert_eq!(class_of("Mode", 1), ("enum".to_owned(), 0));
        assert_eq!(class_of("On", 1), ("enumMember".to_owned(), 0));
        assert_eq!(class_of("i", 1), ("variable".to_owned(), 0));
        assert_eq!(class_of("x", 1), ("variable".to_owned(), 0));
        assert_eq!(class_of("get", 1), ("method".to_owned(), 0));
        assert_eq!(class_of("run", 1), ("function".to_owned(), 0));
        assert_eq!(class_of("print", 0), ("function".to_owned(), 2));
        assert_eq!(class_of("c", 1), ("variable".to_owned(), 0));
    }
}
