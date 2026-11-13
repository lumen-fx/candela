//! A tolerant reading of the buffer around the cursor.
//!
//! The compiler answers what the program declares, but only for a buffer that
//! compiles, and it keeps no type for a local variable. A buffer being edited
//! rarely compiles at the cursor: `bird.` is not a program. This module reads
//! the current text as tokens instead, finds the bindings in scope at the
//! cursor (parameters, `let`s, `for` patterns) and follows an expression's type
//! through the declarations the last good compile recorded (`ProgramSummary`):
//! a parameter's annotation, a struct's fields, a function's return type. It
//! never reports an error; where it cannot tell, the type is unknown.

use crate::analysis::{ProgramSummary, ScopeKind};
use crate::builtins;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ident,
    Number,
    Str,
    Punct,
}

/// A token: its kind and byte range in the text.
#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

/// The text cut into tokens, with the comment ranges kept aside.
pub struct Tokens<'a> {
    pub text: &'a str,
    pub toks: Vec<Tok>,
    comments: Vec<(usize, usize)>,
}

const PUNCTS: [&str; 18] = [
    "...", "::", "->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=",
    "%=", "**", "|>",
];

impl<'a> Tokens<'a> {
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let bytes = text.as_bytes();
        let mut toks = Vec::new();
        let mut comments = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if text[i..].starts_with("//") {
                let end = text[i..].find('\n').map_or(bytes.len(), |n| i + n);
                comments.push((i, end));
                i = end;
            } else if c == b'"' {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                toks.push(Tok {
                    kind: Kind::Str,
                    start,
                    end: i,
                });
            } else if c.is_ascii_digit() {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || (bytes[i] == b'.'
                            && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                            && !text[start..i].contains('.')))
                {
                    i += 1;
                }
                toks.push(Tok {
                    kind: Kind::Number,
                    start,
                    end: i,
                });
            } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] >= 0x80)
                {
                    i += 1;
                }
                toks.push(Tok {
                    kind: Kind::Ident,
                    start,
                    end: i,
                });
            } else {
                let len = PUNCTS
                    .iter()
                    .find(|p| text[i..].starts_with(*p))
                    .map_or(1, |p| p.len());
                toks.push(Tok {
                    kind: Kind::Punct,
                    start: i,
                    end: i + len,
                });
                i += len;
            }
        }
        Self {
            text,
            toks,
            comments,
        }
    }

    #[must_use]
    pub fn s(&self, i: usize) -> &'a str {
        self.toks.get(i).map_or("", |t| &self.text[t.start..t.end])
    }

    #[must_use]
    pub fn kind(&self, i: usize) -> Option<Kind> {
        self.toks.get(i).map(|t| t.kind)
    }

    #[must_use]
    pub fn is_ident(&self, i: usize) -> bool {
        self.kind(i) == Some(Kind::Ident)
    }

    /// Whether `offset` sits inside a comment or a string, where nothing
    /// completes.
    #[must_use]
    pub fn in_comment_or_string(&self, offset: usize) -> bool {
        let in_comment = self
            .comments
            .iter()
            .any(|&(s, e)| s < offset && offset <= e);
        let in_string = self.toks.iter().any(|t| {
            let closed = t.end - t.start >= 2 && self.text[t.start..t.end].ends_with('"');
            t.kind == Kind::Str && t.start < offset && (offset < t.end || !closed)
        });
        in_comment || in_string
    }

    /// The number of tokens that end at or before `offset`.
    #[must_use]
    pub fn count_before(&self, offset: usize) -> usize {
        self.toks.partition_point(|t| t.end <= offset)
    }

    /// The index of the token that covers `offset`, an identifier touching it
    /// first.
    #[must_use]
    pub fn at(&self, offset: usize) -> Option<usize> {
        let i = self.toks.partition_point(|t| t.end < offset);
        let covers = |i: usize| {
            self.toks
                .get(i)
                .is_some_and(|t| t.start <= offset && offset <= t.end)
        };
        if covers(i) && self.is_ident(i) {
            Some(i)
        } else if covers(i + 1) && self.is_ident(i + 1) {
            Some(i + 1)
        } else if covers(i) {
            Some(i)
        } else {
            None
        }
    }

    /// The index of the token closing the bracket opened at `open`, or the
    /// token count when it never closes.
    #[must_use]
    pub fn closing(&self, open: usize) -> usize {
        let mut depth = 0usize;
        for i in open..self.toks.len() {
            match self.s(i) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        self.toks.len()
    }

    /// The brackets still open after the first `n` tokens, outermost first.
    #[must_use]
    pub fn open_brackets(&self, n: usize) -> Vec<usize> {
        let mut stack = Vec::new();
        for i in 0..n.min(self.toks.len()) {
            match self.s(i) {
                "(" | "[" | "{" => stack.push(i),
                ")" | "]" | "}" => {
                    stack.pop();
                }
                _ => {}
            }
        }
        stack
    }

    /// Splits `start..end` at the commas outside any bracket.
    #[must_use]
    pub fn split_commas(&self, start: usize, end: usize) -> Vec<(usize, usize)> {
        let mut parts = Vec::new();
        let mut depth = 0i32;
        let mut from = start;
        for i in start..end {
            match self.s(i) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                "," if depth == 0 => {
                    parts.push((from, i));
                    from = i + 1;
                }
                _ => {}
            }
        }
        parts.push((from, end));
        parts
    }

    /// Where the postfix chain that ends just before `end` starts: `a`,
    /// `a.b[0].c(x)`, `Name { .. }.f`, `m::f(x)`, `(a, b)`.
    #[must_use]
    pub fn chain_start(&self, end: usize) -> usize {
        let mut i = end;
        while i > 0 {
            let mut j = i - 1;
            // Calls, indexes and a struct literal's braces trail their head.
            while matches!(self.s(j), ")" | "]" | "}") {
                let Some(open) = self.opening(j) else {
                    return i;
                };
                let headed =
                    open > 0 && (self.is_ident(open - 1) || matches!(self.s(open - 1), ")" | "]"));
                if self.s(j) == "}" && !(open > 0 && self.is_ident(open - 1)) {
                    return i;
                }
                if !headed {
                    // A parenthesised expression or a list literal.
                    return open;
                }
                j = open - 1;
            }
            if !(self.is_ident(j) || self.kind(j) == Some(Kind::Number)) {
                return i;
            }
            if j > 0 && matches!(self.s(j - 1), "." | "::") {
                i = j - 1;
            } else {
                return j;
            }
        }
        i
    }

    /// The index of the bracket that `close` closes.
    #[must_use]
    pub fn opening(&self, close: usize) -> Option<usize> {
        let mut depth = 0usize;
        for i in (0..=close).rev() {
            match self.s(i) {
                ")" | "]" | "}" => depth += 1,
                "(" | "[" | "{" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// A type as the LSP follows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    /// A struct, an enum, a builtin type or `any`, by the name the compiler's
    /// tables give it (`Vec2`, `State<Phase>`, `int`).
    Named(String),
    List(Box<Ty>),
    Tuple(Vec<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Unknown,
}

impl Ty {
    /// The type written the way the compiler renders it, which is also how a
    /// struct of that type is named in its tables.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Named(n) => n.clone(),
            Self::List(t) => format!("{}[]", t.render()),
            Self::Tuple(ts) => format!(
                "({})",
                ts.iter().map(Self::render).collect::<Vec<_>>().join(", ")
            ),
            Self::Map(k, v) => format!("{{{}: {}}}", k.render(), v.render()),
            Self::Unknown => "any".to_owned(),
        }
    }

    /// The type of one element when iterating with `for`.
    #[must_use]
    pub fn element(&self) -> Self {
        match self {
            Self::List(t) => (**t).clone(),
            Self::Map(k, _) => (**k).clone(),
            Self::Named(n) if n == "string" => self.clone(),
            _ => Self::Unknown,
        }
    }

    /// Reads a type written as text (`(Bird, Transform)[]`).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let toks = Tokens::new(text);
        let mut i = 0;
        parse_type(&toks, &mut i, toks.toks.len())
    }
}

/// Reads a type from `toks[*i..end]`, advancing `i` past it.
pub fn parse_type(toks: &Tokens<'_>, i: &mut usize, end: usize) -> Ty {
    let mut ty = match toks.s(*i) {
        "(" => {
            let close = toks.closing(*i).min(end);
            let mut parts = Vec::new();
            for (s, e) in toks.split_commas(*i + 1, close) {
                if s < e {
                    let mut j = s;
                    parts.push(parse_type(toks, &mut j, e));
                }
            }
            *i = close + 1;
            if parts.len() == 1 {
                parts.pop().unwrap_or(Ty::Unknown)
            } else {
                Ty::Tuple(parts)
            }
        }
        "{" => {
            let close = toks.closing(*i).min(end);
            let mut j = *i + 1;
            let k = parse_type(toks, &mut j, close);
            if toks.s(j) == ":" {
                j += 1;
            }
            let v = parse_type(toks, &mut j, close);
            *i = close + 1;
            Ty::Map(Box::new(k), Box::new(v))
        }
        "fn" => {
            // A function type: skip its parameters and return type.
            *i += 1;
            if toks.s(*i) == "(" {
                *i = toks.closing(*i) + 1;
            }
            if toks.s(*i) == "->" {
                *i += 1;
                let _ = parse_type(toks, i, end);
            }
            Ty::Unknown
        }
        _ if toks.is_ident(*i) => {
            let mut name = toks.s(*i).to_owned();
            *i += 1;
            // A module path in front of the name: the tables name the type
            // without it.
            while toks.s(*i) == "::" && toks.is_ident(*i + 1) {
                name = toks.s(*i + 1).to_owned();
                *i += 2;
            }
            if toks.s(*i) == "<" {
                *i += 1;
                let mut args = Vec::new();
                while *i < end && toks.s(*i) != ">" {
                    args.push(parse_type(toks, i, end).render());
                    if toks.s(*i) == "," {
                        *i += 1;
                    } else if toks.s(*i) != ">" {
                        break;
                    }
                }
                *i += 1;
                name = format!("{name}<{}>", args.join(", "));
            }
            Ty::Named(name)
        }
        _ => {
            *i += 1;
            return Ty::Unknown;
        }
    };
    loop {
        if toks.s(*i) == "[" && toks.s(*i + 1) == "]" {
            ty = Ty::List(Box::new(ty));
            *i += 2;
        } else if toks.s(*i) == "?" {
            *i += 1;
        } else if toks.s(*i) == "|" {
            *i += 1;
            let _ = parse_type(toks, i, end);
            ty = Ty::Unknown;
        } else {
            break;
        }
    }
    ty
}

/// How a binding's type is found.
#[derive(Clone, Debug)]
pub enum Source {
    Known(Ty),
    /// The type of the expression in `start..end`.
    Expr(usize, usize),
    /// One element of iterating the expression in `start..end`, then the
    /// tuple member at the given index when the pattern destructures.
    Element(usize, usize, Option<usize>),
    /// The tuple member at the given index of the expression.
    Member(usize, usize, usize),
}

/// A name bound in a scope.
#[derive(Clone, Debug)]
pub struct Binding {
    pub name: String,
    /// Where the name is written.
    pub at: usize,
    pub source: Source,
}

/// The bindings visible at a point of the buffer, innermost last.
pub struct Scope {
    pub bindings: Vec<Binding>,
    /// The type an enclosing `impl` block attaches to.
    pub impl_type: Option<String>,
}

/// Reads the scopes the first `n` tokens open and returns the bindings still
/// visible after them.
#[must_use]
pub fn scope_at(toks: &Tokens<'_>, n: usize) -> Scope {
    struct Frame {
        bindings: Vec<Binding>,
        impl_type: Option<String>,
    }
    let mut frames: Vec<Frame> = vec![Frame {
        bindings: Vec::new(),
        impl_type: None,
    }];
    let mut pending: Vec<Binding> = Vec::new();
    let mut pending_impl: Option<String> = None;
    let n = n.min(toks.toks.len());
    let mut i = 0;
    while i < n {
        match toks.s(i) {
            "{" => {
                let impl_type = pending_impl
                    .take()
                    .or_else(|| frames.last().and_then(|f| f.impl_type.clone()));
                frames.push(Frame {
                    bindings: std::mem::take(&mut pending),
                    impl_type,
                });
            }
            "}" => {
                if frames.len() > 1 {
                    frames.pop();
                }
            }
            ";" => pending.clear(),
            "impl" if toks.is_ident(i + 1) => {
                let mut j = i + 1;
                let ty = parse_type(toks, &mut j, n);
                pending_impl = Some(ty.render());
                i = j;
                continue;
            }
            "fn" => {
                let mut j = i + 1;
                if toks.is_ident(j) {
                    j += 1;
                }
                if toks.s(j) == "<" {
                    while j < n && toks.s(j) != ">" {
                        j += 1;
                    }
                    j += 1;
                }
                if toks.s(j) == "(" {
                    let close = toks.closing(j);
                    let impl_type = frames.last().and_then(|f| f.impl_type.clone());
                    pending.clear();
                    for (s, e) in toks.split_commas(j + 1, close.min(n)) {
                        if s >= e || !toks.is_ident(s) {
                            continue;
                        }
                        let name = toks.s(s).to_owned();
                        let source = if toks.s(s + 1) == ":" {
                            let mut k = s + 2;
                            Source::Known(parse_type(toks, &mut k, e))
                        } else if name == "self" {
                            impl_type
                                .clone()
                                .map_or(Source::Known(Ty::Unknown), |t| Source::Known(Ty::Named(t)))
                        } else {
                            Source::Known(Ty::Unknown)
                        };
                        pending.push(Binding {
                            name,
                            at: toks.toks[s].start,
                            source,
                        });
                    }
                    // Inside the parameter list itself, its names are not yet
                    // in scope; past it they wait for the body's brace.
                    if close >= n {
                        break;
                    }
                    i = close + 1;
                    continue;
                }
            }
            "let" => {
                // The whole statement, even where it runs past the cursor:
                // the name being declared is typed by its value.
                let all = toks.toks.len();
                let end = (i..all).find(|&k| toks.s(k) == ";").unwrap_or(all);
                let eq = (i..end).find(|&k| toks.s(k) == "=");
                let (value_start, value_end) = eq.map_or((end, end), |eq| (eq + 1, end));
                let frame = frames.last_mut().expect("the file frame is never popped");
                let mut j = i + 1;
                if toks.s(j) == "(" {
                    let close = toks.closing(j);
                    for (idx, (s, e)) in toks.split_commas(j + 1, close).into_iter().enumerate() {
                        if s < e && toks.is_ident(s) && toks.s(s) != "_" {
                            frame.bindings.push(Binding {
                                name: toks.s(s).to_owned(),
                                at: toks.toks[s].start,
                                source: Source::Member(value_start, value_end, idx),
                            });
                        }
                    }
                } else if toks.is_ident(j) {
                    let name = toks.s(j).to_owned();
                    let at = toks.toks[j].start;
                    j += 1;
                    let source = if toks.s(j) == ":" {
                        j += 1;
                        Source::Known(parse_type(toks, &mut j, eq.unwrap_or(end)))
                    } else {
                        Source::Expr(value_start, value_end)
                    };
                    frame.bindings.push(Binding { name, at, source });
                }
                // The value may hold closures and blocks: read on into it.
                i = eq.map_or(i + 1, |eq| eq + 1);
                continue;
            }
            "for" => {
                let Some(in_at) = (i + 1..toks.toks.len()).find(|&k| toks.s(k) == "in") else {
                    break;
                };
                let body = (in_at + 1..toks.toks.len())
                    .find(|&k| toks.s(k) == "{")
                    .unwrap_or(toks.toks.len());
                let (expr_start, expr_end) = (in_at + 1, body);
                pending.clear();
                if toks.s(i + 1) == "(" {
                    let close = toks.closing(i + 1);
                    for (idx, (s, e)) in toks
                        .split_commas(i + 2, close.min(in_at))
                        .into_iter()
                        .enumerate()
                    {
                        if s < e && toks.is_ident(s) && toks.s(s) != "_" {
                            pending.push(Binding {
                                name: toks.s(s).to_owned(),
                                at: toks.toks[s].start,
                                source: Source::Element(expr_start, expr_end, Some(idx)),
                            });
                        }
                    }
                } else if toks.is_ident(i + 1) {
                    pending.push(Binding {
                        name: toks.s(i + 1).to_owned(),
                        at: toks.toks[i + 1].start,
                        source: Source::Element(expr_start, expr_end, None),
                    });
                }
                i = in_at + 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    let impl_type = frames.last().and_then(|f| f.impl_type.clone());
    let mut bindings: Vec<Binding> = frames.into_iter().flat_map(|f| f.bindings).collect();
    // A parameter list the cursor has passed but whose body has not opened
    // yet still names what the body will see.
    bindings.extend(pending);
    Scope {
        bindings,
        impl_type,
    }
}

impl Scope {
    /// The innermost binding named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Binding> {
        self.bindings.iter().rev().find(|b| b.name == name)
    }
}

/// Follows types through the buffer's text and the program's declarations.
pub struct Infer<'a> {
    pub toks: &'a Tokens<'a>,
    pub summary: &'a ProgramSummary,
    pub scope: &'a Scope,
    depth: std::cell::Cell<u32>,
}

impl<'a> Infer<'a> {
    #[must_use]
    pub const fn new(toks: &'a Tokens<'a>, summary: &'a ProgramSummary, scope: &'a Scope) -> Self {
        Self {
            toks,
            summary,
            scope,
            depth: std::cell::Cell::new(0),
        }
    }

    /// The type of a binding.
    #[must_use]
    pub fn binding(&self, b: &Binding) -> Ty {
        if self.depth.get() > 16 {
            return Ty::Unknown;
        }
        self.depth.set(self.depth.get() + 1);
        let ty = match &b.source {
            Source::Known(t) => t.clone(),
            Source::Expr(s, e) => self.expr(*s, *e),
            Source::Element(s, e, idx) => {
                let elem = if self.has_range(*s, *e) {
                    Ty::Named("int".to_owned())
                } else {
                    self.expr(*s, *e).element()
                };
                match idx {
                    Some(i) => member(&elem, *i),
                    None => elem,
                }
            }
            Source::Member(s, e, idx) => member(&self.expr(*s, *e), *idx),
        };
        self.depth.set(self.depth.get() - 1);
        ty
    }

    fn has_range(&self, s: usize, e: usize) -> bool {
        let mut depth = 0i32;
        for i in s..e {
            match self.toks.s(i) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth -= 1,
                ".." if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    /// The type of the expression `toks[s..e]`.
    #[must_use]
    pub fn expr(&self, s: usize, e: usize) -> Ty {
        let t = self.toks;
        if s >= e {
            return Ty::Unknown;
        }
        match t.s(s) {
            "!" => return Ty::Named("bool".to_owned()),
            "-" => return self.expr(s + 1, e),
            _ => {}
        }
        let mut i = s;
        let ty = self.postfix(&mut i, e);
        if i >= e {
            return ty;
        }
        // A binary operator after the first operand.
        match t.s(i) {
            "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" | "is" => {
                Ty::Named("bool".to_owned())
            }
            ".." => Ty::List(Box::new(Ty::Named("int".to_owned()))),
            _ => ty,
        }
    }

    /// Reads a primary expression and its postfix operations from `*i`.
    fn postfix(&self, i: &mut usize, e: usize) -> Ty {
        let t = self.toks;
        let mut ty = self.primary(i, e);
        while *i < e {
            match t.s(*i) {
                "." => {
                    let name = t.s(*i + 1);
                    if t.s(*i + 2) == "(" {
                        ty = self.method_return(&ty, name);
                        *i = t.closing(*i + 2) + 1;
                    } else {
                        ty = self.field(&ty, name);
                        *i += 2;
                    }
                }
                "[" => {
                    let close = t.closing(*i);
                    let slice = (*i..close).any(|k| t.s(k) == "..");
                    ty = match (&ty, slice) {
                        (_, true) => ty,
                        (Ty::List(inner), false) => (**inner).clone(),
                        (Ty::Map(_, v), false) => (**v).clone(),
                        (Ty::Named(n), false) if n == "string" => ty,
                        _ => Ty::Unknown,
                    };
                    *i = close + 1;
                }
                "as" => {
                    *i += 1;
                    ty = parse_type(t, i, e);
                }
                "?" => *i += 1,
                _ => break,
            }
        }
        ty
    }

    fn primary(&self, i: &mut usize, e: usize) -> Ty {
        let t = self.toks;
        let s = *i;
        match t.kind(s) {
            Some(Kind::Number) => {
                *i += 1;
                let float = t.s(s).contains('.');
                return Ty::Named(if float { "float" } else { "int" }.to_owned());
            }
            Some(Kind::Str) => {
                *i += 1;
                return Ty::Named("string".to_owned());
            }
            _ => {}
        }
        match t.s(s) {
            "true" | "false" => {
                *i += 1;
                return Ty::Named("bool".to_owned());
            }
            "[" => {
                let close = t.closing(s);
                *i = close + 1;
                let first = t.split_commas(s + 1, close.min(e))[0];
                return Ty::List(Box::new(self.expr(first.0, first.1)));
            }
            "(" => {
                let close = t.closing(s);
                *i = close + 1;
                let parts = t.split_commas(s + 1, close.min(e));
                if parts.len() == 1 {
                    return self.expr(parts[0].0, parts[0].1);
                }
                return Ty::Tuple(parts.iter().map(|(a, b)| self.expr(*a, *b)).collect());
            }
            _ => {}
        }
        if !t.is_ident(s) {
            *i += 1;
            return Ty::Unknown;
        }
        // A name, a path, a call or a struct literal.
        let mut path = vec![t.s(s)];
        *i += 1;
        while t.s(*i) == "::" && t.is_ident(*i + 1) {
            path.push(t.s(*i + 1));
            *i += 2;
        }
        // `first<int>(xs)` or `Cell<int> { .. }`: a type list counts only
        // where it closes right before a call or a literal.
        let mut generic = String::new();
        if t.s(*i) == "<" {
            let mut depth = 0i32;
            let close = (*i..e).find(|&k| {
                match t.s(k) {
                    "<" => depth += 1,
                    ">" => depth -= 1,
                    _ => {}
                }
                depth == 0
            });
            if let Some(close) = close
                && matches!(t.s(close + 1), "(" | "{")
            {
                let mut k = *i - 1;
                if let Ty::Named(n) = parse_type(t, &mut k, close + 1) {
                    generic = n;
                }
                *i = close + 1;
            }
        }
        let label = path.join("::");
        let last = *path.last().unwrap_or(&"");
        match t.s(*i) {
            "(" => {
                *i = t.closing(*i) + 1;
                if let Some(f) = self.summary.function(&label) {
                    return f.ret.as_deref().map_or(Ty::Unknown, Ty::parse);
                }
                if path.len() == 1
                    && let Some(ret) = builtin_return(builtins::BUILTIN_FUNCTIONS, last)
                {
                    return ret;
                }
                if path.len() >= 2 {
                    let owner = path[path.len() - 2];
                    // A function of an `impl` block, called through its type.
                    if let Some(m) = self.summary.method(owner, last) {
                        return m.ret.as_deref().map_or(Ty::Unknown, Ty::parse);
                    }
                    // A variant built with its payload.
                    if self.summary.enum_named(owner).is_some() {
                        return Ty::Named(owner.to_owned());
                    }
                }
                Ty::Unknown
            }
            "{" if self.is_struct(&label, &generic) => {
                *i = t.closing(*i) + 1;
                Ty::Named(if generic.is_empty() {
                    last.to_owned()
                } else {
                    generic
                })
            }
            _ => {
                if path.len() >= 2 {
                    let owner = path[path.len() - 2];
                    if self.summary.enum_named(owner).is_some() {
                        return Ty::Named(owner.to_owned());
                    }
                    if let Some(f) = self.summary.function(&label)
                        && f.is_const
                    {
                        return f.ret.as_deref().map_or(Ty::Unknown, Ty::parse);
                    }
                    return Ty::Unknown;
                }
                if let Some(b) = self.scope.get(last) {
                    return self.binding(b);
                }
                if let Some(f) = self.summary.function(last)
                    && f.is_const
                {
                    return f.ret.as_deref().map_or(Ty::Unknown, Ty::parse);
                }
                Ty::Unknown
            }
        }
    }

    fn is_struct(&self, label: &str, generic: &str) -> bool {
        if !generic.is_empty() {
            return true;
        }
        matches!(self.summary.lookup(label), Some(ScopeKind::Struct(_)))
    }

    /// The type of `ty.name`.
    #[must_use]
    pub fn field(&self, ty: &Ty, name: &str) -> Ty {
        match ty {
            Ty::Named(n) => self
                .summary
                .struct_named(n)
                .and_then(|s| s.fields.iter().find(|f| f.name == name))
                .map_or(Ty::Unknown, |f| Ty::parse(&f.ty)),
            Ty::Tuple(_) => name.parse().map_or(Ty::Unknown, |i| member(ty, i)),
            _ => Ty::Unknown,
        }
    }

    /// The type `ty.name(...)` returns.
    #[must_use]
    pub fn method_return(&self, ty: &Ty, name: &str) -> Ty {
        if let Ty::Named(n) = ty
            && let Some(m) = self.summary.method(n, name)
        {
            return m.ret.as_deref().map_or(Ty::Unknown, Ty::parse);
        }
        let ret = builtin_return(builtins::BUILTIN_METHODS, name).unwrap_or(Ty::Unknown);
        // `T` in a builtin's signature is the receiver's element.
        match (&ret, ty) {
            (Ty::Named(n), Ty::List(inner)) if n == "T" => (**inner).clone(),
            (Ty::List(r), _) if **r == Ty::Named("T".to_owned()) => ty.clone(),
            _ => ret,
        }
    }
}

/// The member at `idx` of a tuple type.
#[must_use]
pub fn member(ty: &Ty, idx: usize) -> Ty {
    match ty {
        Ty::Tuple(ts) => ts.get(idx).cloned().unwrap_or(Ty::Unknown),
        _ => Ty::Unknown,
    }
}

/// The return type a builtin's documented signature names.
fn builtin_return(table: &[(&str, &str)], name: &str) -> Option<Ty> {
    let (_, doc) = table.iter().find(|(n, _)| *n == name)?;
    let first = doc.lines().next()?;
    let ret = first.rsplit_once("->")?.1.trim();
    Some(Ty::parse(ret))
}

#[cfg(test)]
mod tests {
    use super::{Tokens, Ty, scope_at};

    #[test]
    fn types_read_from_text() {
        let named = |s: &str| Ty::Named(s.to_owned());
        assert_eq!(
            Ty::parse("{string: int}"),
            Ty::Map(Box::new(named("string")), Box::new(named("int")))
        );
        assert_eq!(Ty::parse("{string: int}").render(), "{string: int}");
        assert_eq!(Ty::parse("fn(int) -> int"), Ty::Unknown);
        assert_eq!(Ty::parse("fn()"), Ty::Unknown);
        assert_eq!(Ty::parse("geo::Point"), named("Point"));
        assert_eq!(Ty::parse("int | string"), Ty::Unknown);
        assert_eq!(Ty::parse("Vec2?"), named("Vec2"));
        assert_eq!(Ty::parse("(int)"), named("int"));
        assert_eq!(Ty::parse("Cell<int, (A, B)>"), named("Cell<int, (A, B)>"));
        assert_eq!(Ty::parse("Cell<int"), named("Cell<int>"));
        assert_eq!(Ty::parse("!"), Ty::Unknown);
        assert_eq!(Ty::Unknown.render(), "any");
        assert_eq!(Ty::parse("{string: int}").element(), named("string"));
        assert_eq!(named("string").element(), named("string"));
        assert_eq!(named("int").element(), Ty::Unknown);
    }

    #[test]
    fn tokens_and_brackets() {
        let toks = Tokens::new("a(b, [c]) // done\nx");
        assert!(toks.in_comment_or_string(13));
        assert_eq!(toks.closing(1), 7);
        assert_eq!(toks.opening(7), Some(1));
        assert_eq!(toks.opening(0), None);
        assert_eq!(Tokens::new("f(").closing(1), 2);
        // A space between two tokens is covered by neither.
        let gap = Tokens::new("a  b");
        assert_eq!(gap.at(2), None);
        assert_eq!(Tokens::new("a.b").at(1), Some(0));
        assert_eq!(Tokens::new("a  .  b").at(3), Some(1));
        // A chain stops at a brace that is no struct literal, and at an
        // unmatched closer.
        let block = Tokens::new("f() { }.y");
        assert_eq!(block.chain_start(5), 5);
        let stray = Tokens::new(") .y");
        assert_eq!(stray.chain_start(1), 1);
        let op = Tokens::new("a + .y");
        assert_eq!(op.chain_start(2), 2);
        let start = Tokens::new(".y");
        assert_eq!(start.chain_start(0), 0);
        assert!(Tokens::new("\"open").in_comment_or_string(3));
    }

    #[test]
    fn scopes_of_generic_functions_and_untyped_parameters() {
        let toks = Tokens::new("fn id<T>(x, y: T) {\n let a: int = 1;\n for z in w {");
        let scope = scope_at(&toks, toks.toks.len());
        let names: Vec<&str> = scope.bindings.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["x", "y", "a", "z"]);
        // A `for` with no `in` yet binds nothing.
        let toks = Tokens::new("fn f() { for (a, b)");
        assert!(scope_at(&toks, toks.toks.len()).bindings.is_empty());
    }
}
