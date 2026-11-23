//! Macros a program declares: `macro name(pattern) => template;`.
//!
//! A declaration names a pattern of fragments (`$x: expr`, `$n: ident`,
//! `$t: type`, `$b: block`) and literal tokens, and a template written in
//! candela with `$x` where a fragment goes. A use, `name!( ... )`, matches its
//! region against the pattern and parses the template, with each fragment's
//! text in place, as one expression where the use stands. This happens while
//! the file is read, so the compiler only ever sees the expanded program.
//!
//! The expansion is hygienic in the way a `macro_rules!` one is: a name the
//! template binds (with `let`, `for`, a closure parameter, an `else |e|`
//! binder or a `match` arm) is renamed apart for each expansion, so it neither
//! captures a name the fragments use nor leaks into the code around the use.
//! Every other name in the template is read where the macro is used.

use super::Parser;
use super::ParserErr;
use super::Token;
use crate::cold_path;
use crate::compiler::expr::Span;
use logos::Logos;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use std::cell::Cell;
use std::fmt::Write;
use std::rc::Rc;

/// What one fragment of a pattern matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    /// An expression, up to the literal token that follows it in the pattern.
    Expr,
    /// One name.
    Ident,
    /// A type, up to the literal token that follows it in the pattern.
    Type,
    /// A `{ ... }` block.
    Block,
}

impl FragmentKind {
    fn read(name: &str) -> Option<Self> {
        Some(match name {
            "expr" => Self::Expr,
            "ident" => Self::Ident,
            "type" => Self::Type,
            "block" => Self::Block,
            _ => return None,
        })
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Expr => "expr",
            Self::Ident => "ident",
            Self::Type => "type",
            Self::Block => "block",
        }
    }
}

#[derive(Debug)]
enum PatternPiece {
    /// A token the use writes as it is.
    Literal(SmolStr),
    Fragment(SmolStr, FragmentKind),
}

#[derive(Debug)]
enum TemplatePiece {
    Text(SmolStr),
    /// The text the use gave the fragment at this index of the pattern.
    Fragment(usize),
    /// A name the template binds, renamed apart for each expansion.
    Local(SmolStr),
    /// Another macro's use, `name!( ... )`, whose region the template's
    /// fragments and names reach into; its pieces join with no space.
    Glued(Box<[Self]>),
}

/// One macro a program declares.
#[derive(Debug)]
pub struct MacroDef {
    pub name: SmolStr,
    pattern: Box<[PatternPiece]>,
    template: Box<[TemplatePiece]>,
}

impl MacroDef {
    /// How a use is written, `name!($x: expr, $y: expr)`, for a message to
    /// show.
    #[must_use]
    pub fn usage(&self) -> String {
        let mut out = format!("{}!(", self.name);
        for (i, piece) in self.pattern.iter().enumerate() {
            let text = match piece {
                PatternPiece::Literal(text) => text.to_string(),
                PatternPiece::Fragment(name, kind) => format!("${name}: {}", kind.name()),
            };
            let glued = matches!(text.as_str(), "," | ";" | ")" | "]");
            if i > 0 && !glued {
                out.push(' ');
            }
            out.push_str(&text);
        }
        out.push(')');
        out
    }
}

/// The macros a file can use, by the name a use writes: `name` for one the
/// file declares or imports by name, `module::name` for one reached through a
/// module's name.
pub type MacroScope = FxHashMap<SmolStr, Rc<MacroDef>>;

/// Reads a declaration after its `macro` word: `name(pattern) => template;`.
pub(super) fn parse_macro_decl(parser: &mut Parser<'_>) -> MacroDef {
    let (t, span) = parser.next_token();
    let name = match t {
        Token::Identifier(name) => SmolStr::new(name),
        Token::Macro(m) => {
            cold_path();
            decl_error(
                parser,
                span,
                format!(
                    "A declaration names the macro without the '!': macro {}(...) => ...;",
                    m.name
                ),
            );
        }
        other => {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    other,
                    "A macro declaration is macro name($x: expr) => template;",
                ),
            );
        }
    };
    parser.next_token_expect(
        Token::LParen,
        "A macro's pattern follows its name in parentheses: macro name($x: expr) => template;",
    );
    let pattern = parse_pattern(parser);
    parser.next_token_expect(
        Token::FatArrow,
        "A macro's template follows its pattern after '=>': macro name($x: expr) => template;",
    );
    let template = parse_template(parser, &pattern);
    MacroDef {
        name,
        pattern: pattern.into_boxed_slice(),
        template,
    }
}

#[cold]
#[inline(never)]
fn decl_error(parser: &Parser<'_>, span: Span, message: String) -> ! {
    parser.error(span, ParserErr::MacroDeclaration(message));
}

fn text_of(parser: &Parser<'_>, span: Span) -> SmolStr {
    SmolStr::new(&parser.ctx.src.contents[span.start as usize..span.end as usize])
}

const fn opens(t: &Token<'_>) -> bool {
    matches!(t, Token::LParen | Token::LBracket | Token::LBrace)
}

const fn closes(t: &Token<'_>) -> bool {
    matches!(t, Token::RParen | Token::RBracket | Token::RBrace)
}

/// The pattern, from just past its `(` through the `)` that closes it.
fn parse_pattern(parser: &mut Parser<'_>) -> Vec<PatternPiece> {
    let mut pieces: Vec<PatternPiece> = Vec::new();
    let mut depth = 0u32;
    loop {
        let (t, span) = parser.next_token();
        match t {
            Token::RParen if depth == 0 => break,
            Token::Dollar => {
                let (t, name_span) = parser.next_token();
                let Token::Identifier(name) = t else {
                    cold_path();
                    decl_error(
                        parser,
                        name_span,
                        "A fragment is '$', a name and its kind: $x: expr".to_owned(),
                    );
                };
                parser.next_token_expect(
                    Token::Colon,
                    "A fragment names its kind after a ':': $x: expr, $n: ident, $t: type or $b: block",
                );
                let (t, kind_span) = parser.next_token();
                let kind = match t {
                    Token::Identifier(kind) => FragmentKind::read(kind),
                    _ => None,
                };
                let Some(kind) = kind else {
                    cold_path();
                    decl_error(
                        parser,
                        kind_span,
                        "A fragment's kind is expr, ident, type or block".to_owned(),
                    );
                };
                if pieces
                    .iter()
                    .any(|p| matches!(p, PatternPiece::Fragment(n, _) if n == name))
                {
                    cold_path();
                    decl_error(
                        parser,
                        name_span,
                        format!("The pattern names ${name} twice; give each fragment its own name"),
                    );
                }
                // An expression or a type runs to the token after it, so that
                // token has to be one the use writes as it is.
                if let Some(PatternPiece::Fragment(prev, FragmentKind::Expr | FragmentKind::Type)) =
                    pieces.last()
                {
                    cold_path();
                    decl_error(
                        parser,
                        span,
                        format!(
                            "${prev} runs to the token after it, so a fragment cannot follow it directly. Put a token between them, such as ','"
                        ),
                    );
                }
                pieces.push(PatternPiece::Fragment(SmolStr::new(name), kind));
            }
            _ => {
                if opens(&t) {
                    depth += 1;
                } else if closes(&t) {
                    depth -= 1;
                }
                pieces.push(PatternPiece::Literal(text_of(parser, span)));
            }
        }
    }
    pieces
}

/// One token of a template while it is read.
struct Tt {
    text: SmolStr,
    ident: bool,
    fragment: Option<usize>,
    /// For another macro's use, its text cut at each `$fragment` in it.
    region: Option<Vec<(SmolStr, Option<usize>)>>,
}

impl Tt {
    fn is(&self, text: &str) -> bool {
        self.fragment.is_none() && self.text == text
    }
    fn opens(&self) -> bool {
        self.fragment.is_none() && matches!(self.text.as_str(), "(" | "[" | "{")
    }
    fn closes(&self) -> bool {
        self.fragment.is_none() && matches!(self.text.as_str(), ")" | "]" | "}")
    }
}

/// The template, from just past its `=>` through the `;` that ends the
/// declaration.
fn parse_template(parser: &mut Parser<'_>, pattern: &[PatternPiece]) -> Box<[TemplatePiece]> {
    let mut toks: Vec<Tt> = Vec::new();
    let mut depth = 0u32;
    loop {
        let (t, span) = parser.next_token();
        match t {
            Token::SemiColon if depth == 0 => break,
            Token::Dollar => {
                let (t, name_span) = parser.next_token();
                let Token::Identifier(name) = t else {
                    cold_path();
                    decl_error(
                        parser,
                        name_span,
                        "A '$' in a template is followed by the name of a fragment the pattern declares".to_owned(),
                    );
                };
                let Some(at) = pattern
                    .iter()
                    .position(|p| matches!(p, PatternPiece::Fragment(n, _) if n == name))
                else {
                    cold_path();
                    decl_error(
                        parser,
                        name_span,
                        format!("The pattern declares no fragment ${name}"),
                    );
                };
                toks.push(Tt {
                    text: SmolStr::new(name),
                    ident: false,
                    fragment: Some(at),
                    region: None,
                });
            }
            _ => {
                if opens(&t) {
                    depth += 1;
                } else if closes(&t) {
                    if depth == 0 {
                        cold_path();
                        decl_error(
                            parser,
                            span,
                            "This closes nothing the template opened. A template ends at its ';'"
                                .to_owned(),
                        );
                    }
                    depth -= 1;
                }
                let text = text_of(parser, span);
                let region = matches!(t, Token::Macro(_)).then(|| split_region(&text, pattern));
                toks.push(Tt {
                    text,
                    ident: matches!(t, Token::Identifier(_)),
                    fragment: None,
                    region,
                });
            }
        }
    }
    if toks.is_empty() {
        cold_path();
        let span = parser.eof_span();
        decl_error(
            parser,
            span,
            "A macro's template is the expression it expands to".to_owned(),
        );
    }
    hygiene(&toks)
}

/// Cuts the text of a macro use written in a template at each `$name` that
/// names one of the pattern's fragments.
fn split_region(text: &str, pattern: &[PatternPiece]) -> Vec<(SmolStr, Option<usize>)> {
    let mut parts = Vec::new();
    let mut rest = text;
    let mut plain = String::new();
    while let Some(at) = rest.find('$') {
        plain.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..len];
        match pattern
            .iter()
            .position(|p| matches!(p, PatternPiece::Fragment(n, _) if n == name))
        {
            Some(index) if len > 0 => {
                parts.push((SmolStr::new(&plain), None));
                plain.clear();
                parts.push((SmolStr::new(name), Some(index)));
            }
            _ => plain.push('$'),
        }
        rest = if matches!(parts.last(), Some((_, Some(_)))) && plain.is_empty() {
            &after[len..]
        } else {
            after
        };
    }
    plain.push_str(rest);
    parts.push((SmolStr::new(&plain), None));
    parts
}

/// The pieces of `text`, a stretch of another macro's region, with each name
/// the template binds renamed. A name after a `.` is a field and stays.
fn rename_in(text: &str, bound: &FxHashSet<&str>, out: &mut Vec<TemplatePiece>) {
    let bytes = text.as_bytes();
    let mut plain_from = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let after_dot = text[..start].trim_end().ends_with('.');
            if bound.contains(word) && !after_dot {
                out.push(TemplatePiece::Text(SmolStr::new(&text[plain_from..start])));
                out.push(TemplatePiece::Local(SmolStr::new(word)));
                plain_from = i;
            }
        } else {
            i += 1;
        }
    }
    out.push(TemplatePiece::Text(SmolStr::new(&text[plain_from..])));
}

/// Whether the name token at `i` can be a name a pattern binds: lowercase,
/// not a keyword, not a call, a path, a field label or a struct name.
fn binds_here(toks: &[Tt], i: usize, in_braces: bool) -> bool {
    let t = &toks[i];
    if !t.ident || t.fragment.is_some() || t.text == "_" {
        return false;
    }
    let first = t.text.chars().next().unwrap_or('A');
    if !(first.is_ascii_lowercase() || first == '_') {
        return false;
    }
    if matches!(
        t.text.as_str(),
        "true" | "false" | "null" | "in" | "is" | "as" | "if" | "else"
    ) {
        return false;
    }
    let next = toks.get(i + 1);
    !next.is_some_and(|n| n.is("(") || n.is("::") || n.is(".") || (in_braces && n.is(":")))
}

/// Marks the names a template binds and renames each of their uses.
fn hygiene(toks: &[Tt]) -> Box<[TemplatePiece]> {
    let n = toks.len();
    let mut site = vec![false; n];
    let mut i = 0;
    while i < n {
        let t = &toks[i];
        if t.is("let") || t.is("for") {
            // The pattern runs to the `=` of a let or the `in` of a for.
            let stop: &[&str] = if t.is("let") { &["=", ";"] } else { &["in"] };
            let mut depth = 0i32;
            let mut j = i + 1;
            while j < n {
                let u = &toks[j];
                if u.opens() {
                    depth += 1;
                } else if u.closes() {
                    depth -= 1;
                    if depth < 0 {
                        break;
                    }
                } else if depth == 0 && (stop.iter().any(|s| u.is(s)) || u.is(":")) {
                    break;
                } else if binds_here(toks, j, depth > 0) {
                    site[j] = true;
                }
                j += 1;
            }
        } else if t.is("fn") && toks.get(i + 1).is_some_and(|u| u.is("(")) {
            let mut depth = 1i32;
            let mut j = i + 2;
            while j < n && depth > 0 {
                let u = &toks[j];
                if u.opens() {
                    depth += 1;
                } else if u.closes() {
                    depth -= 1;
                } else if depth == 1
                    && (toks[j - 1].is("(") || toks[j - 1].is(","))
                    && binds_here(toks, j, false)
                {
                    site[j] = true;
                }
                j += 1;
            }
        } else if t.is("else")
            && toks.get(i + 1).is_some_and(|u| u.is("|"))
            && toks.get(i + 3).is_some_and(|u| u.is("|"))
            && binds_here(toks, i + 2, false)
        {
            site[i + 2] = true;
        } else if t.is("match") {
            mark_match_arms(toks, i, &mut site);
        }
        i += 1;
    }
    let bound: FxHashSet<&str> = (0..n)
        .filter(|&i| site[i])
        .map(|i| toks[i].text.as_str())
        .collect();

    let mut out: Vec<TemplatePiece> = Vec::with_capacity(n);
    // For each open `{`, whether it opens a struct literal or pattern, where a
    // lone field name is shorthand for `name: name`.
    let mut braces: Vec<bool> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if let Some(at) = t.fragment {
            out.push(TemplatePiece::Fragment(at));
            continue;
        }
        if let Some(parts) = &t.region {
            let mut glued = Vec::new();
            for (text, fragment) in parts {
                match fragment {
                    Some(at) => glued.push(TemplatePiece::Fragment(*at)),
                    None => rename_in(text, &bound, &mut glued),
                }
            }
            out.push(TemplatePiece::Glued(glued.into_boxed_slice()));
            continue;
        }
        if t.is("{") {
            let after_type = i > 0
                && toks[i - 1].ident
                && toks[i - 1]
                    .text
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase());
            braces.push(after_type);
        } else if t.is("}") {
            braces.pop();
        }
        let renamed = t.ident && bound.contains(t.text.as_str()) && {
            let prev = i.checked_sub(1).map(|p| &toks[p]);
            let next = toks.get(i + 1);
            let after_dot = prev.is_some_and(|p| p.is("."));
            let label = !site[i]
                && next.is_some_and(|n| n.is(":"))
                && prev.is_some_and(|p| p.is("{") || p.is(",") || p.is("("));
            !after_dot && !label
        };
        if !renamed {
            out.push(TemplatePiece::Text(t.text.clone()));
            continue;
        }
        let in_struct = braces.last().copied().unwrap_or(false);
        let prev = &toks[i - 1];
        let next = toks.get(i + 1);
        if in_struct
            && (prev.is("{") || prev.is(","))
            && next.is_some_and(|n| n.is("}") || n.is(","))
        {
            // `P { x }` names the field and the variable alike; only the
            // variable is renamed.
            out.push(TemplatePiece::Text(t.text.clone()));
            out.push(TemplatePiece::Text(SmolStr::new_static(":")));
        }
        out.push(TemplatePiece::Local(t.text.clone()));
    }
    out.into_boxed_slice()
}

/// Marks the names the arms of the `match` at `at` bind: the lowercase names
/// in each arm's pattern, up to its guard or its `=>`.
fn mark_match_arms(toks: &[Tt], at: usize, site: &mut [bool]) {
    let n = toks.len();
    // The arms open at the first `{` not inside the scrutinee's brackets.
    let mut depth = 0i32;
    let mut j = at + 1;
    while j < n {
        if toks[j].is("{") && depth == 0 {
            break;
        }
        if toks[j].opens() {
            depth += 1;
        } else if toks[j].closes() {
            depth -= 1;
        }
        j += 1;
    }
    let mut depth = 0i32;
    let mut in_pattern = true;
    let mut guarded = false;
    j += 1;
    while j < n {
        let u = &toks[j];
        if u.opens() {
            depth += 1;
        } else if u.closes() {
            if depth == 0 {
                return;
            }
            depth -= 1;
            // A block body ends its arm; the next arm may follow with no `,`.
            if depth == 0 && !in_pattern && u.is("}") {
                let comma = toks.get(j + 1).is_some_and(|t| t.is(","));
                if !comma {
                    in_pattern = true;
                    guarded = false;
                }
            }
        } else if depth == 0 && u.is("=>") {
            in_pattern = false;
        } else if depth == 0 && u.is(",") && !in_pattern {
            in_pattern = true;
            guarded = false;
        } else if in_pattern && u.is("if") {
            guarded = true;
        } else if in_pattern && !guarded && binds_here(toks, j, depth > 0) {
            site[j] = true;
        }
        j += 1;
    }
}

thread_local! {
    /// Numbers the expansions on this thread, so the names one renames apart
    /// differ from every other's.
    static EXPANSIONS: Cell<u32> = const { Cell::new(0) };
}

/// Whether an expression fragment's text needs parentheses to stay one
/// operand where the template puts it: it does unless it is a name, a literal
/// or a chain of calls, fields and indexes on one.
fn needs_parens(tokens: &[(Token<'_>, std::ops::Range<usize>)]) -> bool {
    let mut depth = 0i32;
    for (t, _) in tokens {
        if opens(t) {
            depth += 1;
            continue;
        }
        if closes(t) {
            depth -= 1;
            continue;
        }
        if depth > 0 {
            continue;
        }
        let primary = matches!(
            t,
            Token::Identifier(_)
                | Token::Int(_)
                | Token::Float(_)
                | Token::DotDigits(_)
                | Token::String(_)
                | Token::Macro(_)
                | Token::True
                | Token::False
                | Token::Null
                | Token::Dot
                | Token::DoubleColon
                | Token::Question
        );
        if !primary {
            return true;
        }
    }
    false
}

/// Matches the region `body` of a use against `def`'s pattern and answers the
/// text the template expands to, or what did not match.
pub(super) fn expand(def: &MacroDef, body: &str) -> Result<String, String> {
    let mut tokens: Vec<(Token<'_>, std::ops::Range<usize>)> = Vec::new();
    for (t, range) in Token::lexer(body).spanned() {
        let Ok(t) = t else {
            return Err(format!("'{}' does not read as candela", &body[range]));
        };
        tokens.push((t, range));
    }
    let text = |from: usize, to: usize| -> &str {
        if from >= to {
            return "";
        }
        &body[tokens[from].1.start..tokens[to - 1].1.end]
    };
    let mut fragments: Vec<Option<(usize, usize)>> = vec![None; def.pattern.len()];
    let mut pos = 0;
    for (i, piece) in def.pattern.iter().enumerate() {
        match piece {
            PatternPiece::Literal(lit) => match tokens.get(pos) {
                Some((_, range)) if &body[range.clone()] == lit.as_str() => pos += 1,
                Some((_, range)) => {
                    return Err(format!(
                        "expected '{lit}' here, found '{}'",
                        &body[range.clone()]
                    ));
                }
                None => return Err(format!("expected '{lit}' before the region ends")),
            },
            PatternPiece::Fragment(name, FragmentKind::Ident) => match tokens.get(pos) {
                Some((Token::Identifier(_), _)) => {
                    fragments[i] = Some((pos, pos + 1));
                    pos += 1;
                }
                _ => return Err(format!("${name} is one name")),
            },
            PatternPiece::Fragment(name, FragmentKind::Block) => {
                if !matches!(tokens.get(pos), Some((Token::LBrace, _))) {
                    return Err(format!("${name} is a block in braces, {{ ... }}"));
                }
                let mut depth = 0i32;
                let mut end = pos;
                while end < tokens.len() {
                    let t = &tokens[end].0;
                    if opens(t) {
                        depth += 1;
                    } else if closes(t) {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    end += 1;
                }
                if end == tokens.len() {
                    return Err(format!("${name}'s block is never closed"));
                }
                fragments[i] = Some((pos, end + 1));
                pos = end + 1;
            }
            PatternPiece::Fragment(name, kind) => {
                // The fragment runs to the literal after it at its own level,
                // or to the end of the region.
                let stop = match def.pattern.get(i + 1) {
                    Some(PatternPiece::Literal(lit)) => Some(lit.as_str()),
                    _ => None,
                };
                let mut depth = 0i32;
                let mut end = pos;
                while end < tokens.len() {
                    let (t, range) = &tokens[end];
                    if depth == 0 && stop.is_some_and(|s| &body[range.clone()] == s) {
                        break;
                    }
                    if opens(t) || (*kind == FragmentKind::Type && *t == Token::OpInf) {
                        depth += 1;
                    } else if closes(t) || (*kind == FragmentKind::Type && *t == Token::OpSup) {
                        depth -= 1;
                    }
                    end += 1;
                }
                if end == pos {
                    return Err(format!("${name} is missing"));
                }
                fragments[i] = Some((pos, end));
                pos = end;
            }
        }
    }
    if pos < tokens.len() {
        return Err(format!(
            "'{}' is more than the pattern takes",
            text(pos, tokens.len())
        ));
    }
    let id = EXPANSIONS.with(|c| {
        let id = c.get().wrapping_add(1);
        c.set(id);
        id
    });
    let mut out = String::new();
    let write_piece = |out: &mut String, piece: &TemplatePiece| match piece {
        TemplatePiece::Text(t) => out.push_str(t),
        TemplatePiece::Local(name) => {
            let _ = write!(out, "{name}${id}");
        }
        TemplatePiece::Fragment(at) => {
            let (from, to) = fragments[*at].unwrap_or((0, 0));
            let frag = text(from, to);
            let wrap = matches!(
                def.pattern[*at],
                PatternPiece::Fragment(_, FragmentKind::Expr)
            ) && needs_parens(&tokens[from..to]);
            if wrap {
                out.push('(');
                out.push_str(frag);
                out.push(')');
            } else {
                out.push_str(frag);
            }
        }
        TemplatePiece::Glued(_) => {}
    };
    for piece in &def.template {
        if !out.is_empty() {
            out.push(' ');
        }
        if let TemplatePiece::Glued(parts) = piece {
            for part in parts {
                write_piece(&mut out, part);
            }
        } else {
            write_piece(&mut out, piece);
        }
    }
    Ok(out)
}

/// The text an expansion shows a reader: the names it renamed apart read as
/// the template wrote them.
#[must_use]
pub fn readable(expansion: &str) -> String {
    let mut out = String::with_capacity(expansion.len());
    let mut chars = expansion.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek().is_some_and(char::is_ascii_digit) {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}
