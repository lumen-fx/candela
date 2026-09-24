//! Static analysis over a candela buffer, built entirely on top of candela's
//! own lexer/parser/type-checker (`candela::check_only`). This module does not
//! reimplement any language frontend logic: it calls `check_only()`,
//! `collect_warnings` and `collect_diagnostic`, then walks the resulting
//! AST/symbol tables (`CompileOutput`) to answer the position-based questions
//! the LSP needs (hover, completion, go-to-definition, document symbols).
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
//! `candela::compiler::compiler_data::Struct` (unlike `Function`) is not
//! tagged with the source file that declared it. To decide whether a struct
//! declaration belongs to the buffer currently open in the editor (as
//! opposed to one pulled in via `import`), this module checks that the
//! struct's `name_span` indexes back to its own name in the buffer
//! text. That is a heuristic (documented on `struct_is_in_buffer`), not a
//! compiler-guaranteed invariant.

use candela::check_only;
use candela::compiler::compiler_data::{Function, Struct};
use candela::compiler::expr::METHOD_SEP;
use candela::compiler::expr::{Expr, Span};
use candela::compiler::imports::ImportResolver;
use candela::compiler::type_system::ANON_FN_PREFIX;
use candela::macros::MacroEnv;
use candela::{Diagnostic, TypeNames, collect_diagnostic, collect_warnings};

/// A function or struct declaration, with enough information to render a
/// document symbol / hover / go-to-definition target.
#[derive(Debug, Clone)]
pub struct FunctionSymbol {
    pub name: String,
    pub name_span: Span,
    /// Rendered parameter list, e.g. `["a: int", "b"]` (untyped params show
    /// only their name; their type is inferred per call site, see
    /// `signatures`).
    pub params: Vec<String>,
    /// Concrete `(params) -> return` signatures this function has
    /// been specialized for, sourced from `Function::return_type_cache`.
    /// Empty when the function has not been called anywhere in the compiled
    /// program yet, in which case its return type has not been
    /// inferred; this is surfaced as such, not guessed.
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
/// directory, exactly like the `candela` CLI would.
#[must_use]
pub fn analyze(text: &str, path: &str) -> AnalysisOutcome {
    let owned = text.to_owned();
    let path = path.to_owned();
    // Macros belong to whatever program embeds candela, and the server is not
    // that program: it has no expanders and cannot get them. An unregistered
    // macro therefore compiles as `null` here instead of failing, so a buffer
    // that uses its host's macros still type-checks and still reports its own
    // errors.
    let mut macros = MacroEnv::new();
    macros.allow_unknown(true);
    // Library imports resolve against the standard library beside the
    // toolchain, which is what a fresh resolver points at. The server reads no
    // project manifest, so an import of a package the project depends on is
    // reported as a file it cannot find.
    let resolver = ImportResolver::new();
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
            summary: Some(build_summary(&out, text)),
            warnings,
        },
    }
}

fn build_summary(out: &candela::compiler::CompileOutput, buffer_text: &str) -> ProgramSummary {
    let source_files = out.sources.iter().map(|s| s.filename.to_string()).collect();

    // The compiler's own renderer, with the tables this compile produced. A
    // type shows up in a tooltip named the way a diagnostic names it.
    let types = TypeNames {
        structs: &out.structs,
        enums: &out.enums,
    };

    let functions = out
        .functions
        .iter()
        .map(|f| function_symbol(f, types))
        .collect();

    let structs = out
        .structs
        .iter()
        .map(|s| struct_symbol(s, types, buffer_text))
        .collect();

    let refs = out.functions.iter().flat_map(collect_refs).collect();

    ProgramSummary {
        source_files,
        functions,
        structs,
        refs,
    }
}

fn function_symbol(f: &Function, types: TypeNames<'_>) -> FunctionSymbol {
    let params = f
        .args
        .iter()
        .map(|(name, ty)| match ty {
            Some(t) => format!("{name}: {}", types.of(t)),
            None => name.to_string(),
        })
        .collect();

    let signatures = f
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
/// by checking that slicing `buffer_text` at that byte range yields
/// the struct's own name. A struct declared in a different, imported file
/// would need a wildly coincidental span + name collision to pass this
/// check, which is an acceptable false-positive rate for an editor feature
/// (document symbols / hover), but it is not a hard guarantee.
fn struct_is_in_buffer(s: &Struct, buffer_text: &str) -> bool {
    let start = s.name_span.start as usize;
    let end = s.name_span.end as usize;
    buffer_text.get(start..end) == Some(s.name.as_str())
}

fn struct_symbol(s: &Struct, types: TypeNames<'_>, buffer_text: &str) -> StructSymbol {
    let fields = s
        .fields
        .iter()
        .map(|(name, ty, _span)| (name.to_string(), types.of(ty).to_string()))
        .collect();

    StructSymbol {
        name: s.name.to_string(),
        name_span: s.name_span,
        fields,
        src_file: struct_is_in_buffer(s, buffer_text).then_some(0),
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
        Expr::Struct(path, fields, span, _) => {
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
        Expr::Condition(cond, body, _, _)
        | Expr::InlineCondition(cond, body, _, _)
        | Expr::ElseIfBlock(cond, body, _) => {
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
        Expr::WhileBlock(cond, body, _) => {
            visit_expr(cond, src_file, out);
            for b in body.iter() {
                visit_expr(b, src_file, out);
            }
        }
        Expr::FunctionCall(args, path, span, _, _) => {
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
        Expr::ObjFunctionCall(obj, args, path, _obj_span, fn_span, _, _) => {
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
        Expr::FunctionDecl(_, _, body, _, _, _) => {
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
        | Expr::BitAnd(a, b, _, _)
        | Expr::BitOr(a, b, _, _)
        | Expr::BitXor(a, b, _, _)
        | Expr::Shl(a, b, _, _)
        | Expr::Shr(a, b, _, _)
        | Expr::Sup(a, b, _, _)
        | Expr::SupEq(a, b, _, _)
        | Expr::Inf(a, b, _, _)
        | Expr::InfEq(a, b, _, _)
        | Expr::BoolAnd(a, b, _, _)
        | Expr::BoolOr(a, b, _, _) => {
            visit_expr(a, src_file, out);
            visit_expr(b, src_file, out);
        }
        Expr::Eq(a, b, _, _) | Expr::NotEq(a, b, _, _) => {
            visit_expr(a, src_file, out);
            visit_expr(b, src_file, out);
        }
        Expr::BoolNeg(a, _, _) | Expr::Neg(a, _, _) | Expr::BitNot(a, _, _) => {
            visit_expr(a, src_file, out);
        }
        // An indirect call names no function, so it records no reference of
        // its own; the callee and the arguments still hold names to resolve.
        Expr::CallValue(callee, args, _, _) => {
            visit_expr(callee, src_file, out);
            for a in args.iter() {
                visit_expr(a, src_file, out);
            }
        }
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

impl ProgramSummary {
    /// Document symbols: top-level fn/struct declarations that live in the
    /// buffer itself (`src_file == 0`), not ones pulled in via `import`, and
    /// not the ones the compiler made for itself (see [`is_a_written_name`]).
    pub fn own_functions(&self) -> impl Iterator<Item = &FunctionSymbol> {
        self.functions
            .iter()
            .filter(|f| f.src_file == 0 && is_a_written_name(&f.name))
    }
    pub fn own_structs(&self) -> impl Iterator<Item = &StructSymbol> {
        self.structs.iter().filter(|s| s.src_file == Some(0))
    }

    /// The innermost call/struct-literal reference whose span contains
    /// `offset`, restricted to references that live in the buffer itself
    /// (`src_file == 0`); a reference from an imported file's body would
    /// have a span relative to that other file's text, which is
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
    /// given bare name, used to resolve a `RefSite::target_name`.
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

#[cfg(test)]
mod tests {
    use super::ANON_FN_PREFIX;
    use super::METHOD_SEP;
    use super::analyze;

    /// A buffer using a macro of the host it is written for still analyzes:
    /// the server has no expanders, so the macro compiles as `null` and the
    /// rest of the file is checked as usual.
    #[test]
    fn an_unknown_macro_is_not_a_diagnostic() {
        let source = "fn main() {\n    let markup = lmn!(<p>hi</p>);\n}\n";
        let outcome = analyze(source, "buffer.cdl");
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
        let outcome = analyze(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
    }

    /// The macro is skipped, not the errors around it.
    #[test]
    fn a_real_error_beside_a_macro_is_still_reported() {
        let source = "fn main() {\n    let markup = lmn!(<p/>);\n    let broken = 1 +\n}\n";
        let outcome = analyze(source, "buffer.cdl");
        assert!(outcome.diagnostic.is_some());
    }

    /// A fully annotated function nothing calls is compiled at its declared
    /// parameter types, so an error in its body is a diagnostic in the editor
    /// exactly as it is a failure on the command line.
    #[test]
    fn a_body_error_in_an_uncalled_function_is_reported() {
        let source = "fn broken(a: int) -> int {\n    return a + \"nope\";\n}\n\nfn main() {\n    print(1);\n}\n";
        let outcome = analyze(source, "buffer.cdl");
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
        let outcome = analyze(source, "buffer.cdl");
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
        let outcome = analyze(source, "buffer.cdl");
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
        assert_eq!(
            width.params,
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
        let outcome = analyze(source, "buffer.cdl");
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
        assert_eq!(
            holder.fields,
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
        let outcome = analyze(source, "buffer.cdl");
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
        assert_eq!(pick.params, vec![String::from("a: any")]);
    }

    /// The outline lists what the buffer declares. A closure the type checker
    /// hoists and a method lowered to its mangled free function are entries
    /// the compiler made, under names no one can write, and neither shows up.
    #[test]
    fn the_outline_leaves_out_the_names_the_compiler_made() {
        let source = "struct Point { x: int }\n                      impl Point { fn twice(self) -> int { return self.x * 2; } }\n                      fn apply(f, n) { return f(n); }\n                      fn main() {\n                          print(apply(fn(v) { return v + 1; }, Point{ x: 1 }.twice()));\n                      }\n";
        let outcome = analyze(source, "buffer.cdl");
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
        let outcome = analyze(source, "buffer.cdl");
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
        let outcome = analyze(source, "buffer.cdl");
        assert!(outcome.diagnostic.is_none());
        assert!(outcome.summary.is_some());
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(outcome.warnings[0].code, "unannotated_host_parameter");
        assert_eq!(
            source.get(outcome.warnings[0].span.clone()),
            Some("on_click")
        );
    }

    /// The editor opens no library a `dylib` block names, so a buffer whose C
    /// library is not built yet analyses cleanly.
    #[test]
    fn a_dylib_is_bound_without_its_library() {
        let source = "dylib \"nosuch_lib_xyz\" {\n    int f(int);\n}\n\nfn g(x: int) -> int {\n    return nosuch_lib_xyz::f(x);\n}\n";
        let outcome = analyze(source, "buffer.cdl");
        assert!(
            outcome.diagnostic.is_none(),
            "{:?}",
            outcome.diagnostic.map(|d| d.message)
        );
    }
}
