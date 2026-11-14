//! The position-based features: completion, hover, signature help and
//! go-to-definition.
//!
//! Each reads the current text through `crate::syntax` to learn what the
//! cursor is on, and the last good compile's `ProgramSummary` to learn what
//! the names there declare. Nothing in here names a particular host: what an
//! attribute accepts comes from its `host` block declaration, and which items
//! an attribute argument should offer first comes from the attributes the
//! program already wrote.

use crate::analysis::{
    AttrParam, AttrParamKind, AttributeSymbol, EnumSymbol, FieldSymbol, FunctionSymbol,
    ProgramSummary, ScopeKind, StructSymbol, UseOn, VariantSymbol, summary,
};
use crate::builtins;
use crate::syntax::{Infer, Scope, Source, Tokens, Ty, scope_at};
use candela::compiler::expr::Span;
use std::collections::HashSet;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
    ParameterInformation, ParameterLabel, SignatureHelp, SignatureInformation,
};

/// The builtin types a type annotation can name.
const BUILTIN_TYPES: &[&str] = &["int", "float", "bool", "string", "any"];

/// A declaration the cursor can be on.
#[derive(Debug, Clone)]
pub enum Symbol<'a> {
    Function(&'a FunctionSymbol),
    Struct(&'a StructSymbol),
    Enum(&'a EnumSymbol),
    Variant(&'a EnumSymbol, &'a VariantSymbol),
    Field(&'a StructSymbol, &'a FieldSymbol),
    Attribute(&'a AttributeSymbol),
    AttrParam(&'a AttributeSymbol, &'a AttrParam),
    /// A parameter or a local variable of the buffer, written at `at`.
    Local {
        name: String,
        ty: Ty,
        at: usize,
    },
    Builtin(&'static str, &'static str),
}

/// Where a declaration is written: a file of the program and a span in it.
pub struct Target {
    pub src_file: u16,
    pub span: Span,
}

/// The editor's view of one buffer at one cursor.
pub struct Ide<'a> {
    toks: Tokens<'a>,
    summary: &'a ProgramSummary,
    offset: usize,
}

/// An attribute use the cursor is inside the parentheses of.
struct AttrArgs<'a> {
    attr: &'a AttributeSymbol,
    /// The arguments already written, by name.
    written: Vec<String>,
    /// The parameter whose value the cursor is on, when it is past `name:`.
    param: Option<&'a AttrParam>,
    /// The values the list under the cursor already holds.
    listed: Vec<String>,
    /// The index of the argument the cursor is in.
    index: usize,
}

impl<'a> Ide<'a> {
    #[must_use]
    pub fn new(text: &'a str, summary: &'a ProgramSummary, offset: usize) -> Self {
        Self {
            toks: Tokens::new(text),
            summary,
            offset: offset.min(text.len()),
        }
    }

    fn text(&self) -> &'a str {
        self.toks.text
    }

    /// Where the identifier the cursor is typing starts.
    fn word_start(&self) -> usize {
        let before = &self.text()[..self.offset];
        before
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map_or(self.offset, |(i, _)| i)
    }

    fn scope(&self, n: usize) -> Scope {
        scope_at(&self.toks, n)
    }

    /// The path written before `end` (`a::b::` when `end` is past the last
    /// `::`), joined, and where it starts.
    fn path_before(&self, end: usize) -> (String, usize) {
        let t = &self.toks;
        let mut parts = Vec::new();
        let mut i = end;
        while i >= 2 && t.s(i - 1) == "::" && t.is_ident(i - 2) {
            parts.push(t.s(i - 2));
            i -= 2;
        }
        parts.reverse();
        (parts.join("::"), i)
    }

    /// The attribute use whose argument list holds the first `n` tokens'
    /// end, if any.
    fn attr_args(&self, n: usize) -> Option<AttrArgs<'a>> {
        let t = &self.toks;
        let open = t.open_brackets(n);
        // The innermost `(`, with only list brackets inside it.
        let mut k = open.len();
        while k > 0 && t.s(open[k - 1]) == "[" {
            k -= 1;
        }
        if k == 0 || t.s(open[k - 1]) != "(" {
            return None;
        }
        let paren = open[k - 1];
        let list = open.get(k).copied();
        if paren == 0 || !t.is_ident(paren - 1) {
            return None;
        }
        let mut name = vec![t.s(paren - 1)];
        let mut j = paren - 1;
        while j >= 2 && t.s(j - 1) == "::" && t.is_ident(j - 2) {
            name.push(t.s(j - 2));
            j -= 2;
        }
        if j == 0 || t.s(j - 1) != "@" {
            return None;
        }
        name.reverse();
        let attr = self.summary.attribute(&name.join("::"))?;
        let parts = t.split_commas(paren + 1, n);
        let index = parts.len() - 1;
        let arg_name = |(s, e): (usize, usize)| (s + 1 < e && t.s(s + 1) == ":").then(|| t.s(s));
        let written = parts[..index]
            .iter()
            .filter_map(|p| arg_name(*p))
            .map(str::to_owned)
            .collect();
        let current = parts[index];
        let param = arg_name(current).and_then(|n| attr.params.iter().find(|p| p.name == n));
        let listed = list.map_or_else(Vec::new, |l| {
            t.split_commas(l + 1, n)
                .iter()
                .rev()
                .skip(1)
                .filter(|(s, e)| s < e)
                .map(|(s, e)| t.text[t.toks[*s].start..t.toks[e - 1].end].to_owned())
                .collect()
        });
        Some(AttrArgs {
            attr,
            written,
            param,
            listed,
            index,
        })
    }

    /// The struct literal whose braces hold the first `n` tokens' end: its
    /// struct and the brace's index.
    fn struct_literal(&self, n: usize) -> Option<(&'a StructSymbol, usize)> {
        let t = &self.toks;
        let brace = *t.open_brackets(n).last()?;
        if t.s(brace) != "{" || brace == 0 {
            return None;
        }
        // `Name { .. }` or `Name<T> { .. }`.
        let mut j = brace - 1;
        let mut generic = None;
        if t.s(j) == ">" {
            let open = (0..j).rev().find(|&k| t.s(k) == "<")?;
            let mut k = open - 1;
            generic = Some(crate::syntax::parse_type(t, &mut k, brace));
            j = open - 1;
        }
        if !t.is_ident(j) {
            return None;
        }
        if j > 0
            && matches!(
                t.s(j - 1),
                "struct" | "impl" | "enum" | "->" | "::" | "is" | "as"
            )
        {
            return None;
        }
        let name = match generic {
            Some(Ty::Named(n)) => n,
            _ => t.s(j).to_owned(),
        };
        let s = match self.summary.lookup(&name) {
            Some(ScopeKind::Struct(Some(id))) => self.summary.structs.get(id),
            _ => self.summary.struct_named(&name),
        }?;
        Some((s, brace))
    }

    /// The declaration the attribute written before the cursor goes on:
    /// `fn`, `struct` or `enum`, and the item's name.
    fn decorated(&self) -> (Option<&'static str>, Option<&'a str>) {
        let t = &self.toks;
        let start = t.count_before(self.offset);
        let mut depth = 0i32;
        for i in start..t.toks.len() {
            match t.s(i) {
                "(" | "[" => depth += 1,
                ")" | "]" => depth -= 1,
                "fn" | "struct" | "enum" if depth <= 0 => {
                    let kind = match t.s(i) {
                        "fn" => "fn",
                        "struct" => "struct",
                        _ => "enum",
                    };
                    let name = t.is_ident(i + 1).then(|| t.s(i + 1));
                    return (Some(kind), name);
                }
                "{" | "}" | ";" if depth <= 0 => break,
                _ => {}
            }
        }
        (None, None)
    }

    // ---- Completion ----

    /// The completions at the cursor.
    #[must_use]
    pub fn completions(&self) -> Vec<CompletionItem> {
        let mut out = Items::default();
        if self.toks.in_comment_or_string(self.offset) {
            return out.items;
        }
        let t = &self.toks;
        let start = self.word_start();
        let n = t.count_before(start);
        let prev = if n > 0 { t.s(n - 1) } else { "" };

        // `@name`, `@module::name`.
        let (path, path_start) = self.path_before(n);
        if prev == "@" || (path_start > 0 && t.s(path_start - 1) == "@" && !path.is_empty()) {
            self.complete_attribute_names(&mut out, &path);
            return out.items;
        }
        // `Enum::`, `module::`.
        if prev == "::" {
            self.complete_path(&mut out, &path);
            return out.items;
        }
        // `value.`
        if prev == "." {
            let chain = t.chain_start(n - 1);
            let scope = self.scope(chain);
            let infer = Infer::new(t, self.summary, &scope);
            let ty = infer.expr(chain, n - 1);
            self.complete_members(&mut out, &ty);
            return out.items;
        }
        if let Some(args) = self.attr_args(n) {
            self.complete_attribute_args(&mut out, &args);
            return out.items;
        }
        if let Some((s, brace)) = self.struct_literal(n) {
            let parts = t.split_commas(brace + 1, n);
            let (cs, ce) = *parts.last().unwrap_or(&(n, n));
            let at_name = !(cs + 1 < ce && t.s(cs + 1) == ":") && !(cs < ce && t.s(cs) == "..");
            if at_name {
                let written: HashSet<&str> = parts
                    .iter()
                    .filter(|(s, e)| s + 1 < *e && t.s(s + 1) == ":")
                    .map(|(s, _)| t.s(*s))
                    .collect();
                for f in s
                    .fields
                    .iter()
                    .filter(|f| !written.contains(f.name.as_str()))
                {
                    out.push(CompletionItem {
                        label: f.name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(f.ty.clone()),
                        documentation: summary_doc(&f.doc),
                        insert_text: Some(format!("{}: ", f.name)),
                        sort_text: Some(format!("0{}", f.name)),
                        ..CompletionItem::default()
                    });
                }
                return out.items;
            }
            // A field's value: the variants first when it holds an enum.
            let expected = s
                .fields
                .iter()
                .find(|f| f.name == t.s(cs))
                .map(|f| Ty::parse(&f.ty));
            self.complete_expression(&mut out, n, expected.as_ref());
            return out.items;
        }
        if self.at_type_position(n) {
            self.complete_types(&mut out);
            return out.items;
        }
        // After `==`, `!=` or `=`: the left side's type ranks its values.
        let expected = if matches!(prev, "==" | "!=" | "=") && n >= 2 {
            let chain = t.chain_start(n - 1);
            let scope = self.scope(chain);
            let infer = Infer::new(t, self.summary, &scope);
            Some(infer.expr(chain, n - 1))
        } else {
            None
        };
        if t.open_brackets(n).is_empty() {
            self.complete_items(&mut out);
        } else {
            self.complete_expression(&mut out, n, expected.as_ref());
        }
        out.items
    }

    /// Whether the first `n` tokens end where a type is written: after the
    /// `:` of a parameter, a `let` or a struct field, after `->`, or inside
    /// a tuple type.
    fn at_type_position(&self, n: usize) -> bool {
        let t = &self.toks;
        if n == 0 {
            return false;
        }
        match t.s(n - 1) {
            "->" => true,
            ":" => {
                if n >= 3 && t.s(n - 3) == "let" {
                    return true;
                }
                let open = t.open_brackets(n);
                let Some(&inner) = open.last() else {
                    return false;
                };
                match t.s(inner) {
                    // A parameter list: `fn name(` or `fn(`.
                    "(" => {
                        (inner >= 1 && t.s(inner - 1) == "fn")
                            || (inner >= 2 && t.s(inner - 2) == "fn")
                            || (inner >= 1 && t.s(inner - 1) == ">")
                    }
                    // A struct declaration's fields.
                    "{" => inner >= 2 && t.s(inner - 2) == "struct",
                    _ => false,
                }
            }
            "(" | "," => {
                // Inside a tuple type: `(A, ` after a `:`.
                let open = t.open_brackets(n);
                open.last()
                    .is_some_and(|&p| t.s(p) == "(" && p > 0 && matches!(t.s(p - 1), ":" | "->"))
            }
            "<" => n >= 2 && t.is_ident(n - 2) && self.is_type_name(t.s(n - 2)),
            _ => false,
        }
    }

    fn is_type_name(&self, name: &str) -> bool {
        matches!(
            self.summary.lookup(name),
            Some(ScopeKind::Struct(_) | ScopeKind::Enum(_))
        )
    }

    fn complete_attribute_names(&self, out: &mut Items, qualifier: &str) {
        let (target, _) = self.decorated();
        let prefix = if qualifier.is_empty() {
            String::new()
        } else {
            format!("{qualifier}::")
        };
        for entry in &self.summary.scope {
            let ScopeKind::Attribute(id) = entry.kind else {
                continue;
            };
            let Some(label) = entry.label.strip_prefix(&prefix) else {
                continue;
            };
            if label.contains("::") && !prefix.is_empty() {
                continue;
            }
            let a = &self.summary.attributes[id];
            if target.is_some_and(|t| t != a.target) {
                continue;
            }
            let insert = if a.params.iter().any(AttrParam::required) {
                let required: Vec<String> = a
                    .params
                    .iter()
                    .filter(|p| p.required())
                    .enumerate()
                    .map(|(i, p)| format!("{}: ${{{}}}", p.name, i + 1))
                    .collect();
                Some(format!("{label}({})", required.join(", ")))
            } else {
                None
            };
            out.push(CompletionItem {
                label: label.to_owned(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some(a.header()),
                documentation: summary_doc(&a.doc),
                insert_text_format: insert.as_ref().map(|_| InsertTextFormat::SNIPPET),
                insert_text: insert,
                ..CompletionItem::default()
            });
        }
    }

    fn complete_path(&self, out: &mut Items, qualifier: &str) {
        let last = qualifier.rsplit("::").next().unwrap_or(qualifier);
        if let Some(e) = self.enum_for(last) {
            for v in &e.variants {
                out.push(variant_item(e, v, &v.name, "0"));
            }
            for m in self.summary.methods_of(&e.name) {
                if !m.params.first().is_some_and(|p| p.name == "self") {
                    out.push(function_item(m, m.label(), "1"));
                }
            }
            return;
        }
        if let Some(s) = self.struct_for(last) {
            for m in self.summary.methods_of(&s.name) {
                if !m.params.first().is_some_and(|p| p.name == "self") {
                    out.push(function_item(m, m.label(), "1"));
                }
            }
        }
        let prefix = format!("{qualifier}::");
        for entry in &self.summary.scope {
            if let Some(rest) = entry.label.strip_prefix(&prefix)
                && !rest.contains("::")
            {
                self.push_scope_entry(out, rest, entry.kind, "0");
            }
        }
    }

    fn enum_for(&self, name: &str) -> Option<&'a EnumSymbol> {
        match self.summary.lookup(name) {
            Some(ScopeKind::Enum(id)) => self.summary.enums.get(id),
            _ => self.summary.enum_named(name),
        }
    }

    fn struct_for(&self, name: &str) -> Option<&'a StructSymbol> {
        match self.summary.lookup(name) {
            Some(ScopeKind::Struct(Some(id))) => self.summary.structs.get(id),
            _ => self.summary.struct_named(name),
        }
    }

    fn complete_members(&self, out: &mut Items, ty: &Ty) {
        match ty {
            Ty::Named(name) if self.summary.struct_named(name).is_some() => {
                let s = self.summary.struct_named(name).expect("checked above");
                for f in &s.fields {
                    out.push(CompletionItem {
                        label: f.name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(f.ty.clone()),
                        documentation: summary_doc(&f.doc),
                        sort_text: Some(format!("0{}", f.name)),
                        ..CompletionItem::default()
                    });
                }
                for m in self.summary.methods_of(name) {
                    if m.params.first().is_some_and(|p| p.name == "self") {
                        out.push(function_item(m, m.label(), "1"));
                    }
                }
            }
            Ty::Named(name) if self.summary.enum_named(name).is_some() => {
                for m in self.summary.methods_of(name) {
                    if m.params.first().is_some_and(|p| p.name == "self") {
                        out.push(function_item(m, m.label(), "1"));
                    }
                }
            }
            Ty::Tuple(members) => {
                for (i, m) in members.iter().enumerate() {
                    out.push(CompletionItem {
                        label: i.to_string(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(m.render()),
                        ..CompletionItem::default()
                    });
                }
            }
            _ => {
                let receiver = match ty {
                    Ty::Named(n) => Some(n.as_str()),
                    Ty::List(_) => Some("[]"),
                    Ty::Map(..) => Some("{"),
                    _ => None,
                };
                for (name, d) in builtins::BUILTIN_METHODS {
                    if receiver.is_none_or(|r| builtin_applies(d, r)) {
                        out.push(builtin_item(name, CompletionItemKind::METHOD, d));
                    }
                }
            }
        }
    }

    fn complete_attribute_args(&self, out: &mut Items, args: &AttrArgs<'_>) {
        let Some(param) = args.param else {
            // A parameter's name.
            for p in &args.attr.params {
                if args.written.contains(&p.name) {
                    continue;
                }
                let value = if p.list { "[$1]" } else { "$1" };
                out.push(CompletionItem {
                    label: p.name.clone(),
                    kind: Some(CompletionItemKind::PROPERTY),
                    detail: Some(p.render()),
                    insert_text: Some(format!("{}: {value}", p.name)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    sort_text: Some(format!("{}{}", u8::from(!p.required()), p.name)),
                    ..CompletionItem::default()
                });
            }
            return;
        };
        let listed: HashSet<&str> = args.listed.iter().map(String::as_str).collect();
        let (_, decorated) = self.decorated();
        match &param.kind {
            AttrParamKind::Enum(name) => {
                if let Some(e) = self.enum_for(name) {
                    for v in &e.variants {
                        let label = format!("{}::{}", e.name, v.name);
                        if !listed.contains(label.as_str()) {
                            out.push(variant_item(e, v, &label, "0"));
                        }
                    }
                }
            }
            AttrParamKind::AnyEnum => {
                // The enums the declaring host marks with an attribute; every
                // enum the buffer declares when it marks none.
                let marked: Vec<&EnumSymbol> = self
                    .summary
                    .enums
                    .iter()
                    .filter(|e| {
                        self.summary
                            .uses_on(&UseOn::Enum(e.name.clone()))
                            .any(|u| u.namespace == args.attr.namespace)
                    })
                    .collect();
                let enums = if marked.is_empty() {
                    self.summary.own_enums().collect()
                } else {
                    marked
                };
                for e in enums {
                    for v in e.variants.iter().filter(|v| v.payload.is_empty()) {
                        let label = format!("{}::{}", e.name, v.name);
                        if !listed.contains(label.as_str()) {
                            out.push(variant_item(e, v, &label, "0"));
                        }
                    }
                }
            }
            AttrParamKind::Fn => {
                let peers_first = self.peers_first(args.attr, &param.name);
                for f in self.summary.own_functions() {
                    if f.is_const
                        || Some(f.name.as_str()) == decorated
                        || listed.contains(f.name.as_str())
                    {
                        continue;
                    }
                    let peer = self.carries(&f.name, args.attr);
                    let rank = if peer == peers_first { "0" } else { "1" };
                    let mut item = function_item(f, &f.name, rank);
                    item.insert_text = None;
                    item.insert_text_format = None;
                    if peer {
                        item.detail = Some(format!("@{} {}", args.attr.name, f.header()));
                    }
                    out.push(item);
                }
            }
            AttrParamKind::Type => {
                for (i, s) in self.summary.structs.iter().enumerate() {
                    if s.is_tuple()
                        || s.name.contains('<')
                        || !self.summary.is_user_file(s.src_file)
                        || listed.contains(s.name.as_str())
                        || self.summary.lookup(&s.name) != Some(ScopeKind::Struct(Some(i)))
                    {
                        continue;
                    }
                    if Some(s.name.as_str()) == decorated {
                        continue;
                    }
                    // The structs carrying the attribute being written first:
                    // the kind of item the argument most likely names.
                    let peer = self
                        .summary
                        .uses_on(&UseOn::Struct(s.name.clone()))
                        .any(|u| u.namespace == args.attr.namespace && u.name == args.attr.name);
                    let rank = if peer { "0" } else { "1" };
                    out.push(struct_item(s, rank));
                }
            }
            AttrParamKind::Bool => {
                for b in ["true", "false"] {
                    out.push(CompletionItem {
                        label: b.to_owned(),
                        kind: Some(CompletionItemKind::VALUE),
                        ..CompletionItem::default()
                    });
                }
            }
            AttrParamKind::Literal => {}
        }
    }

    /// Whether the function named `name` carries `attr`.
    fn carries(&self, name: &str, attr: &AttributeSymbol) -> bool {
        self.summary
            .uses_on(&UseOn::Fn(name.to_owned()))
            .any(|u| u.namespace == attr.namespace && u.name == attr.name)
    }

    /// Whether a `fn` argument of `attr` named `param` should offer the
    /// functions carrying `attr` itself first. The program's own uses decide:
    /// a parameter it fills with such functions (an ordering against other
    /// systems) ranks them first, one it fills with others (a predicate)
    /// ranks them last. With no use to go by, they come first.
    fn peers_first(&self, attr: &AttributeSymbol, param: &str) -> bool {
        let mut peers = 0;
        let mut others = 0;
        for u in &self.summary.attribute_uses {
            if u.namespace != attr.namespace || u.name != attr.name {
                continue;
            }
            for (p, fns) in &u.fn_args {
                if p != param {
                    continue;
                }
                for f in fns {
                    if self.carries(f, attr) {
                        peers += 1;
                    } else {
                        others += 1;
                    }
                }
            }
        }
        peers >= others
    }

    fn complete_types(&self, out: &mut Items) {
        for ty in BUILTIN_TYPES {
            out.push(CompletionItem {
                label: (*ty).to_owned(),
                kind: Some(CompletionItemKind::KEYWORD),
                sort_text: Some(format!("2{ty}")),
                ..CompletionItem::default()
            });
        }
        for entry in &self.summary.scope {
            match entry.kind {
                ScopeKind::Struct(_) | ScopeKind::Enum(_) => {
                    let own = match entry.kind {
                        ScopeKind::Struct(Some(id)) => self
                            .summary
                            .structs
                            .get(id)
                            .is_some_and(|s| s.src_file == Some(0)),
                        ScopeKind::Enum(id) => self
                            .summary
                            .enums
                            .get(id)
                            .is_some_and(|e| e.src_file == Some(0)),
                        _ => false,
                    };
                    self.push_scope_entry(
                        out,
                        &entry.label,
                        entry.kind,
                        if own { "0" } else { "1" },
                    );
                }
                _ => {}
            }
        }
    }

    /// Top level: declarations, and a skeleton for each attribute a `host`
    /// block declares.
    fn complete_items(&self, out: &mut Items) {
        for kw in builtins::ITEM_KEYWORDS {
            out.push(keyword_item(kw));
        }
        for entry in &self.summary.scope {
            let ScopeKind::Attribute(id) = entry.kind else {
                continue;
            };
            let a = &self.summary.attributes[id];
            let args: Vec<String> = a
                .params
                .iter()
                .filter(|p| p.required())
                .enumerate()
                .map(|(i, p)| format!("{}: ${{{}}}", p.name, i + 2))
                .collect();
            let head = if args.is_empty() {
                format!("@{}", entry.label)
            } else {
                format!("@{}({})", entry.label, args.join(", "))
            };
            let body = match a.target {
                "fn" => "fn ${1:name}($0) {\n}".to_owned(),
                "struct" => "struct ${1:Name} {\n\t$0\n}".to_owned(),
                _ => "enum ${1:Name} { $0 }".to_owned(),
            };
            out.push(CompletionItem {
                label: format!("@{} {}", entry.label, a.target),
                kind: Some(CompletionItemKind::SNIPPET),
                detail: Some(a.header()),
                documentation: summary_doc(&a.doc),
                insert_text: Some(format!("{head}\n{body}")),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                filter_text: Some(entry.label.clone()),
                sort_text: Some(format!("0{}", entry.label)),
                ..CompletionItem::default()
            });
        }
    }

    /// Inside a body: the bindings in scope, the program's names, builtins and
    /// keywords; the values of `expected` first when it is an enum.
    fn complete_expression(&self, out: &mut Items, n: usize, expected: Option<&Ty>) {
        if let Some(Ty::Named(name)) = expected
            && let Some(e) = self.summary.enum_named(name)
        {
            for v in &e.variants {
                out.push(variant_item(e, v, &format!("{}::{}", e.name, v.name), "0"));
            }
        }
        let scope = self.scope(n);
        let infer = Infer::new(&self.toks, self.summary, &scope);
        let mut seen = HashSet::new();
        for b in scope.bindings.iter().rev() {
            // A `let` does not see its own name in its value.
            let own_value = matches!(b.source, Source::Expr(s, e) | Source::Member(s, e, _) if s <= n && n <= e);
            if b.name == "_" || own_value || !seen.insert(b.name.clone()) {
                continue;
            }
            let ty = infer.binding(b);
            out.push(CompletionItem {
                label: b.name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(ty.render()),
                sort_text: Some(format!("1{}", b.name)),
                ..CompletionItem::default()
            });
        }
        for entry in &self.summary.scope {
            if matches!(entry.kind, ScopeKind::Attribute(_)) {
                continue;
            }
            let own = match entry.kind {
                ScopeKind::Function(id) => self
                    .summary
                    .functions
                    .get(id)
                    .is_some_and(|f| f.src_file == 0),
                _ => false,
            };
            self.push_scope_entry(out, &entry.label, entry.kind, if own { "2" } else { "3" });
        }
        for (name, d) in builtins::BUILTIN_FUNCTIONS {
            let mut item = builtin_item(name, CompletionItemKind::FUNCTION, d);
            item.sort_text = Some(format!("4{name}"));
            out.push(item);
        }
        for kw in builtins::STATEMENT_KEYWORDS {
            out.push(keyword_item(kw));
        }
    }

    fn push_scope_entry(&self, out: &mut Items, label: &str, kind: ScopeKind, rank: &str) {
        match kind {
            ScopeKind::Function(id) => {
                if let Some(f) = self.summary.functions.get(id) {
                    out.push(function_item(f, label, rank));
                }
            }
            ScopeKind::Struct(Some(id)) => {
                if let Some(s) = self.summary.structs.get(id) {
                    let mut item = struct_item(s, rank);
                    item.label = label.to_owned();
                    out.push(item);
                }
            }
            ScopeKind::Struct(None) => out.push(CompletionItem {
                label: label.to_owned(),
                kind: Some(CompletionItemKind::STRUCT),
                detail: Some(format!("struct {label}<..>")),
                sort_text: Some(format!("{rank}{label}")),
                ..CompletionItem::default()
            }),
            ScopeKind::Enum(id) => {
                if let Some(e) = self.summary.enums.get(id) {
                    out.push(CompletionItem {
                        label: label.to_owned(),
                        kind: Some(CompletionItemKind::ENUM),
                        detail: Some(enum_header(e)),
                        documentation: summary_doc(&e.doc),
                        sort_text: Some(format!("{rank}{label}")),
                        ..CompletionItem::default()
                    });
                }
            }
            ScopeKind::Attribute(_) => {}
        }
    }

    // ---- Resolution ----

    /// The declaration the identifier under the cursor names.
    #[must_use]
    pub fn symbol(&self) -> Option<Symbol<'a>> {
        let t = &self.toks;
        let i = t.at(self.offset)?;
        if !t.is_ident(i) {
            return None;
        }
        let name = t.s(i);
        let prev = if i > 0 { t.s(i - 1) } else { "" };
        let next = t.s(i + 1);
        let s = self.summary;

        // `@name`, `@module::name`.
        let (path, path_start) = self.path_before(i);
        if prev == "@" || (path_start > 0 && t.s(path_start - 1) == "@" && !path.is_empty()) {
            let label = if path.is_empty() {
                name.to_owned()
            } else {
                format!("{path}::{name}")
            };
            return s.attribute(&label).map(Symbol::Attribute);
        }
        // A parameter name inside an attribute's arguments.
        if next == ":"
            && let Some(args) = self.attr_args(i)
            && let Some(p) = args.attr.params.iter().find(|p| p.name == name)
        {
            return Some(Symbol::AttrParam(args.attr, p));
        }
        // `a.name`
        if prev == "." {
            let chain = t.chain_start(i - 1);
            let scope = self.scope(chain);
            let infer = Infer::new(t, s, &scope);
            let ty = infer.expr(chain, i - 1);
            if let Ty::Named(tn) = &ty {
                if let Some(st) = s.struct_named(tn)
                    && let Some(f) = st.fields.iter().find(|f| f.name == name)
                {
                    return Some(Symbol::Field(st, f));
                }
                if let Some(m) = s.method(tn, name) {
                    return Some(Symbol::Function(m));
                }
            }
            return builtins::BUILTIN_METHODS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(n, d)| Symbol::Builtin(n, d));
        }
        // `Owner::name`
        if prev == "::" {
            let owner = path.rsplit("::").next().unwrap_or(&path);
            if let Some(e) = self.enum_for(owner)
                && let Some(v) = e.variants.iter().find(|v| v.name == name)
            {
                return Some(Symbol::Variant(e, v));
            }
            if let Some(ty) = self
                .enum_for(owner)
                .map(|e| e.name.clone())
                .or_else(|| self.struct_for(owner).map(|s| s.name.clone()))
                && let Some(m) = s.method(&ty, name)
            {
                return Some(Symbol::Function(m));
            }
            return self.named(&format!("{path}::{name}"));
        }
        // A field name in a struct literal or a struct declaration.
        if next == ":" {
            if let Some((st, _)) = self.struct_literal(i)
                && let Some(f) = st.fields.iter().find(|f| f.name == name)
            {
                return Some(Symbol::Field(st, f));
            }
            let open = t.open_brackets(i);
            if let Some(&brace) = open.last()
                && t.s(brace) == "{"
                && brace >= 2
                && t.s(brace - 2) == "struct"
                && let Some(st) = self.struct_for(t.s(brace - 1))
                && let Some(f) = st.fields.iter().find(|f| f.name == name)
            {
                return Some(Symbol::Field(st, f));
            }
        }
        // A method's name in its `impl` block.
        if prev == "fn" {
            let scope = self.scope(i);
            if let Some(ty) = &scope.impl_type
                && let Some(m) = s.method(ty, name)
            {
                return Some(Symbol::Function(m));
            }
        }
        // A parameter or a local.
        if !matches!(prev, "fn" | "struct" | "enum") {
            let scope = self.scope(i);
            if let Some(b) = scope.get(name) {
                let infer = Infer::new(t, s, &scope);
                return Some(Symbol::Local {
                    name: name.to_owned(),
                    ty: infer.binding(b),
                    at: b.at,
                });
            }
            // The declaration of a parameter or a binding itself: its scope
            // opens at the next brace, or ends its statement.
            let end = (i..t.toks.len())
                .find(|&k| matches!(t.s(k), "{" | ";"))
                .map_or(t.toks.len(), |k| k + 1);
            let scope_after = self.scope(end);
            if let Some(b) = scope_after
                .bindings
                .iter()
                .find(|b| b.at == t.toks[i].start)
            {
                let infer = Infer::new(t, s, &scope_after);
                return Some(Symbol::Local {
                    name: name.to_owned(),
                    ty: infer.binding(b),
                    at: b.at,
                });
            }
        }
        if let Some(sym) = self.named(name) {
            return Some(sym);
        }
        builtins::BUILTIN_FUNCTIONS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(n, d)| Symbol::Builtin(n, d))
    }

    /// The program's declaration the buffer writes as `label`.
    fn named(&self, label: &str) -> Option<Symbol<'a>> {
        let s = self.summary;
        match s.lookup(label)? {
            ScopeKind::Function(id) => s.functions.get(id).map(Symbol::Function),
            ScopeKind::Struct(Some(id)) => s.structs.get(id).map(Symbol::Struct),
            ScopeKind::Struct(None) => {
                // A generic struct: any instantiation stands for it.
                let base = label.rsplit("::").next().unwrap_or(label);
                s.structs
                    .iter()
                    .find(|st| st.name.split('<').next() == Some(base) && st.name.contains('<'))
                    .map(Symbol::Struct)
            }
            ScopeKind::Enum(id) => s.enums.get(id).map(Symbol::Enum),
            ScopeKind::Attribute(id) => s.attributes.get(id).map(Symbol::Attribute),
        }
    }

    /// The hover text for the identifier under the cursor, ending in a link
    /// to the item's documentation when its package says where that lives.
    #[must_use]
    pub fn hover(&self) -> Option<String> {
        let sym = self.symbol()?;
        let mut text = hover_markdown(&sym);
        if let Some(link) = self.docs_link(&sym) {
            text.push_str(&format!("\n\n[Open docs]({link})"));
        }
        Some(text)
    }

    /// The link to a declaration's documentation: the `docs` template of the
    /// package that declares it, filled with the item's name as a reader
    /// writes it (`Type`, `Type.method`, `Type.field`, `Enum::Variant`,
    /// `@attribute`, `function`).
    fn docs_link(&self, sym: &Symbol<'_>) -> Option<String> {
        let (file, name) = match sym {
            Symbol::Function(f) => (
                Some(f.src_file),
                match &f.method {
                    Some((ty, name)) => format!("{}.{name}", base_name(ty)),
                    None => f.name.clone(),
                },
            ),
            Symbol::Struct(s) => (s.src_file, base_name(&s.name).to_owned()),
            Symbol::Enum(e) => (e.src_file, base_name(&e.name).to_owned()),
            Symbol::Variant(e, v) => (e.src_file, format!("{}::{}", base_name(&e.name), v.name)),
            Symbol::Field(s, f) => (s.src_file, format!("{}.{}", base_name(&s.name), f.name)),
            Symbol::Attribute(a) | Symbol::AttrParam(a, _) => {
                (Some(a.src_file), format!("@{}", a.name))
            }
            Symbol::Local { .. } | Symbol::Builtin(..) => return None,
        };
        let template = self
            .summary
            .sources
            .get(usize::from(file?))?
            .docs
            .as_deref()?;
        Some(candela::docs_link(template, &name))
    }

    /// Where the declaration under the cursor is written. A local's is in the
    /// current buffer, at a byte offset; everything else is in the program's
    /// files.
    #[must_use]
    pub fn definition(&self) -> Option<Result<Target, usize>> {
        Some(match self.symbol()? {
            Symbol::Function(f) => Ok(Target {
                src_file: f.src_file,
                span: f.name_span,
            }),
            Symbol::Struct(st) => Ok(Target {
                src_file: st.src_file?,
                span: st.name_span,
            }),
            Symbol::Enum(e) => Ok(Target {
                src_file: e.src_file?,
                span: e.name_span,
            }),
            Symbol::Variant(e, v) => Ok(Target {
                src_file: e.src_file?,
                span: v.span,
            }),
            Symbol::Field(st, f) => Ok(Target {
                src_file: st.src_file?,
                span: f.span,
            }),
            Symbol::Attribute(a) | Symbol::AttrParam(a, _) => Ok(Target {
                src_file: a.src_file,
                span: a.span,
            }),
            Symbol::Local { at, .. } => Err(at),
            Symbol::Builtin(..) => return None,
        })
    }

    // ---- Signature help ----

    /// The signature of the call or attribute whose parentheses hold the
    /// cursor.
    #[must_use]
    pub fn signature_help(&self) -> Option<SignatureHelp> {
        let t = &self.toks;
        if t.in_comment_or_string(self.offset) {
            return None;
        }
        let n = t.count_before(self.offset);
        if let Some(args) = self.attr_args(n) {
            let a = args.attr;
            let params: Vec<String> = a.params.iter().map(AttrParam::render).collect();
            let active = args
                .param
                .and_then(|p| a.params.iter().position(|q| q.name == p.name))
                .unwrap_or_else(|| {
                    // The next parameter not written yet.
                    a.params
                        .iter()
                        .position(|p| !args.written.contains(&p.name))
                        .unwrap_or(args.index)
                });
            return Some(signature(
                &format!("@{}", a.name),
                &params,
                "",
                &a.doc,
                active,
            ));
        }
        let open = t.open_brackets(n);
        let &paren = open.iter().rev().find(|&&o| t.s(o) == "(")?;
        if paren == 0 || !t.is_ident(paren - 1) {
            return None;
        }
        // Only a call's own parentheses: nothing but brackets inside it.
        if open.iter().any(|&o| o > paren && t.s(o) == "{") {
            return None;
        }
        let active = t.split_commas(paren + 1, n).len() - 1;
        let name = t.s(paren - 1);
        let callee = paren - 1;
        let f = if callee > 0 && t.s(callee - 1) == "." {
            let chain = t.chain_start(callee - 1);
            let scope = self.scope(chain);
            let infer = Infer::new(t, self.summary, &scope);
            match infer.expr(chain, callee - 1) {
                Ty::Named(ty) => self.summary.method(&ty, name),
                _ => None,
            }
        } else if callee > 0 && t.s(callee - 1) == "fn" {
            return None;
        } else {
            let (path, _) = self.path_before(callee);
            let label = if path.is_empty() {
                name.to_owned()
            } else {
                format!("{path}::{name}")
            };
            if let Some(e) = self.enum_for(&path)
                && let Some(v) = e.variants.iter().find(|v| v.name == name)
            {
                return Some(signature(
                    &format!("{}::{}", e.name, v.name),
                    &v.payload,
                    "",
                    &v.doc,
                    active,
                ));
            }
            self.summary.function(&label).or_else(|| {
                let ty = self
                    .enum_for(&path)
                    .map(|e| e.name.clone())
                    .or_else(|| self.struct_for(&path).map(|s| s.name.clone()))?;
                self.summary.method(&ty, name)
            })
        };
        if let Some(f) = f {
            let params: Vec<String> = f
                .call_params()
                .map(crate::analysis::Param::render)
                .collect();
            let ret = f
                .ret
                .as_ref()
                .map(|r| format!(" -> {r}"))
                .unwrap_or_default();
            return Some(signature(
                &format!("fn {}", f.label()),
                &params,
                &ret,
                &f.doc,
                active,
            ));
        }
        let (_, d) = builtins::BUILTIN_FUNCTIONS
            .iter()
            .chain(builtins::BUILTIN_METHODS)
            .find(|(n, _)| *n == name)?;
        let first = d.lines().next().unwrap_or_default();
        Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: first.to_owned(),
                documentation: doc(d),
                parameters: None,
                active_parameter: u32::try_from(active).ok(),
            }],
            active_signature: Some(0),
            active_parameter: u32::try_from(active).ok(),
        })
    }
}

/// Completion items without duplicate labels.
#[derive(Default)]
struct Items {
    items: Vec<CompletionItem>,
    seen: HashSet<String>,
}

impl Items {
    fn push(&mut self, item: CompletionItem) {
        if self.seen.insert(item.label.clone()) {
            self.items.push(item);
        }
    }
}

/// A completion item's documentation: the summary line of a doc comment.
fn summary_doc(text: &str) -> Option<Documentation> {
    doc(&summary(text))
}

/// A generic type's name without its arguments: `Lookup` for `Lookup<T>`.
fn base_name(name: &str) -> &str {
    name.split('<').next().unwrap_or(name)
}

fn doc(text: &str) -> Option<Documentation> {
    (!text.is_empty()).then(|| {
        Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: text.to_owned(),
        })
    })
}

fn keyword_item(kw: &str) -> CompletionItem {
    CompletionItem {
        label: kw.to_owned(),
        kind: Some(CompletionItemKind::KEYWORD),
        sort_text: Some(format!("5{kw}")),
        ..CompletionItem::default()
    }
}

fn builtin_item(name: &str, kind: CompletionItemKind, d: &str) -> CompletionItem {
    CompletionItem {
        label: name.to_owned(),
        kind: Some(kind),
        detail: Some(d.lines().next().unwrap_or_default().to_owned()),
        documentation: doc(d),
        ..CompletionItem::default()
    }
}

fn function_item(f: &FunctionSymbol, label: &str, rank: &str) -> CompletionItem {
    let (insert, format) = if f.is_const {
        (None, None)
    } else if f.call_params().next().is_some() {
        (
            Some(format!("{label}($0)")),
            Some(InsertTextFormat::SNIPPET),
        )
    } else {
        (Some(format!("{label}()")), None)
    };
    CompletionItem {
        label: label.to_owned(),
        kind: Some(if f.is_const {
            CompletionItemKind::CONSTANT
        } else if f.method.is_some() {
            CompletionItemKind::METHOD
        } else {
            CompletionItemKind::FUNCTION
        }),
        detail: Some(f.header()),
        documentation: summary_doc(&f.doc),
        insert_text: insert,
        insert_text_format: format,
        sort_text: Some(format!("{rank}{label}")),
        ..CompletionItem::default()
    }
}

fn struct_item(s: &StructSymbol, rank: &str) -> CompletionItem {
    CompletionItem {
        label: s.name.clone(),
        kind: Some(CompletionItemKind::STRUCT),
        detail: Some(format!("struct {}", s.name)),
        documentation: summary_doc(&s.doc),
        sort_text: Some(format!("{rank}{}", s.name)),
        ..CompletionItem::default()
    }
}

fn variant_item(e: &EnumSymbol, v: &VariantSymbol, label: &str, rank: &str) -> CompletionItem {
    let detail = if v.payload.is_empty() {
        format!("{}::{}", e.name, v.name)
    } else {
        format!("{}::{}({})", e.name, v.name, v.payload.join(", "))
    };
    CompletionItem {
        label: label.to_owned(),
        kind: Some(CompletionItemKind::ENUM_MEMBER),
        detail: Some(detail),
        documentation: summary_doc(if v.doc.is_empty() { &e.doc } else { &v.doc }),
        // Declaration order.
        sort_text: Some(format!(
            "{rank}{:04}",
            e.variants
                .iter()
                .position(|w| w.name == v.name)
                .unwrap_or(0)
        )),
        ..CompletionItem::default()
    }
}

fn enum_header(e: &EnumSymbol) -> String {
    let variants: Vec<String> = e
        .variants
        .iter()
        .map(|v| {
            if v.payload.is_empty() {
                v.name.clone()
            } else {
                format!("{}({})", v.name, v.payload.join(", "))
            }
        })
        .collect();
    format!("enum {} {{ {} }}", e.name, variants.join(", "))
}

/// Whether a builtin method documented as `<receiver>.name` applies to a
/// value whose type renders as `receiver`.
fn builtin_applies(doc: &str, receiver: &str) -> bool {
    let Some(rest) = doc.strip_prefix('<') else {
        return true;
    };
    let Some((types, _)) = rest.split_once(">.") else {
        return true;
    };
    types.split('|').any(|t| {
        let t = t.trim();
        t == receiver || (receiver == "[]" && t.ends_with("[]")) || t == "T"
    })
}

fn signature(name: &str, params: &[String], ret: &str, d: &str, active: usize) -> SignatureHelp {
    let mut label = format!("{name}(");
    let mut infos = Vec::new();
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            label.push_str(", ");
        }
        let start = u32::try_from(label.encode_utf16().count()).unwrap_or(0);
        label.push_str(p);
        let end = u32::try_from(label.encode_utf16().count()).unwrap_or(0);
        infos.push(ParameterInformation {
            label: ParameterLabel::LabelOffsets([start, end]),
            documentation: None,
        });
    }
    label.push(')');
    label.push_str(ret);
    let active = u32::try_from(active).ok();
    SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: doc(d),
            parameters: Some(infos),
            active_parameter: active,
        }],
        active_signature: Some(0),
        active_parameter: active,
    }
}

/// The hover text for a declaration: its signature in a code block, then its
/// comment.
#[must_use]
pub fn hover_markdown(sym: &Symbol<'_>) -> String {
    let (code, d) = match sym {
        Symbol::Function(f) => {
            let mut code = f.header();
            if let Some((ty, _)) = &f.method {
                code = format!("impl {ty}\n{code}");
            }
            // A parameter without a type is specialised per call site.
            let mut d = f.doc.clone();
            if f.call_params().any(|p| p.ty.is_none()) && !f.signatures.is_empty() {
                let lines: Vec<String> = f.signatures.iter().map(|s| format!("- `{s}`")).collect();
                if !d.is_empty() {
                    d.push_str("\n\n");
                }
                d.push_str(&format!("Called as:\n{}", lines.join("\n")));
            }
            (code, d)
        }
        Symbol::Struct(s) => {
            let fields: Vec<String> = s
                .fields
                .iter()
                .map(|f| match &f.default {
                    Some(d) => format!("    {}: {} = {d},", f.name, f.ty),
                    None => format!("    {}: {},", f.name, f.ty),
                })
                .collect();
            let code = if fields.is_empty() {
                format!("struct {} {{}}", s.name)
            } else {
                format!("struct {} {{\n{}\n}}", s.name, fields.join("\n"))
            };
            let members = s.fields.iter().map(|f| (f.name.as_str(), f.doc.as_str()));
            (code, with_members(&s.doc, "Fields", members))
        }
        Symbol::Enum(e) => {
            let members = e.variants.iter().map(|v| (v.name.as_str(), v.doc.as_str()));
            (enum_header(e), with_members(&e.doc, "Variants", members))
        }
        Symbol::Variant(e, v) => {
            let code = if v.payload.is_empty() {
                format!("{}::{}", e.name, v.name)
            } else {
                format!("{}::{}({})", e.name, v.name, v.payload.join(", "))
            };
            (
                code,
                if v.doc.is_empty() {
                    e.doc.clone()
                } else {
                    v.doc.clone()
                },
            )
        }
        Symbol::Field(s, f) => (
            match &f.default {
                Some(d) => format!("{}.{}: {} = {d}", s.name, f.name, f.ty),
                None => format!("{}.{}: {}", s.name, f.name, f.ty),
            },
            f.doc.clone(),
        ),
        Symbol::Attribute(a) => (a.header(), a.doc.clone()),
        Symbol::AttrParam(a, p) => (format!("@{}({})", a.name, p.render()), a.doc.clone()),
        Symbol::Local { name, ty, .. } => (format!("{name}: {}", ty.render()), String::new()),
        Symbol::Builtin(name, d) => return format!("**{name}** (builtin)\n\n{d}"),
    };
    if d.is_empty() {
        format!("```candela\n{code}\n```")
    } else {
        format!("```candela\n{code}\n```\n\n{d}")
    }
}

/// A struct's or an enum's doc comment followed by a list of its members
/// with the summary of each, when any member has a doc comment.
fn with_members<'m>(
    doc: &str,
    heading: &str,
    members: impl Iterator<Item = (&'m str, &'m str)>,
) -> String {
    let members: Vec<(&str, &str)> = members.collect();
    if members.iter().all(|(_, d)| d.is_empty()) {
        return doc.to_owned();
    }
    let lines: Vec<String> = members
        .iter()
        .map(|(name, d)| {
            let line = summary(d);
            if line.is_empty() {
                format!("- `{name}`")
            } else {
                format!("- `{name}`: {line}")
            }
        })
        .collect();
    let list = format!("**{heading}**\n\n{}", lines.join("\n"));
    if doc.is_empty() {
        list
    } else {
        format!("{doc}\n\n{list}")
    }
}

/// Warnings about attributes the compiler accepts but a host is unlikely to
/// mean: the same attribute written twice on one item, and a variant passed
/// for an `enum` parameter from an enum the declaring host marks with none of
/// its attributes while it marks others. Each is a byte range in `text`, a
/// message and a code.
#[must_use]
pub fn attribute_warnings(
    text: &str,
    summary: &ProgramSummary,
) -> Vec<(usize, usize, String, &'static str)> {
    let t = Tokens::new(text);
    let mut out = Vec::new();
    // The attributes of the run being read, cleared at each declaration.
    let mut run: Vec<String> = Vec::new();
    let mut i = 0;
    while i < t.toks.len() {
        match t.s(i) {
            "fn" | "struct" | "enum" => run.clear(),
            "@" if t.is_ident(i + 1) => {
                let mut j = i + 1;
                let mut name = t.s(j).to_owned();
                while t.s(j + 1) == "::" && t.is_ident(j + 2) {
                    name = format!("{name}::{}", t.s(j + 2));
                    j += 2;
                }
                let start = t.toks[i].start;
                let end = t.toks[j].end;
                if let Some(attr) = summary.attribute(&name) {
                    if run.contains(&name) {
                        out.push((
                            start,
                            end,
                            format!("@{name} is already written on this item"),
                            "attribute_repeated",
                        ));
                    }
                    run.push(name.clone());
                    if t.s(j + 1) == "(" {
                        let close = t.closing(j + 1);
                        unmarked_variants(&t, summary, attr, j + 2, close, &mut out);
                        j = close;
                    }
                }
                i = j + 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn unmarked_variants(
    t: &Tokens<'_>,
    summary: &ProgramSummary,
    attr: &AttributeSymbol,
    start: usize,
    end: usize,
    out: &mut Vec<(usize, usize, String, &'static str)>,
) {
    let marked = |e: &EnumSymbol| {
        summary
            .uses_on(&UseOn::Enum(e.name.clone()))
            .any(|u| u.namespace == attr.namespace)
    };
    if !summary.enums.iter().any(marked) {
        return;
    }
    for (s, e) in t.split_commas(start, end) {
        if !(s + 1 < e && t.s(s + 1) == ":") {
            continue;
        }
        let Some(p) = attr.params.iter().find(|p| p.name == t.s(s)) else {
            continue;
        };
        if p.kind != AttrParamKind::AnyEnum {
            continue;
        }
        for k in s + 2..e {
            if t.s(k + 1) == "::"
                && t.is_ident(k)
                && let Some(en) = summary.enum_named(t.s(k))
                && !marked(en)
            {
                let names: Vec<String> = summary
                    .enums
                    .iter()
                    .filter(|e| marked(e))
                    .map(|e| e.name.clone())
                    .collect();
                out.push((
                    t.toks[k].start,
                    t.toks[(k + 2).min(e - 1)].end,
                    format!(
                        "{} carries no attribute of \"{}\"; {} takes a variant of {}",
                        en.name,
                        attr.namespace,
                        p.name,
                        names.join(" or ")
                    ),
                    "attribute_unmarked_enum",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Ide;
    use crate::analysis::{ProgramSummary, analyze};
    use candela::compiler::imports::ImportResolver;

    /// A host module the way a game engine declares one, and a game using it.
    const GAME: &str = r#"host "ecs" {
    attribute component(requires: type[] = []) on struct;
    attribute resource on struct;
    attribute state on enum;
    attribute system(after: fn[] = [], run_if: fn[] = [], in_state: enum[] = [],
                     on_enter: enum?, schedule: Schedule = Schedule::Update) on fn;
}

enum Schedule { Startup, Update }

/// A point on the plane.
struct Vec2 { x: float = 0.0, y: float = 0.0 }
impl Vec2 {
    /// How far from the origin.
    fn length(self) -> float { return (self.x * self.x + self.y * self.y).sqrt(); }
}
fn vec2(x: float, y: float) -> Vec2 { return Vec2 { x: x, y: y }; }

struct Transform { position: Vec2 = Vec2 { .. }, rotation: float = 0.0 }
struct State<S> { now: S, next: S }
impl State<S> {
    fn set(self, s: S) { self.next = s; }
}

@state
enum Phase { Ready, Playing, Over }
enum Mood { Calm, Angry }

@component struct Bird { vy: float = 0.0 }
@component(requires: [Transform]) struct Pipe { scored: bool = false }
@resource struct Game { score: int = 0 }

fn ready(g: Game) -> bool => g.score == 0;

@system(run_if: [ready])
fn fall(bird: Bird, t: Transform) {
    bird.vy += 1.0;
}

@system(on_enter: Phase::Ready)
fn reset(birds: (Bird, Transform)[], game: Game, state: State<Phase>) {
    for (bird, t) in birds {
        t.position = vec2(0.0, 0.0);
    }
    game.score = 0;
}
"#;

    fn summary() -> ProgramSummary {
        let outcome = analyze(GAME, "game.cdl", &ImportResolver::new());
        assert!(
            outcome.errors.is_empty(),
            "{:?}",
            outcome.errors.first().map(|d| d.message.clone())
        );
        outcome.summary.expect("the game compiles")
    }

    /// `GAME` with `edit` written in place of `anchor`; `$0` in `edit` marks
    /// the cursor. Returns the text and the cursor's offset.
    fn edited(anchor: &str, edit: &str) -> (String, usize) {
        assert!(GAME.contains(anchor), "{anchor}");
        let text = GAME.replacen(anchor, edit, 1);
        let at = text.find("$0").expect("the edit marks the cursor");
        (text.replacen("$0", "", 1), at)
    }

    fn labels(anchor: &str, edit: &str) -> Vec<String> {
        let s = summary();
        let (text, at) = edited(anchor, edit);
        // In the order a client lists them: by sort text, then label.
        let mut items = Ide::new(&text, &s, at).completions();
        items.sort_by(|a, b| {
            let key = |i: &tower_lsp::lsp_types::CompletionItem| {
                (
                    i.sort_text.clone().unwrap_or_else(|| i.label.clone()),
                    i.label.clone(),
                )
            };
            key(a).cmp(&key(b))
        });
        items.into_iter().map(|i| i.label).collect()
    }

    fn hover(anchor: &str, edit: &str) -> String {
        let s = summary();
        let (text, at) = edited(anchor, edit);
        Ide::new(&text, &s, at).hover().unwrap_or_default()
    }

    #[test]
    fn fields_and_methods_after_a_dot() {
        let got = labels("    bird.vy += 1.0;", "    t.position.$0");
        assert_eq!(got, vec!["x", "y", "length"]);
    }

    #[test]
    fn fields_of_a_query_tuple_element() {
        let got = labels("        t.position = vec2(0.0, 0.0);", "        bird.$0");
        assert_eq!(got, vec!["vy"]);
        let got = labels("        t.position = vec2(0.0, 0.0);", "        t.$0");
        assert_eq!(got, vec!["position", "rotation"]);
    }

    #[test]
    fn fields_of_a_generic_instance_and_of_a_let() {
        let got = labels("    game.score = 0;", "    state.$0");
        assert!(
            got.contains(&"now".to_owned()) && got.contains(&"set".to_owned()),
            "{got:?}"
        );
        let got = labels(
            "    game.score = 0;",
            "    let v = vec2(1.0, 2.0);\n    v.$0",
        );
        assert_eq!(got, vec!["x", "y", "length"]);
    }

    #[test]
    fn enum_variants_after_the_enum() {
        let got = labels("    game.score = 0;", "    let p = Phase::$0");
        assert_eq!(got, vec!["Ready", "Playing", "Over"]);
    }

    #[test]
    fn the_variants_of_the_compared_value_come_first() {
        let got = labels("    game.score = 0;", "    if state.now == $0");
        assert_eq!(&got[..3], ["Phase::Ready", "Phase::Playing", "Phase::Over"]);
    }

    #[test]
    fn struct_literal_fields_leave_out_the_written_ones() {
        let got = labels(
            "        t.position = vec2(0.0, 0.0);",
            "        let x = Transform { rotation: 1.0, $0 };",
        );
        assert_eq!(got, vec!["position"]);
    }

    #[test]
    fn attribute_names_fit_the_item() {
        let got = labels("@resource struct Game", "@$0\nstruct Game");
        assert_eq!(got, vec!["component", "resource"]);
        let got = labels("@system(run_if: [ready])", "@$0");
        assert_eq!(got, vec!["system"]);
    }

    #[test]
    fn attribute_parameters_leave_out_the_written_ones() {
        let got = labels("@system(run_if: [ready])", "@system(run_if: [ready], $0)");
        assert_eq!(got, vec!["after", "in_state", "on_enter", "schedule"]);
    }

    #[test]
    fn attribute_values_follow_the_declaration() {
        // A declared enum: its variants.
        let got = labels("@system(run_if: [ready])", "@system(schedule: $0)");
        assert_eq!(got, vec!["Schedule::Startup", "Schedule::Update"]);
        // Any enum: the variants of the enums the host marks, not others.
        let got = labels("@system(on_enter: Phase::Ready)", "@system(on_enter: $0)");
        assert_eq!(got, vec!["Phase::Ready", "Phase::Playing", "Phase::Over"]);
        let got = labels(
            "@system(on_enter: Phase::Ready)",
            "@system(in_state: [Phase::Ready, $0])",
        );
        assert_eq!(got, vec!["Phase::Playing", "Phase::Over"]);
        // A type: the structs carrying the attribute first.
        let got = labels(
            "@component(requires: [Transform])",
            "@component(requires: [$0])",
        );
        assert_eq!(&got[..1], ["Bird"]);
        assert!(got.contains(&"Transform".to_owned()));
        assert!(
            !got.contains(&"Pipe".to_owned()),
            "the struct being declared: {got:?}"
        );
    }

    #[test]
    fn function_arguments_rank_like_the_program_uses_them() {
        // `run_if` already names a function without the attribute, so those
        // come first; `after` has no use, so the systems do.
        let got = labels("@system(on_enter: Phase::Ready)", "@system(run_if: [$0])");
        assert_eq!(got[0], "ready");
        let got = labels("@system(on_enter: Phase::Ready)", "@system(after: [$0])");
        assert_eq!(got[0], "fall");
        assert!(
            !got.contains(&"reset".to_owned()),
            "the system itself: {got:?}"
        );
    }

    #[test]
    fn attribute_skeletons_at_the_top_level() {
        let got = labels("fn ready(", "$0\nfn ready(");
        for want in [
            "@component struct",
            "@state enum",
            "@system fn",
            "struct",
            "fn",
        ] {
            assert!(got.contains(&want.to_owned()), "{want} in {got:?}");
        }
    }

    #[test]
    fn locals_and_types() {
        let got = labels("    game.score = 0;", "    $0");
        assert!(got.contains(&"birds".to_owned()) && got.contains(&"vec2".to_owned()));
        let got = labels("fn fall(bird: Bird,", "fn fall(bird: $0,");
        assert!(got.contains(&"Bird".to_owned()) && got.contains(&"int".to_owned()));
        assert!(!got.contains(&"vec2".to_owned()), "{got:?}");
    }

    #[test]
    fn hover_shows_the_type_and_the_comment() {
        let h = hover("    bird.vy += 1.0;", "    t.position.len$0gth();");
        assert!(
            h.contains("fn length() -> float") && h.contains("How far from the origin."),
            "{h}"
        );
        let h = hover("    bird.vy += 1.0;", "    let p = t.posi$0tion;");
        assert!(h.contains("Transform.position: Vec2"), "{h}");
        let h = hover(
            "        t.position = vec2(0.0, 0.0);",
            "        bi$0rd.vy = 1.0;",
        );
        assert!(h.contains("bird: Bird"), "{h}");
        let h = hover("fn vec2(", "fn ve$0c2(");
        assert!(h.contains("fn vec2(x: float, y: float) -> Vec2"), "{h}");
        let h = hover("struct Vec2 {", "struct Ve$0c2 {");
        assert!(h.contains("A point on the plane."), "{h}");
    }

    #[test]
    fn signature_help_for_calls_and_attributes() {
        let s = summary();
        let (text, at) = edited(
            "        t.position = vec2(0.0, 0.0);",
            "        t.position = vec2(0.0, $0",
        );
        let help = Ide::new(&text, &s, at)
            .signature_help()
            .expect("a signature");
        assert_eq!(
            help.signatures[0].label,
            "fn vec2(x: float, y: float) -> Vec2"
        );
        assert_eq!(help.active_parameter, Some(1));
        let (text, at) = edited("@system(run_if: [ready])", "@system(schedule: $0");
        let help = Ide::new(&text, &s, at)
            .signature_help()
            .expect("a signature");
        assert!(
            help.signatures[0]
                .label
                .starts_with("@system(after: fn[] = []")
        );
        assert_eq!(help.active_parameter, Some(4));
    }

    #[test]
    fn definitions_of_fields_methods_attributes_and_locals() {
        let s = summary();
        let def = |anchor: &str, edit: &str| {
            let (text, at) = edited(anchor, edit);
            match Ide::new(&text, &s, at).definition() {
                Some(Ok(t)) => GAME[t.span.start as usize..t.span.end as usize].to_owned(),
                Some(Err(at)) => format!("local@{}", &text[at..at + 4]),
                None => String::new(),
            }
        };
        assert_eq!(def("    bird.vy += 1.0;", "    t.posi$0tion;"), "position");
        assert_eq!(
            def("    bird.vy += 1.0;", "    t.position.len$0gth();"),
            "length"
        );
        assert_eq!(
            def("@system(run_if: [ready])", "@sys$0tem(run_if: [ready])"),
            "attribute system"
        );
        assert_eq!(
            def(
                "@system(on_enter: Phase::Ready)",
                "@system(on_enter: Phase::Rea$0dy)"
            ),
            "Ready"
        );
        assert_eq!(
            def("@system(run_if: [ready])", "@system(run_if: [rea$0dy])"),
            "ready"
        );
        assert_eq!(
            def("    bird.vy += 1.0;", "    bi$0rd.vy += 1.0;"),
            "local@bird"
        );
    }

    #[test]
    fn attribute_misuse_the_compiler_accepts_is_a_warning() {
        let s = summary();
        let text = GAME.replace(
            "@system(on_enter: Phase::Ready)",
            "@system(on_enter: Mood::Calm)\n@system",
        );
        let warnings: Vec<&str> = super::attribute_warnings(&text, &s)
            .into_iter()
            .map(|w| w.3)
            .collect();
        assert_eq!(
            warnings,
            vec!["attribute_unmarked_enum", "attribute_repeated"]
        );
        assert!(super::attribute_warnings(GAME, &s).is_empty());
    }
}

#[cfg(test)]
mod more_tests {
    use super::{Ide, Symbol, hover_markdown};
    use crate::analysis::{ProgramSummary, analyze};
    use candela::compiler::imports::ImportResolver;

    /// The forms `tests::GAME` leaves out: a required attribute parameter, a
    /// constant, payload variants, functions without `self`, methods on an
    /// enum, and the expressions a type is followed through.
    const MORE: &str = r#"host "app" {
    attribute settings(section: string) on struct;
    attribute flagged(on: bool = true, weight: int = 1, tags: string[] = []) on fn;
}

const LIMIT: int = 3;

/// A shape on the board.
enum Shape {
    /// Round.
    Circle(float),
    Square(float, float),
    Dot,
}
impl Shape {
    fn area(self) -> float { return 1.0; }
}

/// Tuning read from the settings.
@settings(section: "board")
struct Board {
    /// How wide.
    width: int = 4,
    names: string[] = [],
    lookup: {string: int} = {},
}
impl Board {
    fn empty() -> Board { return Board { .. }; }
    fn cell(self, i: int) -> (int, string) { return (i, "a"); }
}

fn pair(a, b) { return a + b; }

struct Holder<T> { value: T }

@flagged(weight: 2)
fn run(board: Board, shape: Shape) {
    let n: int = LIMIT;
    let (i, name) = board.cell(1);
    let names = board.names;
    let first = board.names[0];
    let count = board.lookup["a"];
    let all = [1.5, 2.5];
    let both = (1, "x");
    let neg = -n;
    let flag = !true;
    let s = "abc";
    let words = s.split(",");
    let size = names.len();
    let grown = Board { width: 9, .. }.width;
    let any_n: any = n;
    let as_int = any_n as int;
    let total = pair(1, 2);
    for k in 0..n {
        print(k);
    }
    for w in names {
        print(w);
    }
    let v = Shape::Circle(1.0);
    let d = Shape::Dot;
    let lim = LIMIT;
    let cmp = n > 1;
    let part = names[0..1];
    let ch = s[0];
    let table = board.lookup;
    let second = both.1;
    let held = Holder<int> { value: 1 };
    let inner = held.value;
    for key in table {
        print(key);
    }
    print(shape.area() + float(size) + float(i) + all[0]);
}
"#;

    fn summary() -> ProgramSummary {
        let outcome = analyze(MORE, "more.cdl", &ImportResolver::new());
        assert!(
            outcome.errors.is_empty(),
            "{:?}",
            outcome.errors.first().map(|d| d.message.clone())
        );
        outcome.summary.expect("the program compiles")
    }

    fn edited(anchor: &str, edit: &str) -> (String, usize) {
        assert!(MORE.contains(anchor), "{anchor}");
        let text = MORE.replacen(anchor, edit, 1);
        let at = text.find("$0").expect("the edit marks the cursor");
        (text.replacen("$0", "", 1), at)
    }

    fn labels(s: &ProgramSummary, anchor: &str, edit: &str) -> Vec<String> {
        let (text, at) = edited(anchor, edit);
        let mut items = Ide::new(&text, s, at).completions();
        items.sort_by(|a, b| {
            let key = |i: &tower_lsp::lsp_types::CompletionItem| {
                (
                    i.sort_text.clone().unwrap_or_else(|| i.label.clone()),
                    i.label.clone(),
                )
            };
            key(a).cmp(&key(b))
        });
        items.into_iter().map(|i| i.label).collect()
    }

    /// The type a local's hover reports, at the local's declaration.
    fn local_type(s: &ProgramSummary, name: &str) -> String {
        let at = MORE
            .lines()
            .filter(|l| l.trim_start().starts_with("let "))
            .find_map(|l| {
                let pattern = l.split('=').next()?;
                let col = pattern.match_indices(name).map(|(c, _)| c).find(|&c| {
                    let word =
                        |b: Option<u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
                    !word(pattern.as_bytes().get(c + name.len()).copied())
                        && !word(
                            c.checked_sub(1)
                                .and_then(|p| pattern.as_bytes().get(p).copied()),
                        )
                })?;
                let line_start = MORE.find(l)?;
                Some(line_start + col)
            })
            .expect("declared");
        match Ide::new(MORE, s, at).symbol() {
            Some(Symbol::Local { ty, .. }) => ty.render(),
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn types_follow_expressions() {
        let s = summary();
        for (name, ty) in [
            ("n", "int"),
            ("i", "int"),
            ("name", "string"),
            ("names", "string[]"),
            ("first", "string"),
            ("count", "int"),
            ("all", "float[]"),
            ("both", "(int, string)"),
            ("neg", "int"),
            ("flag", "bool"),
            ("s", "string"),
            ("words", "string[]"),
            ("size", "int"),
            ("grown", "int"),
            ("as_int", "int"),
            ("total", "int"),
            ("v", "Shape"),
            ("d", "Shape"),
            ("lim", "int"),
            ("cmp", "bool"),
            ("part", "string[]"),
            ("ch", "string"),
            ("table", "{string: int}"),
            ("second", "string"),
            ("held", "Holder<int>"),
            ("inner", "int"),
        ] {
            assert_eq!(local_type(&s, name), ty, "{name}");
        }
        // Loop variables over a range and over a list.
        let at = MORE.find("print(k)").expect("k") + 6;
        assert!(
            matches!(Ide::new(MORE, &s, at).symbol(), Some(Symbol::Local { ty, .. }) if ty.render() == "int")
        );
        let at = MORE.find("print(w)").expect("w") + 6;
        assert!(
            matches!(Ide::new(MORE, &s, at).symbol(), Some(Symbol::Local { ty, .. }) if ty.render() == "string")
        );
    }

    #[test]
    fn members_of_tuples_enums_and_builtins() {
        let s = summary();
        let anchor = "    let v = Shape::Circle(1.0);";
        assert_eq!(labels(&s, anchor, "    both.$0"), vec!["0", "1"]);
        assert_eq!(labels(&s, anchor, "    shape.$0"), vec!["area"]);
        let words = labels(&s, anchor, "    s.$0");
        assert!(
            words.contains(&"split".to_owned()) && words.contains(&"len".to_owned()),
            "{words:?}"
        );
        assert!(!words.contains(&"push".to_owned()), "{words:?}");
        let list = labels(&s, anchor, "    names.$0");
        assert!(
            list.contains(&"push".to_owned()) && !list.contains(&"uppercase".to_owned()),
            "{list:?}"
        );
        // A call's result and a struct literal chain like a name.
        assert_eq!(
            labels(&s, anchor, "    Board::empty().$0")[..3],
            ["lookup", "names", "width"]
        );
        assert_eq!(labels(&s, anchor, "    board.cell(1).$0"), vec!["0", "1"]);
        assert_eq!(labels(&s, anchor, "    (board).$0")[..1], ["lookup"]);
    }

    #[test]
    fn paths_types_and_values() {
        let s = summary();
        let anchor = "    let v = Shape::Circle(1.0);";
        // A struct's functions without `self`, after its name.
        assert_eq!(labels(&s, anchor, "    let b = Board::$0"), vec!["empty"]);
        // Variants with payloads, and the enum's methods without `self`.
        assert_eq!(
            labels(&s, anchor, "    let x = Shape::$0"),
            vec!["Circle", "Square", "Dot"]
        );
        // Types after `->`, after `let x:` and in a struct declaration.
        let types = labels(&s, "fn pair(a, b)", "fn pair(a, b) -> $0");
        assert!(types.contains(&"Board".to_owned()) && types.contains(&"Shape".to_owned()));
        assert!(labels(&s, anchor, "    let x: $0").contains(&"int".to_owned()));
        assert!(labels(&s, "    width: int = 4,", "    width: $0").contains(&"Board".to_owned()));
        assert!(labels(&s, anchor, "    let x: (int, $0").contains(&"Shape".to_owned()));
        // A field's value: the enum's variants first.
        let text = "    let b = Board { width: $0 };";
        assert!(labels(&s, anchor, text).contains(&"LIMIT".to_owned()));
    }

    #[test]
    fn attribute_arguments_of_every_kind() {
        let s = summary();
        let anchor = "@flagged(weight: 2)";
        assert_eq!(
            labels(&s, anchor, "@flagged(on: $0)"),
            vec!["false", "true"]
        );
        assert!(labels(&s, anchor, "@flagged(weight: $0)").is_empty());
        // The required parameter is written by the completion.
        let (text, at) = edited(
            "@settings(section: \"board\")",
            "@$0\nstruct Other {}\n@settings(section: \"board\")",
        );
        let items = Ide::new(&text, &s, at).completions();
        let settings = items
            .iter()
            .find(|i| i.label == "settings")
            .expect("settings");
        assert_eq!(
            settings.insert_text.as_deref(),
            Some("settings(section: ${1})")
        );
        // The top-level skeleton names it too.
        let (text, at) = edited("fn pair(a, b)", "$0\nfn pair(a, b)");
        let items = Ide::new(&text, &s, at).completions();
        let skeleton = items
            .iter()
            .find(|i| i.label == "@settings struct")
            .expect("skeleton");
        assert!(
            skeleton
                .insert_text
                .as_deref()
                .unwrap_or_default()
                .starts_with("@settings(section: ${2})")
        );
    }

    #[test]
    fn hover_on_every_kind_of_declaration() {
        let s = summary();
        let hover = |anchor: &str, edit: &str| {
            let (text, at) = edited(anchor, edit);
            Ide::new(&text, &s, at).hover().unwrap_or_default()
        };
        let anchor = "    let v = Shape::Circle(1.0);";
        assert!(
            hover(anchor, "    let v = Shape::Circ$0le(1.0);")
                .contains("Shape::Circle(float)\n```\n\nRound.")
        );
        assert!(
            hover(anchor, "    let v = Sha$0pe::Dot;")
                .contains("enum Shape { Circle(float), Square(float, float), Dot }")
        );
        assert!(
            hover("@flagged(weight: 2)", "@flagged(wei$0ght: 2)")
                .contains("@flagged(weight: int = 1)")
        );
        assert!(
            hover("@flagged(weight: 2)", "@flag$0ged(weight: 2)")
                .contains("attribute flagged(on: bool = true")
        );
        assert!(hover(anchor, "    pri$0nt(1);").contains("(builtin)"));
        assert!(hover(anchor, "    s.upper$0case();").contains("(builtin)"));
        assert!(
            hover("    width: int = 4,", "    wid$0th: int = 4,")
                .contains("Board.width: int = 4\n```\n\nHow wide.")
        );
        assert!(
            hover(anchor, "    let x = Board { wid$0th: 1, .. };").contains("Board.width: int")
        );
        assert!(hover("fn pair(a, b)", "fn pa$0ir(a, b)").contains("Called as:"));
        assert!(hover(anchor, "    let x = LIM$0IT;").contains("const LIMIT: int"));
        assert!(hover(anchor, "    let x = Board::emp$0ty();").contains("fn empty() -> Board"));
        assert!(hover("    fn area(self)", "    fn ar$0ea(self)").contains("impl Shape"));
        assert!(
            hover("struct Board {", "struct Bo$0ard {").contains("Tuning read from the settings.")
        );
    }

    #[test]
    fn definitions_of_types() {
        let s = summary();
        let def = |anchor: &str, edit: &str| {
            let (text, at) = edited(anchor, edit);
            match Ide::new(&text, &s, at).definition() {
                Some(Ok(t)) => MORE[t.span.start as usize..t.span.end as usize].to_owned(),
                Some(Err(_)) => "local".to_owned(),
                None => String::new(),
            }
        };
        let anchor = "    let v = Shape::Circle(1.0);";
        assert_eq!(def(anchor, "    let v = Sha$0pe::Dot;"), "Shape");
        assert_eq!(def(anchor, "    let v = Boa$0rd::empty();"), "Board");
        assert_eq!(def(anchor, "    pri$0nt(1);"), "");
    }

    #[test]
    fn signatures_of_methods_variants_and_builtins() {
        let s = summary();
        let sig = |anchor: &str, edit: &str| {
            let (text, at) = edited(anchor, edit);
            Ide::new(&text, &s, at)
                .signature_help()
                .map(|h| (h.signatures[0].label.clone(), h.active_parameter))
        };
        let anchor = "    let v = Shape::Circle(1.0);";
        assert_eq!(
            sig(anchor, "    board.cell($0"),
            Some(("fn cell(i: int) -> (int, string)".to_owned(), Some(0)))
        );
        assert_eq!(
            sig(anchor, "    let x = Shape::Square(1.0, $0"),
            Some(("Shape::Square(float, float)".to_owned(), Some(1)))
        );
        assert_eq!(
            sig(anchor, "    let x = Board::empty($0"),
            Some(("fn empty() -> Board".to_owned(), Some(0)))
        );
        assert_eq!(
            sig(anchor, "    print($0").map(|s| s.0),
            Some("print(T)".to_owned())
        );
        assert_eq!(
            sig(anchor, "    let x = Board { width: pair($0 }").map(|s| s.0),
            Some("fn pair(a, b) -> int".to_owned())
        );
        assert_eq!(sig(anchor, "    pair(fn(x) { $0"), None);
        assert_eq!(
            sig("@flagged(weight: 2)", "@flagged(weight: 2, $0)").and_then(|s| s.1),
            Some(0)
        );
        assert_eq!(sig(anchor, "    // pair($0"), None);
    }

    #[test]
    fn hover_text_of_a_builtin_and_a_local() {
        let s = summary();
        let at = MORE.find("let n").expect("n") + 4;
        let sym = Ide::new(MORE, &s, at).symbol().expect("a local");
        assert_eq!(hover_markdown(&sym), "```candela\nn: int\n```");
        assert!(Ide::new(MORE, &s, 0).completions().is_empty() || MORE.starts_with("host"));
    }
}

#[cfg(test)]
mod doc_tests {
    use super::Ide;
    use crate::analysis::{ProgramSummary, analyze};
    use candela::compiler::imports::ImportResolver;
    use std::path::PathBuf;

    const DOCS: &str = r#"host "app" {
    /// Marks a struct the app keeps.
    attribute kept on struct;
}

/// How a body moves.
///
/// The kinds differ in what moves them.
enum Kind {
    /// Never moves.
    Fixed,
    /// Moved by forces.
    ///
    /// Gravity included.
    Free,
    Scripted,
}

/// A physics body.
///
/// More detail that a completion list leaves out.
@kept
struct Body {
    /// Fixed, free or scripted.
    kind: Kind = Kind::Free,
    /// Multiplies gravity.
    gravity_scale: float = 1.0,
    mass: float,
}

impl Body {
    /// Pushes the body.
    fn push(self, x: float) -> float { return x * self.mass; }
}

/// A value in a box.
struct Boxed<T> {
    /// What the box holds.
    value: T,
}

fn open(b: Boxed<int>) -> int { return b.value; }

// A plain comment, which is not documentation.
fn plain() -> int { return 1; }

//// A rule line, which is not documentation either.
fn ruled() -> int { return 2; }

fn main() {
    let b = Body { mass: 2.0, .. };
    let k = Kind::Fixed;
    b.push(1.0);
    plain();
    ruled();
}
"#;

    /// A package in a temporary directory whose manifest has `docs` (when
    /// given), with `DOCS` as its entry.
    fn package(tag: &str, docs: Option<&str>) -> (PathBuf, ProgramSummary) {
        let root =
            std::env::temp_dir().join(format!("candela-lsp-docs-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).expect("a temporary directory");
        let mut manifest = String::from("[package]\nname = \"app\"\nversion = \"0.1.0\"\n");
        if let Some(docs) = docs {
            manifest.push_str(&format!("docs = \"{docs}\"\n"));
        }
        std::fs::write(root.join("candela.toml"), manifest).expect("a manifest");
        let path = root.join("src").join("main.cdl");
        std::fs::write(&path, DOCS).expect("the entry");
        let outcome = analyze(
            DOCS,
            path.to_str().expect("a UTF-8 path"),
            &ImportResolver::new(),
        );
        assert!(
            outcome.errors.is_empty(),
            "{:?}",
            outcome.errors.first().map(|d| d.message.clone())
        );
        (root, outcome.summary.expect("the program compiles"))
    }

    fn hover(s: &ProgramSummary, at: &str) -> String {
        let marked = at.replace("$0", "");
        let start = DOCS.find(&marked).unwrap_or_else(|| panic!("{marked}"));
        let offset = start + at.find("$0").expect("a cursor");
        Ide::new(DOCS, s, offset).hover().unwrap_or_default()
    }

    #[test]
    fn hover_shows_an_enum_with_every_variant_and_its_summary() {
        let (root, s) = package("enum", None);
        let h = hover(&s, "enum K$0ind {");
        std::fs::remove_dir_all(root).ok();
        assert!(
            h.starts_with("```candela\nenum Kind { Fixed, Free, Scripted }\n```"),
            "{h}"
        );
        assert!(
            h.contains("How a body moves.\n\nThe kinds differ in what moves them."),
            "{h}"
        );
        assert!(
            h.contains(
                "**Variants**\n\n- `Fixed`: Never moves.\n- `Free`: Moved by forces.\n- `Scripted`"
            ),
            "{h}"
        );
        assert!(
            !h.contains("Gravity included."),
            "a variant shows its summary only: {h}"
        );
        assert!(!h.contains("Open docs"), "no docs URL, no link: {h}");
    }

    #[test]
    fn hover_on_a_variant_shows_its_own_doc() {
        let (root, s) = package("variant", None);
        let h = hover(&s, "let k = Kind::Fi$0xed;");
        std::fs::remove_dir_all(root).ok();
        assert_eq!(h, "```candela\nKind::Fixed\n```\n\nNever moves.");
    }

    #[test]
    fn hover_shows_a_struct_with_its_fields_defaults_and_summaries() {
        let (root, s) = package("struct", None);
        let h = hover(&s, "struct Bo$0dy {");
        std::fs::remove_dir_all(root).ok();
        assert!(
            h.contains(
                "struct Body {\n    kind: Kind = Kind::Free,\n    gravity_scale: float = 1.0,\n    mass: float,\n}"
            ),
            "{h}"
        );
        assert!(h.contains("A physics body.\n\nMore detail"), "{h}");
        assert!(
            h.contains(
                "**Fields**\n\n- `kind`: Fixed, free or scripted.\n- `gravity_scale`: Multiplies gravity.\n- `mass`"
            ),
            "{h}"
        );
        let h = hover(&s, "    gravity_sc$0ale: float = 1.0,");
        assert!(
            h.contains("Body.gravity_scale: float = 1.0\n```\n\nMultiplies gravity."),
            "{h}"
        );
    }

    #[test]
    fn hover_on_a_generic_instance_shows_its_template_doc() {
        let (root, s) = package("generic", Some("https://example.com/{name}"));
        let h = hover(&s, "fn open(b: Box$0ed<int>)");
        std::fs::remove_dir_all(root).ok();
        assert!(h.contains("struct Boxed<int> {\n    value: int,\n}"), "{h}");
        assert!(h.contains("A value in a box."), "{h}");
        assert!(h.contains("- `value`: What the box holds."), "{h}");
        assert!(h.ends_with("[Open docs](https://example.com/Boxed)"), "{h}");
    }

    #[test]
    fn only_triple_slash_comments_are_documentation() {
        let (root, s) = package("plain", None);
        let plain = hover(&s, "fn pla$0in()");
        let ruled = hover(&s, "fn rul$0ed()");
        let attr = hover(&s, "@ke$0pt");
        std::fs::remove_dir_all(root).ok();
        assert_eq!(plain, "```candela\nfn plain() -> int\n```");
        assert_eq!(ruled, "```candela\nfn ruled() -> int\n```");
        assert!(attr.ends_with("Marks a struct the app keeps."), "{attr}");
    }

    #[test]
    fn completion_documentation_is_the_summary() {
        let (root, s) = package("complete", None);
        let at = DOCS.find("    plain();").expect("plain") + 4;
        let items = Ide::new(DOCS, &s, at).completions();
        std::fs::remove_dir_all(root).ok();
        let body = items.iter().find(|i| i.label == "Body").expect("Body");
        let Some(tower_lsp::lsp_types::Documentation::MarkupContent(m)) = &body.documentation
        else {
            panic!("Body has no documentation: {body:?}");
        };
        assert_eq!(m.value, "A physics body.");
    }

    #[test]
    fn hover_links_to_the_documentation_the_package_names() {
        let (root, s) = package("link", Some("https://example.com/api?name={name}"));
        let links: Vec<String> = [
            "enum K$0ind {",
            "let k = Kind::Fi$0xed;",
            "struct Bo$0dy {",
            "    gravity_sc$0ale: float = 1.0,",
            "    b.pu$0sh(1.0);",
            "fn pla$0in()",
            "@ke$0pt",
        ]
        .iter()
        .map(|at| {
            let h = hover(&s, at);
            h.rsplit_once("[Open docs](")
                .map_or_else(String::new, |(_, l)| l.trim_end_matches(')').to_owned())
        })
        .collect();
        std::fs::remove_dir_all(root).ok();
        let base = "https://example.com/api?name=";
        assert_eq!(
            links,
            [
                format!("{base}Kind"),
                format!("{base}Kind%3A%3AFixed"),
                format!("{base}Body"),
                format!("{base}Body.gravity_scale"),
                format!("{base}Body.push"),
                format!("{base}plain"),
                format!("{base}%40kept"),
            ]
        );
    }
}
