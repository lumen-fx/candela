//! Static analysis over a candela buffer, built entirely on top of candela's
//! own lexer/parser/type-checker (`candela::compiler::compile`). This module
//! does not reimplement any language frontend logic: it calls `compile()`
//! and `collect_diagnostic`, then walks the resulting AST/symbol tables
//! (`CompileOutput`) to answer the position-based questions the LSP needs
//! (hover, completion, go-to-definition, document symbols).
//!
//! `compile()` (as opposed to `candela::Engine::compile`) parses,
//! type-checks, and code-generates a program WITHOUT running `main`, so
//! running this on every keystroke has no script side effects (no `print`
//! output, no host calls, no infinite loops from the user's own code).
//!
//! candela's error funnel (parser + compiler + runtime) is fatal-on-first-
//! error: `collect_diagnostic` yields at most ONE `Diagnostic` per call, not
//! a full list. That is a property of the underlying compiler, not a
//! limitation added here; see the crate README for what this means for
//! `publishDiagnostics`.
//!
//! `candela::compiler::compiler_data::Struct` (unlike `Function`) is not
//! tagged with the source file that declared it. To decide whether a struct
//! declaration belongs to the buffer currently open in the editor (as
//! opposed to one pulled in via `import`), this module checks that the
//! struct's `name_span` actually indexes back to its own name in the buffer
//! text. That is a heuristic (documented on `struct_is_in_buffer`), not a
//! compiler-guaranteed invariant.

use candela::compiler::compile;
use candela::compiler::compiler_data::{Function, Struct};
use candela::compiler::expr::{Expr, Span};
use candela::compiler::type_system::DataType;
use candela::{Diagnostic, collect_diagnostic};

/// A function or struct declaration, with enough information to render a
/// document symbol / hover / go-to-definition target.
#[derive(Debug, Clone)]
pub struct FunctionSymbol {
    pub name: String,
    pub name_span: Span,
    /// Rendered parameter list, e.g. `["a: int", "b"]` (untyped params show
    /// only their name -- their type is inferred per call site, see
    /// `signatures`).
    pub params: Vec<String>,
    /// Concrete `(params) -> return` signatures this function has actually
    /// been specialized for, sourced from `Function::return_type_cache`.
    /// Empty when the function has not been called anywhere in the compiled
    /// program yet, in which case its return type has genuinely not been
    /// inferred -- this is surfaced as such, not guessed.
    pub signatures: Vec<String>,
    /// Index into `ProgramSummary::source_files`.
    pub src_file: u16,
}

#[derive(Debug, Clone)]
pub struct StructSymbol {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<(String, String)>,
    /// Best-effort; see the heuristic note on `struct_is_in_buffer`. `None`
    /// when the heuristic could not place it in any known source file.
    pub src_file: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Call,
    StructLiteral,
}

/// A use-site of a function or struct found while walking the compiled
/// function bodies: `span` covers just the callee name/path (not the whole
/// call), so hover/go-to-definition can match a click precisely.
#[derive(Debug, Clone)]
pub struct RefSite {
    pub span: Span,
    pub kind: RefKind,
    /// Bare (last path segment) name; `namespace::name` qualification is not
    /// resolved, see the crate README's "known simplifications".
    pub target_name: String,
    pub src_file: u16,
}

/// Everything this module extracts from a successful `compile()`, in plain
/// owned data (no `Rc`, no borrows) so it can be produced synchronously and
/// then freely moved across an `.await` point in the LSP's async handlers.
#[derive(Debug, Clone, Default)]
pub struct ProgramSummary {
    /// Indexed by candela's internal `src_file` id. Index 0 is always the
    /// buffer this summary was built from.
    pub source_files: Vec<String>,
    pub functions: Vec<FunctionSymbol>,
    pub structs: Vec<StructSymbol>,
    pub refs: Vec<RefSite>,
}

/// The result of analyzing one buffer: a `Diagnostic` on failure (parse or
/// type error), or a `ProgramSummary` on success. Never both -- candela's
/// compiler does not currently support returning partial results alongside
/// an error.
#[derive(Debug, Clone, Default)]
pub struct AnalysisOutcome {
    pub diagnostic: Option<Diagnostic>,
    pub summary: Option<ProgramSummary>,
}

/// Runs candela's compiler (parse + type-check + codegen, no execution) over
/// `text` as if it were the file at `path`, and extracts a `ProgramSummary`
/// or the first `Diagnostic` produced.
///
/// `path` should be the buffer's real filesystem path (from the LSP URI) so
/// that any `import "..."` statements resolve relative to the right
/// directory, exactly like the `candela` CLI would.
#[must_use]
pub fn analyze(text: &str, path: &str) -> AnalysisOutcome {
    let owned = text.to_owned();
    let path = path.to_owned();
    match collect_diagnostic(move || compile(owned, &path, false)) {
        Err(diagnostic) => AnalysisOutcome {
            diagnostic: Some(diagnostic),
            summary: None,
        },
        Ok(out) => AnalysisOutcome {
            diagnostic: None,
            summary: Some(build_summary(&out, text)),
        },
    }
}

fn build_summary(out: &candela::compiler::CompileOutput, buffer_text: &str) -> ProgramSummary {
    let source_files = out.sources.iter().map(|s| s.filename.to_string()).collect();

    let functions = out
        .functions
        .iter()
        .map(|f| function_symbol(f, &out.structs))
        .collect();

    let structs = out
        .structs
        .iter()
        .map(|s| struct_symbol(s, &out.structs, buffer_text))
        .collect();

    let refs = out.functions.iter().flat_map(collect_refs).collect();

    ProgramSummary {
        source_files,
        functions,
        structs,
        refs,
    }
}

fn function_symbol(f: &Function, structs: &[Struct]) -> FunctionSymbol {
    let params = f
        .args
        .iter()
        .map(|(name, ty)| match ty {
            Some(t) => format!("{name}: {}", format_datatype(t, structs)),
            None => name.to_string(),
        })
        .collect();

    let signatures = f
        .return_type_cache
        .iter()
        .map(|(args, ret)| {
            let args_s = args
                .iter()
                .map(|t| format_datatype(t, structs))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({args_s}) -> {}", format_datatype(ret, structs))
        })
        .collect();

    FunctionSymbol {
        name: f.name.to_string(),
        name_span: f.name_span,
        params,
        signatures,
        src_file: f.src_file,
    }
}

/// `Struct` carries no source-file id (unlike `Function`), so this
/// approximates "does this struct's `name_span` land inside `buffer_text`"
/// by checking that slicing `buffer_text` at that byte range actually yields
/// the struct's own name. A struct declared in a different, imported file
/// would need a wildly coincidental span + name collision to pass this
/// check, which is an acceptable false-positive rate for an editor feature
/// (document symbols / hover), but it is not a hard guarantee.
fn struct_is_in_buffer(s: &Struct, buffer_text: &str) -> bool {
    let start = s.name_span.start as usize;
    let end = s.name_span.end as usize;
    buffer_text.get(start..end) == Some(s.name.as_str())
}

fn struct_symbol(s: &Struct, structs: &[Struct], buffer_text: &str) -> StructSymbol {
    let fields = s
        .fields
        .iter()
        .map(|(name, ty, _span)| (name.to_string(), format_datatype(ty, structs)))
        .collect();

    StructSymbol {
        name: s.name.to_string(),
        name_span: s.name_span,
        fields,
        src_file: struct_is_in_buffer(s, buffer_text).then_some(0),
    }
}

/// Renders a `DataType` for humans, resolving `DataType::Struct(id)` to its
/// declared name (plain `Display` on `DataType` only ever prints the literal
/// word "struct", since it has no access to the struct table).
fn format_datatype(dt: &DataType, structs: &[Struct]) -> String {
    match dt {
        DataType::Float => "float".to_owned(),
        DataType::Int => "int".to_owned(),
        DataType::Bool => "bool".to_owned(),
        DataType::String => "string".to_owned(),
        DataType::Null => "null".to_owned(),
        DataType::Unknown => "unknown".to_owned(),
        DataType::Fn(_) => "function".to_owned(),
        DataType::Array(Some(inner)) => format!("{}[]", format_datatype(inner, structs)),
        DataType::Array(None) => "unknown[]".to_owned(),
        DataType::Union(types) => types
            .iter()
            .map(|t| format_datatype(t, structs))
            .collect::<Vec<_>>()
            .join(" | "),
        DataType::Struct(id) => structs
            .get(*id as usize)
            .map_or_else(|| "struct".to_owned(), |s| s.name.to_string()),
        DataType::Enum(_) => "enum".to_owned(),
        DataType::Map(kv) => format!(
            "{{{}: {}}}",
            kv.0.as_ref()
                .map_or_else(|| "unknown".to_owned(), |t| format_datatype(t, structs)),
            kv.1.as_ref()
                .map_or_else(|| "unknown".to_owned(), |t| format_datatype(t, structs)),
        ),
    }
}

/// Walks every expression reachable from `f.code`, collecting a `RefSite`
/// for each function call and struct literal so hover / go-to-definition can
/// later match a cursor position against them without re-walking the AST.
fn collect_refs(f: &Function) -> Vec<RefSite> {
    let mut out = Vec::new();
    for e in f.code.iter() {
        visit_expr(e, f.src_file, &mut out);
    }
    out
}

fn visit_expr(e: &Expr, src_file: u16, out: &mut Vec<RefSite>) {
    match e {
        Expr::Float(_)
        | Expr::Int(_)
        | Expr::Bool(_)
        | Expr::Null
        | Expr::String(_)
        | Expr::Var(_, _)
        | Expr::Break
        | Expr::Continue
        | Expr::ImportDylib(..)
        | Expr::HostBlock(..)
        | Expr::ImportFile(..)
        | Expr::StructDeclare(..)
        | Expr::EnumDeclare(..)
        | Expr::NamespacedRef(..) => {}

        Expr::Match(scrutinee, arms, wildcard, _) => {
            visit_expr(scrutinee, src_file, out);
            for (pat, body) in arms.iter() {
                visit_expr(pat, src_file, out);
                for b in body.iter() {
                    visit_expr(b, src_file, out);
                }
            }
            if let Some(w) = wildcard {
                for b in w.iter() {
                    visit_expr(b, src_file, out);
                }
            }
        }

        Expr::Array(items, _) => {
            for it in items.iter() {
                visit_expr(it, src_file, out);
            }
        }
        Expr::Map(pairs, _) => {
            for (k, _, v, _) in pairs.iter() {
                visit_expr(k, src_file, out);
                visit_expr(v, src_file, out);
            }
        }
        Expr::Struct(path, fields, span) => {
            for (_, val, _, _) in fields.iter() {
                visit_expr(val, src_file, out);
            }
            if let Some(name) = path.last() {
                out.push(RefSite {
                    span: *span,
                    kind: RefKind::StructLiteral,
                    target_name: name.to_string(),
                    src_file,
                });
            }
        }
        Expr::GetStructField(obj, _, _, _) => visit_expr(obj, src_file, out),
        Expr::SetStructField(obj, _, val, _, _, _) => {
            visit_expr(obj, src_file, out);
            visit_expr(val, src_file, out);
        }
        Expr::VarDeclare(_, val) => visit_expr(val, src_file, out),
        Expr::VarAssign(_, val, _) => visit_expr(val, src_file, out),
        Expr::Condition(cond, body, _)
        | Expr::InlineCondition(cond, body, _)
        | Expr::ElseIfBlock(cond, body) => {
            visit_expr(cond, src_file, out);
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::ElseBlock(body)
        | Expr::AnonymousFunction(_, body, _)
        | Expr::EvalBlock(body)
        | Expr::LoopBlock(body) => {
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::WhileBlock(cond, body) => {
            visit_expr(cond, src_file, out);
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::FunctionCall(args, path, span, _) => {
            for a in args.iter() {
                visit_expr(a, src_file, out);
            }
            if let Some(name) = path.last() {
                out.push(RefSite {
                    span: *span,
                    kind: RefKind::Call,
                    target_name: name.to_string(),
                    src_file,
                });
            }
        }
        Expr::ObjFunctionCall(obj, args, path, _obj_span, fn_span, _) => {
            visit_expr(obj, src_file, out);
            for a in args.iter() {
                visit_expr(a, src_file, out);
            }
            if let Some(name) = path.last() {
                out.push(RefSite {
                    span: *fn_span,
                    kind: RefKind::Call,
                    target_name: name.to_string(),
                    src_file,
                });
            }
        }
        Expr::FunctionDecl(_, _, body, _) => {
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::ReturnVal(opt) => {
            if let Some(v) = opt.as_ref() {
                visit_expr(v, src_file, out);
            }
        }
        Expr::ArrayGetIndex(base, idx, _) => {
            visit_expr(base, src_file, out);
            visit_expr(idx, src_file, out);
        }
        Expr::ArrayGetSlice(base, s, e, _) => {
            visit_expr(base, src_file, out);
            visit_expr(s, src_file, out);
            visit_expr(e, src_file, out);
        }
        Expr::ArrayModify(base, idx, val, _, _) => {
            visit_expr(base, src_file, out);
            visit_expr(idx, src_file, out);
            visit_expr(val, src_file, out);
        }
        Expr::ForLoop(_, arr, body, _) => {
            visit_expr(arr, src_file, out);
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::IntForLoop(_, from, to, body, _, _) => {
            visit_expr(from, src_file, out);
            visit_expr(to, src_file, out);
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::TryCatchBlock(try_body, _, catch_body) => {
            for b in try_body.iter() {
                visit_expr(b, src_file, out);
            }
            for b in catch_body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::Mul(a, b, _, _)
        | Expr::Div(a, b, _, _)
        | Expr::Add(a, b, _, _)
        | Expr::Sub(a, b, _, _)
        | Expr::Mod(a, b, _, _)
        | Expr::Pow(a, b, _, _)
        | Expr::Sup(a, b, _, _)
        | Expr::SupEq(a, b, _, _)
        | Expr::Inf(a, b, _, _)
        | Expr::InfEq(a, b, _, _)
        | Expr::BoolAnd(a, b, _, _)
        | Expr::BoolOr(a, b, _, _) => {
            visit_expr(a, src_file, out);
            visit_expr(b, src_file, out);
        }
        Expr::Eq(a, b) | Expr::NotEq(a, b) => {
            visit_expr(a, src_file, out);
            visit_expr(b, src_file, out);
        }
        Expr::BoolNeg(a, _, _) | Expr::Neg(a, _, _) => visit_expr(a, src_file, out),
    }
}

impl ProgramSummary {
    /// Document symbols: top-level fn/struct declarations that live in the
    /// buffer itself (`src_file == 0`), not ones pulled in via `import`.
    pub fn own_functions(&self) -> impl Iterator<Item = &FunctionSymbol> {
        self.functions.iter().filter(|f| f.src_file == 0)
    }
    pub fn own_structs(&self) -> impl Iterator<Item = &StructSymbol> {
        self.structs.iter().filter(|s| s.src_file == Some(0))
    }

    /// The innermost call/struct-literal reference whose span contains
    /// `offset`, restricted to references that live in the buffer itself
    /// (`src_file == 0`) -- a reference from an imported file's body would
    /// have a span relative to that *other* file's text, which is
    /// meaningless against this buffer's offsets.
    #[must_use]
    pub fn reference_at(&self, offset: u32) -> Option<&RefSite> {
        self.refs
            .iter()
            .filter(|r| r.src_file == 0 && r.span.start <= offset && offset <= r.span.end)
            // Prefer the tightest (shortest) enclosing span.
            .min_by_key(|r| r.span.end - r.span.start)
    }

    /// The function declaration (if any) whose name_span contains `offset`,
    /// restricted to this buffer.
    #[must_use]
    pub fn own_function_decl_at(&self, offset: u32) -> Option<&FunctionSymbol> {
        self.own_functions()
            .find(|f| f.name_span.start <= offset && offset <= f.name_span.end)
    }

    /// The struct declaration (if any) whose name_span contains `offset`,
    /// restricted to this buffer.
    #[must_use]
    pub fn own_struct_decl_at(&self, offset: u32) -> Option<&StructSymbol> {
        self.own_structs()
            .find(|s| s.name_span.start <= offset && offset <= s.name_span.end)
    }

    /// All function declarations (from this buffer or an import) with the
    /// given bare name -- used to resolve a `RefSite::target_name`.
    pub fn functions_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a FunctionSymbol> {
        self.functions.iter().filter(move |f| f.name == name)
    }
    pub fn structs_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a StructSymbol> {
        self.structs.iter().filter(move |s| s.name == name)
    }
}
