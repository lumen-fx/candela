//! Static analysis over a candela buffer, built entirely on top of candela's
//! own lexer/parser/type-checker (`candela::check_only`). This module does not
//! reimplement any language frontend logic: it calls `check_only()`,
//! `collect_warnings` and `collect_diagnostic`, then copies the symbol tables
//! the compile produced (`CompileOutput`) into plain owned data the editor
//! features in `crate::ide` read.
//!
//! `check_only()` is the compile `candela check` performs: the program, and
//! then an entry point for every fully annotated function in the buffer, which
//! is what type-checks the body of a function nothing in the program calls.
//! Going through it is what keeps the editor and the command line reporting
//! the same errors on the same file. Like `check`, it wants no `main`, and it
//! opens no library a `dylib` block names, so editing never loads native code.
//!
//! Nothing here runs the program (as `candela::Engine::compile` would): it
//! parses, type-checks and generates code, so running on every keystroke has
//! no script side effects (no `print` output, no host calls, no infinite loops
//! from the user's own code).
//!
//! candela's error funnel (parser + compiler + runtime) is fatal-on-first-
//! error: `collect_diagnostic` yields at most one `Diagnostic` per call, not
//! a full list. That is a property of the underlying compiler, not a
//! limitation added here; see the crate README for what this means for
//! `publishDiagnostics`.
//!
//! `candela::compiler::compiler_data::Struct` and `EnumType` (unlike
//! `Function`) are not tagged with the source file that declared them. This
//! module places one in the first source file whose text at the declaration's
//! `name_span` is its name (see `declaring_file`), a heuristic rather than a
//! compiler-guaranteed invariant.

use candela::check_only;
use candela::compiler::attributes::{AttrParamTypeExpr, AttrValueExpr, AttributeTargetKind};
use candela::compiler::compiler_data::Function;
use candela::compiler::expr::{METHOD_SEP, Span};
use candela::compiler::imports::ImportResolver;
use candela::compiler::type_system::{ANON_FN_PREFIX, TypeExpr};
use candela::compiler::{CompileOutput, Namespace, SymbolKind};
use candela::macros::MacroEnv;
use candela::{AttributeTarget, AttributeValue};
use candela::{Diagnostic, TypeNames, collect_diagnostic, collect_warnings};

/// One file the compile read: the buffer (index 0) or a module it imports.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: String,
    /// The text the compile read, which is what every span into this file
    /// indexes.
    pub text: String,
    /// Whether the file is a standard library module.
    pub is_std: bool,
}

/// A function parameter, with its declared type when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: Option<String>,
}

impl Param {
    #[must_use]
    pub fn render(&self) -> String {
        match &self.ty {
            Some(ty) => format!("{}: {ty}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A function or method declaration.
#[derive(Debug, Clone)]
pub struct FunctionSymbol {
    /// The name in the compiler's table. A method's is `Type#name`.
    pub name: String,
    /// For a method, the type its `impl` block attaches it to and its own
    /// name.
    pub method: Option<(String, String)>,
    pub name_span: Span,
    pub params: Vec<Param>,
    /// The declared return type, or the one inferred when every call site
    /// agrees on it.
    pub ret: Option<String>,
    /// Concrete `(params) -> return` signatures this function has been
    /// specialized for, sourced from `Function::return_type_cache`. Empty when
    /// the function has not been called anywhere in the compiled program yet.
    pub signatures: Vec<String>,
    /// Index into `ProgramSummary::sources`.
    pub src_file: u16,
    /// The `//` comment lines written right above the declaration.
    pub doc: String,
    /// Declared with `const NAME: T = expr;`.
    pub is_const: bool,
}

impl FunctionSymbol {
    /// The name a call writes: the method's own name for a method.
    #[must_use]
    pub fn label(&self) -> &str {
        self.method.as_ref().map_or(&self.name, |(_, name)| name)
    }

    /// The parameters a call passes: a method's `self` is its receiver.
    pub fn call_params(&self) -> impl Iterator<Item = &Param> {
        let skip = usize::from(
            self.method.is_some() && self.params.first().is_some_and(|p| p.name == "self"),
        );
        self.params.iter().skip(skip)
    }

    /// `fn name(a: int, b) -> T`, the way the declaration reads.
    #[must_use]
    pub fn header(&self) -> String {
        if self.is_const {
            return format!(
                "const {}: {}",
                self.name,
                self.ret.as_deref().unwrap_or("any")
            );
        }
        let params: Vec<String> = self.call_params().map(Param::render).collect();
        let ret = self
            .ret
            .as_ref()
            .map(|r| format!(" -> {r}"))
            .unwrap_or_default();
        format!("fn {}({}){ret}", self.label(), params.join(", "))
    }
}

#[derive(Debug, Clone)]
pub struct FieldSymbol {
    pub name: String,
    pub ty: String,
    pub span: Span,
    pub doc: String,
}

#[derive(Debug, Clone)]
pub struct StructSymbol {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<FieldSymbol>,
    /// The file that declares it; see `declaring_file`. `None` for a tuple
    /// type and for a struct the heuristic could not place.
    pub src_file: Option<u16>,
    pub doc: String,
}

impl StructSymbol {
    /// A tuple type is a struct the compiler declares, named the way the type
    /// is written.
    #[must_use]
    pub fn is_tuple(&self) -> bool {
        self.name.starts_with('(')
    }
}

#[derive(Debug, Clone)]
pub struct VariantSymbol {
    pub name: String,
    pub payload: Vec<String>,
    pub span: Span,
    pub doc: String,
}

#[derive(Debug, Clone)]
pub struct EnumSymbol {
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<VariantSymbol>,
    pub src_file: Option<u16>,
    pub doc: String,
}

/// What an attribute parameter accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrParamKind {
    /// Any type the script can name.
    Type,
    /// A function the script declares.
    Fn,
    /// A payload-free variant of any enum the script declares.
    AnyEnum,
    /// A variant of the named enum.
    Enum(String),
    Bool,
    /// An int, a float or a string.
    Literal,
}

#[derive(Debug, Clone)]
pub struct AttrParam {
    pub name: String,
    pub kind: AttrParamKind,
    /// Declared as a list (`fn[]`, `type[]`, `enum[]`).
    pub list: bool,
    /// The type as the declaration writes it.
    pub ty: String,
    pub default: Option<String>,
    /// Written `name: T?`.
    pub optional: bool,
}

impl AttrParam {
    /// Whether a use must give it.
    #[must_use]
    pub const fn required(&self) -> bool {
        !self.optional && self.default.is_none()
    }

    #[must_use]
    pub fn render(&self) -> String {
        let opt = if self.optional { "?" } else { "" };
        match &self.default {
            Some(d) => format!("{}: {}{opt} = {d}", self.name, self.ty),
            None => format!("{}: {}{opt}", self.name, self.ty),
        }
    }
}

/// An attribute a `host` block declares.
#[derive(Debug, Clone)]
pub struct AttributeSymbol {
    /// The host namespace that declares it.
    pub namespace: String,
    pub name: String,
    /// `fn`, `struct` or `enum`: what it may be written on.
    pub target: &'static str,
    pub params: Vec<AttrParam>,
    pub span: Span,
    pub src_file: u16,
    pub doc: String,
}

impl AttributeSymbol {
    /// `attribute name(params) on target`, the way the declaration reads.
    #[must_use]
    pub fn header(&self) -> String {
        let params: Vec<String> = self.params.iter().map(AttrParam::render).collect();
        if params.is_empty() {
            format!("attribute {} on {}", self.name, self.target)
        } else {
            format!(
                "attribute {}({}) on {}",
                self.name,
                params.join(", "),
                self.target
            )
        }
    }
}

/// The item an attribute use is written on, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseOn {
    Fn(String),
    Struct(String),
    Enum(String),
}

/// An attribute the program's scripts wrote on an item.
#[derive(Debug, Clone)]
pub struct AttributeUseSymbol {
    pub namespace: String,
    pub name: String,
    pub on: UseOn,
    /// The functions each `fn` or `fn[]` argument names, by parameter.
    pub fn_args: Vec<(String, Vec<String>)>,
}

/// What a name written in the buffer can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// An index into `ProgramSummary::functions`.
    Function(usize),
    /// An index into `ProgramSummary::structs`; `None` for a generic struct,
    /// which names no one struct.
    Struct(Option<usize>),
    /// An index into `ProgramSummary::enums`.
    Enum(usize),
    /// An index into `ProgramSummary::attributes`.
    Attribute(usize),
}

/// A name the buffer can write as it stands: its own declarations, the items
/// its imports named or a glob import brought in, and `module::name` for each
/// module an import bound.
#[derive(Debug, Clone)]
pub struct ScopeEntry {
    pub label: String,
    pub kind: ScopeKind,
}

/// Everything this module extracts from a successful compile, in plain owned
/// data (no `Rc`, no borrows) so it can be produced synchronously and then
/// freely moved across an `.await` point in the LSP's async handlers.
#[derive(Debug, Clone, Default)]
pub struct ProgramSummary {
    /// Indexed by candela's internal `src_file` id. Index 0 is always the
    /// buffer this summary was built from.
    pub sources: Vec<SourceFile>,
    pub functions: Vec<FunctionSymbol>,
    /// Indexed like the compiler's struct table.
    pub structs: Vec<StructSymbol>,
    /// Indexed like the compiler's enum table.
    pub enums: Vec<EnumSymbol>,
    pub attributes: Vec<AttributeSymbol>,
    pub attribute_uses: Vec<AttributeUseSymbol>,
    /// The names the buffer can write.
    pub scope: Vec<ScopeEntry>,
}

/// The result of analyzing one buffer: a `Diagnostic` on failure (parse or
/// type error), or a `ProgramSummary` on success. Never both; candela's
/// compiler does not currently support returning partial results alongside
/// an error.
#[derive(Debug, Clone, Default)]
pub struct AnalysisOutcome {
    pub diagnostic: Option<Diagnostic>,
    pub summary: Option<ProgramSummary>,
    /// The warnings a compile that succeeded raised. A warning does not stop
    /// the compile, so these come with a summary, never with a diagnostic.
    pub warnings: Vec<Diagnostic>,
}

/// Runs candela's compiler (parse + type-check + codegen, no execution) over
/// `text` as if it were the file at `path`, and extracts a `ProgramSummary`
/// or the first `Diagnostic` produced.
///
/// The compile is the one `candela check` runs, entry points included, so the
/// editor reports the errors the command line reports and no others.
///
/// `path` should be the buffer's real filesystem path (from the LSP URI) so
/// that any `import "..."` statements resolve relative to the right
/// directory, exactly like the `candela` CLI would. `resolver` says where a
/// library import reads from: the standard library, and the packages the
/// editor's settings name (see `crate::backend::ImportSettings`).
#[must_use]
pub fn analyze(text: &str, path: &str, resolver: &ImportResolver) -> AnalysisOutcome {
    let owned = text.to_owned();
    let path = path.to_owned();
    let resolver = resolver.clone();
    // Macros belong to whatever program embeds candela, and the server is not
    // that program: it has no expanders and cannot get them. An unregistered
    // macro therefore compiles as `null` here instead of failing, so a buffer
    // that uses its host's macros still type-checks and still reports its own
    // errors.
    let mut macros = MacroEnv::new();
    macros.allow_unknown(true);
    // The server is not the host either, so it enables no `@cfg` flag. Code a
    // `@cfg` turns off is parsed and dropped, which is what a compile with no
    // flags on does: a syntax error in it is reported, a name it uses that
    // exists only on another target is not.
    // Warnings are collected, never printed: the server talks to the editor
    // over stdio, where a report would corrupt the protocol.
    match macros.scope(move || {
        collect_diagnostic(move || collect_warnings(move || check_only(owned, &path, &resolver)))
    }) {
        Err(diagnostic) => AnalysisOutcome {
            diagnostic: Some(diagnostic),
            summary: None,
            warnings: Vec::new(),
        },
        Ok((out, warnings)) => AnalysisOutcome {
            diagnostic: None,
            summary: Some(build_summary(&out)),
            warnings,
        },
    }
}

fn build_summary(out: &CompileOutput) -> ProgramSummary {
    let sources: Vec<SourceFile> = out
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| SourceFile {
            path: s.filename.to_string(),
            text: s.contents.clone(),
            is_std: u16::try_from(i).is_ok_and(|i| out.namespaces.is_std(i)),
        })
        .collect();

    // The compiler's own renderer, with the tables this compile produced. A
    // type shows up in a tooltip named the way a diagnostic names it.
    let types = TypeNames {
        structs: &out.structs,
        enums: &out.enums,
    };

    let functions = out
        .functions
        .iter()
        .map(|f| function_symbol(f, types, &sources))
        .collect();

    let structs = out
        .structs
        .iter()
        .map(|s| {
            let src_file = if s.is_tuple() {
                None
            } else {
                declaring_file(&sources, &s.name, s.name_span)
            };
            let text = src_file.map_or("", |f| sources[f as usize].text.as_str());
            StructSymbol {
                name: s.name.to_string(),
                name_span: s.name_span,
                fields: s
                    .fields
                    .iter()
                    .map(|(name, ty, span)| {
                        let span = field_name_span(text, name, *span);
                        FieldSymbol {
                            name: name.to_string(),
                            ty: types.of(ty).to_string(),
                            span,
                            doc: own_line_doc(text, span.start as usize),
                        }
                    })
                    .collect(),
                src_file,
                doc: doc_above(text, s.name_span.start as usize),
            }
        })
        .collect();

    let enums = out
        .enums
        .iter()
        .map(|e| {
            let src_file = declaring_file(&sources, &e.name, e.name_span);
            let text = src_file.map_or("", |f| sources[f as usize].text.as_str());
            EnumSymbol {
                name: e.name.to_string(),
                name_span: e.name_span,
                variants: e
                    .variants
                    .iter()
                    .map(|v| VariantSymbol {
                        name: v.name.to_string(),
                        payload: v.payload.iter().map(|t| types.of(t).to_string()).collect(),
                        span: v.name_span,
                        doc: own_line_doc(text, v.name_span.start as usize),
                    })
                    .collect(),
                src_file,
                doc: doc_above(text, e.name_span.start as usize),
            }
        })
        .collect();

    let attributes = out
        .attribute_decls
        .iter()
        .map(|(namespace, file, decl)| {
            let text = sources.get(*file as usize).map_or("", |s| s.text.as_str());
            AttributeSymbol {
                namespace: namespace.to_string(),
                name: decl.name.to_string(),
                target: match decl.target {
                    AttributeTargetKind::Fn => "fn",
                    AttributeTargetKind::Struct => "struct",
                    AttributeTargetKind::Enum => "enum",
                },
                params: decl
                    .params
                    .iter()
                    .map(|p| {
                        let (kind, list) = attr_param_kind(&p.ty);
                        AttrParam {
                            name: p.name.to_string(),
                            kind,
                            list,
                            ty: render_attr_param_type(&p.ty),
                            default: p.default.as_ref().map(render_attr_value),
                            optional: p.optional,
                        }
                    })
                    .collect(),
                span: decl.span,
                src_file: *file,
                doc: doc_above(text, decl.span.start as usize),
            }
        })
        .collect();

    let attribute_uses = out
        .attributes
        .iter()
        .map(|a| AttributeUseSymbol {
            namespace: a.namespace.clone(),
            name: a.name.clone(),
            on: match &a.target {
                AttributeTarget::Fn { name, .. } => UseOn::Fn(name.clone()),
                AttributeTarget::Struct(id) => UseOn::Struct(
                    out.structs
                        .get(*id as usize)
                        .map(|s| s.name.to_string())
                        .unwrap_or_default(),
                ),
                AttributeTarget::Enum(id) => UseOn::Enum(
                    out.enums
                        .get(*id as usize)
                        .map(|e| e.name.to_string())
                        .unwrap_or_default(),
                ),
            },
            fn_args: a
                .args
                .iter()
                .filter_map(|(name, value)| {
                    let fns = fn_names(value);
                    (!fns.is_empty()).then(|| (name.clone(), fns))
                })
                .collect(),
        })
        .collect();

    ProgramSummary {
        sources,
        functions,
        structs,
        enums,
        attributes,
        attribute_uses,
        scope: scope_entries(out.namespaces.root()),
    }
}

/// The function names an attribute argument holds.
fn fn_names(value: &AttributeValue) -> Vec<String> {
    match value {
        AttributeValue::Fn { name, .. } => vec![name.clone()],
        AttributeValue::List(items) => items.iter().flat_map(fn_names).collect(),
        _ => Vec::new(),
    }
}

fn attr_param_kind(ty: &AttrParamTypeExpr) -> (AttrParamKind, bool) {
    match ty {
        AttrParamTypeExpr::Type => (AttrParamKind::Type, false),
        AttrParamTypeExpr::Fn => (AttrParamKind::Fn, false),
        AttrParamTypeExpr::AnyEnum => (AttrParamKind::AnyEnum, false),
        AttrParamTypeExpr::List(inner) => (attr_param_kind(inner).0, true),
        AttrParamTypeExpr::Value(TypeExpr::Array(inner)) => (value_kind(inner), true),
        AttrParamTypeExpr::Value(t) => (value_kind(t), false),
    }
}

fn value_kind(t: &TypeExpr) -> AttrParamKind {
    match t {
        TypeExpr::Identifier(name, _) => match name.as_str() {
            "bool" => AttrParamKind::Bool,
            "int" | "float" | "string" => AttrParamKind::Literal,
            other => AttrParamKind::Enum(other.to_owned()),
        },
        TypeExpr::NamespacedIdentifier(path, _) => {
            path.last().map_or(AttrParamKind::Literal, |n| {
                AttrParamKind::Enum(n.to_string())
            })
        }
        _ => AttrParamKind::Literal,
    }
}

fn render_attr_param_type(ty: &AttrParamTypeExpr) -> String {
    match ty {
        AttrParamTypeExpr::Type => "type".to_owned(),
        AttrParamTypeExpr::Fn => "fn".to_owned(),
        AttrParamTypeExpr::AnyEnum => "enum".to_owned(),
        AttrParamTypeExpr::Value(t) => render_type_expr(t),
        AttrParamTypeExpr::List(inner) => format!("{}[]", render_attr_param_type(inner)),
    }
}

/// A type annotation, written back the way the source spells it.
fn render_type_expr(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Identifier(name, _) => name.to_string(),
        TypeExpr::NamespacedIdentifier(path, _) => path.join("::"),
        TypeExpr::Generic(g) => {
            let mut name: Vec<String> = g.namespace.iter().map(ToString::to_string).collect();
            name.push(g.name.to_string());
            let args: Vec<String> = g.args.iter().map(render_type_expr).collect();
            format!("{}<{}>", name.join("::"), args.join(", "))
        }
        TypeExpr::Array(inner) => format!("{}[]", render_type_expr(inner)),
        TypeExpr::Map(k, v) => format!("{{{}: {}}}", render_type_expr(k), render_type_expr(v)),
        TypeExpr::Union(members) => members
            .iter()
            .map(render_type_expr)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeExpr::Fn(f) => {
            let args: Vec<String> = f.args.iter().map(render_type_expr).collect();
            match &f.return_type {
                Some(r) => format!("fn({}) -> {}", args.join(", "), render_type_expr(r)),
                None => format!("fn({})", args.join(", ")),
            }
        }
    }
}

fn render_attr_value(v: &AttrValueExpr) -> String {
    match v {
        AttrValueExpr::Int(i, _) => i.to_string(),
        AttrValueExpr::Float(f, _) => format!("{f:?}"),
        AttrValueExpr::Bool(b, _) => b.to_string(),
        AttrValueExpr::String(s, _) => format!("\"{s}\""),
        AttrValueExpr::List(items, _) => format!(
            "[{}]",
            items
                .iter()
                .map(render_attr_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        AttrValueExpr::Path(t, _) => render_type_expr(t),
    }
}

/// The names the entry file can write: what its scope holds, and each module
/// an import bound, behind that module's name.
fn scope_entries(root: &Namespace) -> Vec<ScopeEntry> {
    let mut entries: Vec<ScopeEntry> = Vec::new();
    let mut add = |label: String, kind: &SymbolKind| {
        let kind = match kind {
            SymbolKind::Fn(id) => ScopeKind::Function(*id as usize),
            SymbolKind::Struct(id) => ScopeKind::Struct(Some(*id as usize)),
            SymbolKind::Template(_) => ScopeKind::Struct(None),
            SymbolKind::Enum(id) => ScopeKind::Enum(*id as usize),
            SymbolKind::Attribute(id) => ScopeKind::Attribute(*id as usize),
        };
        if !entries.iter().any(|e| e.label == label) {
            entries.push(ScopeEntry { label, kind });
        }
    };
    for (name, kind) in &root.symbols {
        if is_a_written_name(name) {
            add(name.to_string(), kind);
        }
    }
    for (alias, module) in &root.children {
        // A `host` block's namespace has no file behind it; its functions
        // live outside the scope tree.
        if module.import_path.is_empty() {
            continue;
        }
        for (name, kind) in &module.symbols {
            if is_a_written_name(name) && name != "main" {
                add(format!("{alias}::{name}"), kind);
            }
        }
    }
    entries
}

fn function_symbol(f: &Function, types: TypeNames<'_>, sources: &[SourceFile]) -> FunctionSymbol {
    let params = f
        .args
        .iter()
        .map(|(name, ty)| Param {
            name: name.to_string(),
            ty: ty.as_ref().map(|t| types.of(t).to_string()),
        })
        .collect();

    let signatures: Vec<String> = f
        .return_type_cache
        .iter()
        .map(|(args, ret)| {
            let args_s = args
                .iter()
                .map(|t| types.of(t).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("({args_s}) -> {}", types.of(ret))
        })
        .collect();

    let ret = f
        .return_type
        .as_ref()
        .map_or_else(
            || {
                let mut rets = f
                    .return_type_cache
                    .iter()
                    .map(|(_, r)| types.of(r).to_string());
                let first = rets.next()?;
                rets.all(|r| r == first).then_some(first)
            },
            |(t, _)| Some(types.of(t).to_string()),
        )
        // A function that hands nothing back reads without an arrow.
        .filter(|r| r != "null");

    let method = f
        .name
        .split_once(METHOD_SEP)
        .map(|(ty, name)| (ty.to_owned(), name.to_owned()));
    let doc = sources
        .get(f.src_file as usize)
        .map(|s| doc_above(&s.text, f.name_span.start as usize))
        .unwrap_or_default();

    FunctionSymbol {
        name: f.name.to_string(),
        method,
        name_span: f.name_span,
        params,
        ret,
        signatures,
        src_file: f.src_file,
        doc,
        is_const: f.is_const,
    }
}

/// Where a field's name is written. The compiler records the span of the
/// field's type; the name is the last occurrence of it before that.
fn field_name_span(text: &str, name: &str, type_span: Span) -> Span {
    text.get(..type_span.start as usize)
        .and_then(|before| before.rfind(name))
        .and_then(|start| {
            Some(Span {
                start: u32::try_from(start).ok()?,
                end: u32::try_from(start + name.len()).ok()?,
            })
        })
        .unwrap_or(type_span)
}

/// The source file a struct or an enum was declared in: the first whose text
/// at `span` is the declaration's name. An instantiation of a generic,
/// `State<Phase>`, is placed at its template, whose name is the part before
/// the `<`.
fn declaring_file(sources: &[SourceFile], name: &str, span: Span) -> Option<u16> {
    let base = name.split('<').next().unwrap_or(name);
    let range = span.start as usize..span.end as usize;
    sources
        .iter()
        .position(|s| s.text.get(range.clone()) == Some(base))
        .and_then(|i| u16::try_from(i).ok())
}

/// The `//` comment lines right above the declaration that starts on the line
/// holding `offset`, without their markers. Attribute lines between the
/// comment and the declaration are skipped; a blank line ends the comment.
#[must_use]
pub fn doc_above(text: &str, offset: usize) -> String {
    comment_above(text, offset, true)
}

/// The comment lines above `offset`. With `attributes`, the lines of an
/// attribute written in front of the declaration are stepped over, including
/// one written across several lines, read bottom up until the line that opens
/// it with `@`.
fn comment_above(text: &str, offset: usize, attributes: bool) -> String {
    let Some(before) = text.get(..offset) else {
        return String::new();
    };
    let mut lines: Vec<&str> = before.lines().collect();
    // The declaration's own line, up to the name.
    if !before.ends_with('\n') {
        lines.pop();
    }
    let mut doc = Vec::new();
    let mut in_attribute = false;
    while let Some(line) = lines.pop() {
        let t = line.trim();
        if attributes && t.starts_with('@') {
            in_attribute = false;
            if !doc.is_empty() {
                break;
            }
            continue;
        }
        if in_attribute {
            continue;
        }
        if let Some(comment) = t.strip_prefix("//") {
            doc.push(comment.strip_prefix(' ').unwrap_or(comment));
        } else if attributes
            && doc.is_empty()
            && (t.ends_with(',') || t.ends_with(')') || t.ends_with(']'))
        {
            in_attribute = true;
        } else {
            break;
        }
    }
    doc.reverse();
    doc.join("\n").trim().to_owned()
}

/// The comment above a member (a field or a variant), which only has one of
/// its own when the member starts its own line.
fn own_line_doc(text: &str, offset: usize) -> String {
    let Some(before) = text.get(..offset) else {
        return String::new();
    };
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    if before[line_start..].trim().is_empty() {
        comment_above(text, offset, false)
    } else {
        String::new()
    }
}

/// Whether `name` is one the buffer wrote, as opposed to one the compiler made
/// for itself.
///
/// The function table holds both: a closure is hoisted to a synthetic
/// top-level function, and a method is lowered to the mangled free function
/// its call sites resolve through. Neither is a name a person can write, so
/// neither belongs in an outline, in a completion list, or under a cursor.
#[must_use]
pub fn is_a_written_name(name: &str) -> bool {
    !name.starts_with(ANON_FN_PREFIX) && !name.contains(METHOD_SEP)
}

/// Whether `name` is an identifier rather than an operator a method can be
/// named by (`Vec2#+`).
#[must_use]
pub fn is_identifier(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

impl ProgramSummary {
    /// Document symbols: top-level fn declarations that live in the buffer
    /// itself (`src_file == 0`), not ones pulled in via `import`, and not the
    /// ones the compiler made for itself (see [`is_a_written_name`]).
    pub fn own_functions(&self) -> impl Iterator<Item = &FunctionSymbol> {
        self.functions
            .iter()
            .filter(|f| f.src_file == 0 && is_a_written_name(&f.name))
    }

    /// The structs the buffer declares. A tuple type is a struct the compiler
    /// declares, named the way the type is written, so its name can match the
    /// buffer's text without being a declaration.
    pub fn own_structs(&self) -> impl Iterator<Item = &StructSymbol> {
        self.structs
            .iter()
            .filter(|s| s.src_file == Some(0) && !s.is_tuple() && !s.name.contains('<'))
    }

    pub fn own_enums(&self) -> impl Iterator<Item = &EnumSymbol> {
        self.enums
            .iter()
            .filter(|e| e.src_file == Some(0) && !e.name.contains('<'))
    }

    /// Whether `file` is one of the program's own files or packages rather
    /// than a standard library module.
    #[must_use]
    pub fn is_user_file(&self, file: Option<u16>) -> bool {
        file.and_then(|f| self.sources.get(f as usize))
            .is_some_and(|s| !s.is_std)
    }

    #[must_use]
    pub fn struct_named(&self, name: &str) -> Option<&StructSymbol> {
        self.structs.iter().find(|s| s.name == name)
    }

    #[must_use]
    pub fn enum_named(&self, name: &str) -> Option<&EnumSymbol> {
        self.enums.iter().find(|e| e.name == name)
    }

    /// The methods an `impl` block attaches to the type named `ty`, leaving
    /// out the operators.
    pub fn methods_of<'a>(&'a self, ty: &'a str) -> impl Iterator<Item = &'a FunctionSymbol> {
        self.functions.iter().filter(move |f| {
            f.method
                .as_ref()
                .is_some_and(|(t, name)| t == ty && is_identifier(name))
        })
    }

    #[must_use]
    pub fn method(&self, ty: &str, name: &str) -> Option<&FunctionSymbol> {
        self.functions.iter().find(|f| {
            f.method
                .as_ref()
                .is_some_and(|(t, n)| t == ty && n == name && is_identifier(n))
        })
    }

    /// The scope entry the buffer writes as `label` (`name` or
    /// `module::name`).
    #[must_use]
    pub fn lookup(&self, label: &str) -> Option<ScopeKind> {
        self.scope.iter().find(|e| e.label == label).map(|e| e.kind)
    }

    /// The free function the buffer calls as `label`.
    #[must_use]
    pub fn function(&self, label: &str) -> Option<&FunctionSymbol> {
        match self.lookup(label)? {
            ScopeKind::Function(id) => self.functions.get(id),
            _ => None,
        }
    }

    /// The attribute the buffer writes as `@label`.
    #[must_use]
    pub fn attribute(&self, label: &str) -> Option<&AttributeSymbol> {
        match self.lookup(label)? {
            ScopeKind::Attribute(id) => self.attributes.get(id),
            _ => None,
        }
    }

    /// The attribute uses written on `on`.
    pub fn uses_on<'a>(&'a self, on: &'a UseOn) -> impl Iterator<Item = &'a AttributeUseSymbol> {
        self.attribute_uses.iter().filter(move |u| &u.on == on)
    }
}

#[cfg(test)]
mod tests {
    use super::ANON_FN_PREFIX;
    use super::METHOD_SEP;
    use super::analyze;
    use candela::compiler::imports::ImportResolver;

    fn analyze_buffer(text: &str, path: &str) -> super::AnalysisOutcome {
        analyze(text, path, &ImportResolver::new())
    }

    /// A buffer using a macro of the host it is written for still analyzes:
    /// the server has no expanders, so the macro compiles as `null` and the
    /// rest of the file is checked as usual.
    #[test]
    fn a_long_operator_chain_is_analyzed() {
        // Generated code can hold a chain far longer than anyone writes; the
        // server walks it without recursing down it and stays up.
        let chain = vec!["a"; 20_000].join(" + ");
        let source = format!("fn main() {{\n    let a = 1;\n    print({chain});\n}}\n");
        let outcome = analyze_buffer(&source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        assert!(summary.functions.iter().any(|f| f.name == "main"));
    }

    #[test]
    fn an_unknown_macro_is_not_a_diagnostic() {
        let source = "fn main() {\n    let markup = lmn!(<p>hi</p>);\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        assert!(summary.functions.iter().any(|f| f.name == "main"));
    }

    /// Code under a `@cfg` the server does not enable is dropped before names
    /// resolve, so a module or a function that exists only on another target
    /// is not reported.
    #[test]
    fn code_under_a_false_cfg_is_not_a_diagnostic() {
        let source = "@cfg(web)\nimport \"js\";\n\n@cfg(web)\nfn track(e: string) { js::call(\"gtag\", [e]); }\n@cfg(not(web))\nfn track(e: string) {}\n\nfn main() {\n    @cfg(web)\n    let x = only_on_web();\n    track(\"open\");\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
    }

    /// A misspelt function in an attribute is a diagnostic in the editor, as
    /// it is a failure on the command line.
    #[test]
    fn a_bad_attribute_argument_is_reported() {
        let source = "host \"ecs\" {\n    attribute system(after: fn[] = []) on fn;\n}\n\n@system(after: [movment])\nfn spawn() {}\n\nfn movement() {}\n\nfn main() {}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        let diagnostic = outcome.diagnostic.expect("movment is no function");
        assert_eq!(diagnostic.code, "attribute_bad_argument");
    }

    /// A tuple program analyses clean, and the structs the compiler declares
    /// for its tuple types stay out of the outline even where the buffer
    /// spells the type the way the struct is named.
    #[test]
    fn tuples_analyse_and_stay_out_of_the_outline() {
        let source = "fn swap(t: (int, int)) -> (int, int) {\n    let (a, b) = t;\n    return (b, a);\n}\n\nfn main() {\n    print(swap((1, 2)).0);\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        assert!(summary.structs.iter().any(|s| s.name == "(int, int)"));
        assert_eq!(summary.own_structs().count(), 0);
    }

    /// The macro is skipped, not the errors around it.
    #[test]
    fn a_real_error_beside_a_macro_is_still_reported() {
        let source = "fn main() {\n    let markup = lmn!(<p/>);\n    let broken = 1 +\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(outcome.diagnostic.is_some());
    }

    /// A fully annotated function nothing calls is compiled at its declared
    /// parameter types, so an error in its body is a diagnostic in the editor
    /// exactly as it is a failure on the command line.
    #[test]
    fn a_body_error_in_an_uncalled_function_is_reported() {
        let source = "fn broken(a: int) -> int {\n    return a + \"nope\";\n}\n\nfn main() {\n    print(1);\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        let diagnostic = outcome
            .diagnostic
            .expect("the uncalled function's body is checked");
        assert!(
            diagnostic.message.contains("int") && diagnostic.message.contains("string"),
            "{}",
            diagnostic.message
        );
        // The error is placed in the buffer, on the offending expression.
        assert!(source.get(diagnostic.span.clone()).is_some());
    }

    /// A method named by an operator symbol lowers to a mangled free function
    /// like any other method, so it belongs in no outline: `Vec2#+` is not a
    /// name anyone wrote.
    #[test]
    fn an_operator_method_stays_out_of_the_symbols() {
        let source = "struct Vec2 { x: int }\n\
                      impl Vec2 {\n\
                      \x20   fn +(self, other: Vec2) -> Vec2 { return other; }\n\
                      \x20   fn len(self) -> int { return self.x; }\n\
                      }\n\
                      fn main() {\n\
                      \x20   print((Vec2 { x: 1 } + Vec2 { x: 2 }).x);\n\
                      }\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        assert!(
            summary
                .functions
                .iter()
                .any(|f| f.name.contains(METHOD_SEP))
        );
        let outlined: Vec<&str> = summary.own_functions().map(|f| f.name.as_str()).collect();
        assert_eq!(outlined, vec!["main"]);
    }

    /// A type in a tooltip is named the way the compiler names it in a
    /// diagnostic: a user enum reads by the name it was declared under, at the
    /// top level and inside a list or a map. The server used to render every
    /// enum as the bare word `enum`, because it had a renderer of its own.
    #[test]
    fn hover_names_an_enum_typed_parameter() {
        let source = "enum Value { Num(int), Text(string) }\n\
                      fn width(v: Value, xs: Value[], m: {string: Value}) -> int {\n\
                      \x20   return xs.len();\n\
                      }\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        let width = summary
            .own_functions()
            .find(|f| f.name == "width")
            .expect("the function is in the outline");
        let params: Vec<String> = width.params.iter().map(super::Param::render).collect();
        assert_eq!(
            params,
            vec![
                String::from("v: Value"),
                String::from("xs: Value[]"),
                String::from("m: {string: Value}"),
            ]
        );
        assert_eq!(
            width.signatures,
            vec![String::from("(Value, Value[], {string: Value}) -> int")]
        );
    }

    /// A struct's fields are rendered through the same renderer, so an
    /// enum-typed field in the outline and in a struct tooltip is named too.
    #[test]
    fn a_struct_field_of_an_enum_type_is_named() {
        let source = "enum Value { Num(int), Text(string) }\n\
                      struct Holder { first: Value, rest: Value[] }\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        let holder = summary
            .own_structs()
            .find(|s| s.name == "Holder")
            .expect("the struct is in the outline");
        let fields: Vec<(String, String)> = holder
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        assert_eq!(
            fields,
            vec![
                (String::from("first"), String::from("Value")),
                (String::from("rest"), String::from("Value[]")),
            ]
        );
    }

    /// The renderer's spellings are the compiler's, so a dynamic slot reads
    /// `any`, the way the program writes it, and a union is written with `|`,
    /// as in a diagnostic.
    #[test]
    fn a_dynamic_slot_is_spelled_the_way_the_compiler_spells_it() {
        let source = "fn pick(a: any) -> int {\n    return 1;\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        let pick = summary
            .own_functions()
            .find(|f| f.name == "pick")
            .expect("the function is in the outline");
        assert_eq!(pick.params[0].render(), "a: any");
    }

    /// The outline lists what the buffer declares. A closure the type checker
    /// hoists and a method lowered to its mangled free function are entries
    /// the compiler made, under names no one can write, and neither shows up.
    #[test]
    fn the_outline_leaves_out_the_names_the_compiler_made() {
        let source = "struct Point { x: int }\n                      impl Point { fn twice(self) -> int { return self.x * 2; } }\n                      fn apply(f, n) { return f(n); }\n                      fn main() {\n                          print(apply(fn(v) { return v + 1; }, Point{ x: 1 }.twice()));\n                      }\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        let names: Vec<&str> = summary.own_functions().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"apply") && names.contains(&"main"),
            "the functions the buffer declares are there, got {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.starts_with(ANON_FN_PREFIX)),
            "the hoisted closure is not, got {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.contains(METHOD_SEP)),
            "the mangled method is not, got {names:?}"
        );
    }

    /// `check` compiles a file with no `main`, and so does the server: a
    /// library buffer is analysed, not reported as missing an entry point.
    #[test]
    fn a_buffer_with_no_main_still_analyzes() {
        let source = "fn double(a: int) -> int {\n    return a * 2;\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        let double = summary
            .own_functions()
            .find(|f| f.name == "double")
            .expect("the library's function is in the outline");
        // The entry point compiled for it is a call site, so its return type
        // is inferred and hover has a signature to show.
        assert_eq!(double.signatures, vec![String::from("(int) -> int")]);
    }

    /// A warning comes back with the summary, and nothing reaches stdout,
    /// which is the channel the server talks to the editor over.
    #[test]
    fn a_warning_comes_back_with_the_summary() {
        let source = "fn on_click(id) {\n    print(id);\n}\n\nfn main() {}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(outcome.diagnostic.is_none());
        assert!(outcome.summary.is_some());
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].code, "unannotated_host_parameter");
        assert_eq!(
            source.get(outcome.warnings[0].span.clone()),
            Some("on_click")
        );
    }

    /// A buffer written with a `match` used as a value, `|` alternatives,
    /// `=>` bodies and an open-ended slice analyses cleanly, and the outline
    /// shows the function whose body is an arrow.
    #[test]
    fn expression_forms_analyze() {
        let source = "enum Light { Stop, Slow, Go }\n\nfn word(l: Light) -> string => match l {\n    Stop | Slow => \"wait\",\n    Go => \"go\",\n};\n\nfn main() {\n    let xs = [1, 2, 3].map(fn(x) => x * 2);\n    print(word(Light::Go), xs[1..]);\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        assert!(summary.own_functions().any(|f| f.name == "word"));
    }

    /// The editor opens no library a `dylib` block names, so a buffer whose C
    /// library is not built yet analyses cleanly.
    #[test]
    fn a_dylib_is_bound_without_its_library() {
        let source = "dylib \"nosuch_lib_xyz\" {\n    int f(int);\n}\n\nfn g(x: int) -> int {\n    return nosuch_lib_xyz::f(x);\n}\n";
        let outcome = analyze_buffer(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
    }

    /// A call through a module's name resolves to that module's function, not
    /// to another of the same name, and completion offers the module's names
    /// behind it and the named items bare.
    #[test]
    fn a_namespaced_call_resolves_through_its_module() {
        let dir = std::env::temp_dir().join(format!("candela_lsp_ns_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        std::fs::write(dir.join("geo.cdl"), "fn area(w, h) { return w * h; }\n").unwrap();
        std::fs::write(
            dir.join("other.cdl"),
            "fn area(w, h) { return 0; }\nfn unit() { return 1; }\n",
        )
        .unwrap();
        let source = "import \"./geo.cdl\";\n\
                      import \"./other.cdl\" { unit };\n\
                      fn main() {\n\
                      \x20   print(geo::area(2, 3) + unit());\n\
                      }\n";
        let path = dir.join("buffer.cdl");
        let outcome = analyze_buffer(source, path.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
        let summary = outcome.summary.expect("a summary is produced");
        let area = summary
            .function("geo::area")
            .expect("geo::area is in scope");
        assert!(
            summary.sources[area.src_file as usize]
                .path
                .ends_with("geo.cdl"),
            "{:?}",
            summary.sources.iter().map(|s| &s.path).collect::<Vec<_>>()
        );
        let labels: Vec<&str> = summary.scope.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"geo::area"), "{labels:?}");
        assert!(labels.contains(&"unit"), "{labels:?}");
        assert!(!labels.contains(&"area"), "{labels:?}");
    }
}
