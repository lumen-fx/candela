// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::BOLD;
use crate::RED;
use crate::RESET;
use crate::cfg;
use crate::cfg::Cfg;
use crate::compiler::compiler_data::Source;
use crate::compiler::expr::OPERATOR_SYMBOLS;
use crate::compiler::expr::{Expr, Span, var_assign};
use crate::compiler::type_system::FnTypeExpr;
use crate::compiler::type_system::GenericType;
use crate::compiler::type_system::ImplTemplate;
use crate::compiler::type_system::TypeExpr;
use crate::compiler::type_system::TypeParams;
use crate::errors::BLUE;
use crate::errors::blue;
use crate::errors::crash;
use crate::macros;
use crate::macros::Expansion;
use crate::macros::MAX_EXPANSION_DEPTH;
use ariadne::Color;
use ariadne::Label;
use ariadne::Report;
use ariadne::ReportKind;
use blocks::parse_condition_block;
use blocks::parse_enum_declare;
use blocks::parse_eval_block;
use blocks::parse_for_loop;
use blocks::parse_function;
use blocks::parse_impl_block;
use blocks::parse_loop_block;
use blocks::parse_match;
use blocks::parse_struct_declare;
use blocks::parse_try_catch_block;
use blocks::parse_while_block;
use lexer::LexErr;
use lexer::MacroToken;
use lexer::parse_string;
use logos::SpannedIter;
use parser_expr::add_op;
use parser_expr::parse_expr;
use smol_strc::SmolStr;
use std::hint::{cold_path, unreachable_unchecked};
use std::io::Write;
use std::iter::Peekable;

use lexer::Token;
use logos::Logos;

mod blocks;
mod lexer;
mod parser_expr;
mod term;

type TokenIter<'a> = Peekable<SpannedIter<'a, Token<'a>>>;

struct ParserCtx<'a> {
    src: &'a Source,
    /// The configuration `@cfg(...)` attributes are read against.
    cfg: &'a Cfg,
}

struct Parser<'a> {
    input: TokenIter<'a>,
    ctx: ParserCtx<'a>,
    last_token_end: usize,
    /// Set while parsing the source a macro expanded to. That source is not in
    /// the file, so every span the sub-parse produces is the span of the macro
    /// invocation instead: expressions spliced into the program point at the
    /// macro site, and so does any error raised while parsing them.
    span_override: Option<Span>,
    /// How many terms, blocks and types enclose the one being parsed. Each
    /// level is one recursive descent frame, so this is what bounds the stack
    /// the parser (and the compiler walking the tree it builds) can use.
    nesting: u32,
    /// The second `>` of a `>>` that closed one level of a nested type-argument
    /// list, waiting to be read as a token of its own. `Box<Box<int>>` ends in
    /// one token and closes two lists, so the inner list takes the first half
    /// and leaves the second here for the list around it.
    pending_gt: Option<Span>,
    /// How many `@cfg` attributes that evaluated false enclose what is being
    /// parsed. Code under one is parsed, so its syntax is checked on every
    /// configuration, and then dropped; a macro inside it is not expanded,
    /// since the expander may exist only where the code is compiled.
    inactive: u32,
}

/// The deepest that terms, blocks and types may nest.
///
/// A level is one term inside another (`(`, `[`, `-`, a call argument, a
/// struct field), one block inside another, or one type inside another; a
/// macro expansion parsed inside an expression counts the levels around the
/// invocation. The json parser caps documents at the same depth.
pub const MAX_NESTING_DEPTH: u32 = 128;

#[derive(Clone)]
enum ParserErr<'a> {
    UnexpectedEOF,
    UnknownToken,
    /// An integer literal outside the range `int` holds.
    IntLiteralOutOfRange,
    /// A float literal outside the range `float` holds.
    FloatLiteralOutOfRange,
    /// A number whose exponent marker carries no digits.
    FloatExponentMissingDigits,
    /// (expected, received)
    UnexpectedToken(Token<'a>, Token<'a>, &'static str),
    /// (expected, received)
    UnexpectedTokenStr(&'static str, Token<'a>, &'static str),
    ArrayElementsMissingComma,
    InlineConditionNoElseBlock,
    DivisionByZero,
    ModuloByZero,
    IntegerNegativeExponent,
    /// A literal shift by a negative count, or by 64 or more, which `int` has
    /// no result for. Carries the count that was written.
    ShiftCountOutOfRange(i64),
    ArgumentsMissingCommaSeparator,
    TryBlockNoCatch,
    MatchBlockNoNonWildcardArm,
    MatchBlockZeroArms,
    /// A `fn` declaration written inside a block rather than at the top level
    /// of a file.
    NestedFunctionDeclaration,
    /// The removed `import std::string;` form; carries the path segments
    /// joined with `/` so the error suggests the exact replacement.
    LegacyNamespacedImport(String),
    ImportPathBadExtension,
    /// `name!(` whose region the file ends before closing.
    UnterminatedMacroRegion(&'a str),
    /// `name!(...)` with no expander registered for `name`.
    UnknownMacro(&'a str),
    /// (macro name, what the expander said)
    MacroExpansionFailed(&'a str, String),
    /// The expander returned more than one expression.
    MacroExpansionTrailingTokens(&'a str),
    /// Macros expanded into one another past [`MAX_EXPANSION_DEPTH`].
    MacroRecursionLimit(&'a str),
    /// Terms, blocks or types nested past [`MAX_NESTING_DEPTH`].
    NestingTooDeep,
    /// An operator token after `fn` in an `impl` block that a type cannot
    /// define. Carries the symbol that was written.
    OperatorMethodUnknown(&'static str),
    /// `@name` for a name that is not an attribute.
    UnknownAttribute(&'a str),
    /// An attribute with nothing after it to apply to.
    AttributeWithoutItem,
}

impl ParserErr<'_> {
    /// Stable, machine-readable identifier for this parser error, mirroring
    /// `ErrType::kind` on the runtime side.
    const fn kind(&self) -> &'static str {
        match self {
            ParserErr::UnexpectedEOF => "unexpected_eof",
            ParserErr::UnknownToken => "unknown_token",
            ParserErr::IntLiteralOutOfRange => "int_literal_out_of_range",
            ParserErr::FloatLiteralOutOfRange => "float_literal_out_of_range",
            ParserErr::FloatExponentMissingDigits => "float_exponent_missing_digits",
            ParserErr::UnexpectedToken(..) | ParserErr::UnexpectedTokenStr(..) => {
                "unexpected_token"
            }
            ParserErr::ArrayElementsMissingComma => "array_elements_missing_comma",
            ParserErr::InlineConditionNoElseBlock => "inline_condition_no_else_block",
            ParserErr::DivisionByZero => "division_by_zero",
            ParserErr::ModuloByZero => "modulo_by_zero",
            ParserErr::IntegerNegativeExponent => "integer_negative_exponent",
            ParserErr::ShiftCountOutOfRange(_) => "shift_count_out_of_range",
            ParserErr::ArgumentsMissingCommaSeparator => "arguments_missing_comma_separator",
            ParserErr::TryBlockNoCatch => "try_block_no_catch",
            ParserErr::MatchBlockNoNonWildcardArm => "match_block_no_non_wildcard_arm",
            ParserErr::MatchBlockZeroArms => "match_block_zero_arms",
            ParserErr::NestedFunctionDeclaration => "nested_function_declaration",
            ParserErr::LegacyNamespacedImport(_) => "legacy_namespaced_import",
            ParserErr::ImportPathBadExtension => "import_path_bad_extension",
            ParserErr::UnterminatedMacroRegion(_) => "unterminated_macro_region",
            ParserErr::UnknownMacro(_) => "unknown_macro",
            ParserErr::MacroExpansionFailed(..) => "macro_expansion_failed",
            ParserErr::MacroExpansionTrailingTokens(_) => "macro_expansion_trailing_tokens",
            ParserErr::MacroRecursionLimit(_) => "macro_recursion_limit",
            ParserErr::NestingTooDeep => "nesting_too_deep",
            ParserErr::OperatorMethodUnknown(_) => "operator_method_unknown",
            ParserErr::UnknownAttribute(_) => "unknown_attribute",
            ParserErr::AttributeWithoutItem => "attribute_without_item",
        }
    }
}

/// The parser error a refused token earns, so a literal the lexer read and
/// rejected reports what was wrong with it.
const fn lex_err(e: LexErr) -> ParserErr<'static> {
    match e {
        LexErr::UnknownToken => ParserErr::UnknownToken,
        LexErr::IntOutOfRange => ParserErr::IntLiteralOutOfRange,
        LexErr::FloatOutOfRange => ParserErr::FloatLiteralOutOfRange,
        LexErr::FloatExponentMissingDigits => ParserErr::FloatExponentMissingDigits,
    }
}

#[cold]
#[inline(never)]
fn throw_parser_error(src: &Source, Span { start, end }: Span, t: ParserErr) -> ! {
    let kind = t.kind();
    let err_message = match t {
        ParserErr::UnexpectedEOF => "Unexpected EOF",
        ParserErr::UnknownToken => "Unknown token",
        ParserErr::IntLiteralOutOfRange => &format!(
            "This number does not fit an {BLUE}{BOLD}int{RESET}, which holds {} to {}. A wider value goes in a {BLUE}{BOLD}float{RESET}",
            i64::MIN,
            i64::MAX
        ),
        ParserErr::FloatLiteralOutOfRange => &format!(
            "This number does not fit a {BLUE}{BOLD}float{RESET}, which holds {:e} to {:e}",
            f64::MIN,
            f64::MAX
        ),
        ParserErr::FloatExponentMissingDigits => {
            "This exponent has no digits after it. Write the power, as in 1e3 or 2.5e-1"
        }
        ParserErr::UnexpectedToken(expected, received, msg) => &format_args!(
            "Expected {BLUE}{BOLD}{expected}{RESET}, but got {RED}{BOLD}{received}{RESET}. {msg}"
        )
        .to_string(),
        ParserErr::UnexpectedTokenStr(expected, received, msg) => &format_args!(
            "Expected {BLUE}{BOLD}{expected}{RESET}, but got {RED}{BOLD}{received}{RESET}. {msg}"
        )
        .to_string(),
        ParserErr::ArrayElementsMissingComma => "Array elements must be separated by a comma",
        ParserErr::InlineConditionNoElseBlock => "Inline conditions must have an else block",
        ParserErr::DivisionByZero => "Division by zero",
        ParserErr::ModuloByZero => "Modulo by zero",
        ParserErr::IntegerNegativeExponent => "Integers cannot be raised to a negative exponent",
        ParserErr::ShiftCountOutOfRange(count) => &format_args!(
            "An {BLUE}{BOLD}int{RESET} is 64 bits wide, so it cannot be shifted by {RED}{BOLD}{count}{RESET}. The count must be between 0 and 63"
        )
        .to_string(),
        ParserErr::ArgumentsMissingCommaSeparator => "Arguments must be separated by a comma",
        ParserErr::TryBlockNoCatch => {
            "A {BLUE}{BOLD}try{RESET} block must have at least one {BLUE}{BOLD}catch{RESET} block"
        }
        ParserErr::MatchBlockNoNonWildcardArm => {
            "{BLUE}{BOLD}Match blocks{RESET} must have {BOLD}at least one non-wildcard arm{RESET}"
        }
        ParserErr::MatchBlockZeroArms => {
            "{BLUE}{BOLD}Match blocks{RESET} must have {BOLD}at least one arm{RESET}"
        }
        ParserErr::NestedFunctionDeclaration => {
            "Functions declare at the top level of a file, not inside a block. Move this declaration out of the enclosing block"
        }
        ParserErr::LegacyNamespacedImport(path) => &format!(
            "This import form was removed. Write {BLUE}{BOLD}import \"{path}\";{RESET} instead: a quoted path with no extension imports the library from the shipped library directory"
        ),
        ParserErr::ImportPathBadExtension => &format!(
            "An import path either ends in {BLUE}{BOLD}.cdl{RESET} (a file import) or has no extension (a library import from the shipped library directory)"
        ),
        ParserErr::UnterminatedMacroRegion(name) => &format!(
            "This {BLUE}{BOLD}{name}!{RESET} region is never closed: the file ends before its ')'"
        ),
        ParserErr::UnknownMacro(name) => &format!(
            "No macro named {RED}{BOLD}{name}!{RESET} is registered. Macros come from the program embedding candela, which decides what each one means"
        ),
        ParserErr::MacroExpansionFailed(name, message) => {
            &format!("The {BLUE}{BOLD}{name}!{RESET} macro rejected this region: {message}")
        }
        ParserErr::MacroExpansionTrailingTokens(name) => &format!(
            "The {BLUE}{BOLD}{name}!{RESET} macro expanded to more than one expression. A macro stands where an expression stands, so it expands to exactly one"
        ),
        ParserErr::MacroRecursionLimit(name) => &format!(
            "Expanding this macro reached {RED}{BOLD}{name}!{RESET} more than {MAX_EXPANSION_DEPTH} macros deep. An expansion may use another macro, but the chain has to end"
        ),
        ParserErr::NestingTooDeep => &format!(
            "This is nested more than {MAX_NESTING_DEPTH} levels deep. Move the inner part into a variable or a function"
        ),
        ParserErr::OperatorMethodUnknown(symbol) => &format!(
            "A type does not define {RED}{BOLD}{symbol}{RESET}. The operators it defines are {BLUE}{BOLD}{OPERATOR_SYMBOLS}{RESET}{}",
            match symbol {
                "!=" => ", and != comes from the == it defines",
                ">" => ", and > comes from the < it defines",
                ">=" => ", and >= comes from the <= it defines",
                _ => "",
            }
        ),
        ParserErr::UnknownAttribute(name) => &format!(
            "There is no attribute named {RED}{BOLD}{name}{RESET}. The attribute candela has is {BLUE}{BOLD}@cfg(...){RESET}"
        ),
        ParserErr::AttributeWithoutItem => {
            "An attribute applies to the declaration or statement after it, and nothing follows this one"
        }
    };
    if crate::errors::diagnostics_enabled() {
        crate::errors::emit_diagnostic(
            src.filename.as_str(),
            (start as usize)..(end as usize),
            crate::errors::strip_ansi(err_message),
            kind,
        );
    }
    let mut out = candela_vm::captured_output::stderr();
    let _ = writeln!(out, "{RED}CANDELA ERROR{RESET}");
    let report = Report::build(
        ReportKind::Error,
        (src.filename.as_str(), (start as usize)..(end as usize)),
    )
    .with_label(
        Label::new((src.filename.as_str(), (start as usize)..(end as usize)))
            .with_message(err_message)
            .with_color(Color::Red),
    )
    .finish();

    // A stderr that refuses the report costs the report. Raising here would
    // replace the error in the source with an unrelated one about the stream.
    let _ = report.write(
        (
            src.filename.as_str(),
            ariadne::Source::from(src.contents.as_str()),
        ),
        &mut out,
    );

    #[cfg(debug_assertions)]
    panic!();

    #[cfg(not(any(debug_assertions, target_arch = "wasm32", feature = "embed")))]
    std::process::exit(1);

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen::throw_str("candela_error");

    #[cfg(all(feature = "embed", not(debug_assertions)))]
    panic!();
}

impl<'a> Parser<'a> {
    /// The span the file sees for a token this parser just read; see
    /// [`Parser::span_override`].
    #[inline(always)]
    const fn placed(&self, span: Span) -> Span {
        match self.span_override {
            Some(macro_span) => macro_span,
            None => span,
        }
    }
    #[inline(always)]
    fn eof_span(&self) -> Span {
        let end = self.ctx.src.contents.len();
        self.placed((end, end).into())
    }
    #[cold]
    #[inline(never)]
    fn error(&self, span: Span, error: ParserErr) -> ! {
        throw_parser_error(self.ctx.src, span, error)
    }
    /// Opens one nesting level for the construct starting at `span`, or raises
    /// [`ParserErr::NestingTooDeep`] at it. Pair with [`Parser::leave`].
    #[inline(always)]
    fn enter(&mut self, span: Span) {
        if self.nesting >= MAX_NESTING_DEPTH {
            cold_path();
            self.error(span, ParserErr::NestingTooDeep);
        }
        self.nesting += 1;
    }
    #[inline(always)]
    const fn leave(&mut self) {
        self.nesting -= 1;
    }
    #[inline(always)]
    fn next_token(&mut self) -> (Token<'a>, Span) {
        if let Some(span) = self.pending_gt.take() {
            let span = self.placed(span);
            self.last_token_end = span.end as usize;
            return (Token::OpSup, span);
        }
        let t = self.input.next().unwrap_or_else(
            #[cold]
            || {
                self.error(self.eof_span(), ParserErr::UnexpectedEOF);
            },
        );
        let span = self.placed((t.1.start, t.1.end).into());
        self.last_token_end = span.end as usize;
        (
            t.0.unwrap_or_else(
                #[cold]
                |e| self.error(span, lex_err(e)),
            ),
            span,
        )
    }
    #[inline(always)]
    fn peek_token(&mut self) -> Token<'a> {
        if self.pending_gt.is_some() {
            return Token::OpSup;
        }
        let Some((t, start, end)) = self
            .input
            .peek()
            .map(|(t, span)| (*t, span.start, span.end))
        else {
            self.error(self.eof_span(), ParserErr::UnexpectedEOF);
        };
        t.unwrap_or_else(
            #[cold]
            |e| self.error(self.placed((start, end).into()), lex_err(e)),
        )
    }
    #[inline(always)]
    fn peek_token_span(&mut self) -> Span {
        if let Some(span) = self.pending_gt {
            return self.placed(span);
        }
        let Some((_, start, end)) = self
            .input
            .peek()
            .map(|(t, span)| (*t, span.start, span.end))
        else {
            self.error(self.eof_span(), ParserErr::UnexpectedEOF);
        };
        self.placed(Span {
            start: start as u32,
            end: end as u32,
        })
    }
    #[inline(always)]
    fn peek_token_opt(&mut self) -> Option<Token<'a>> {
        if self.pending_gt.is_some() {
            return Some(Token::OpSup);
        }
        let (t, start, end) = self
            .input
            .peek()
            .map(|(t, span)| (*t, span.start, span.end))?;
        Some(t.unwrap_or_else(
            #[cold]
            |e| self.error(self.placed((start, end).into()), lex_err(e)),
        ))
    }
    #[inline(always)]
    fn peek_token_opt_span(&mut self) -> Option<Span> {
        if let Some(span) = self.pending_gt {
            return Some(self.placed(span));
        }
        let span = self
            .input
            .peek()
            .map(|x| (x.1.start as u32, x.1.end as u32).into())?;
        Some(self.placed(span))
    }
    #[inline(always)]
    fn next_token_expect(&mut self, expected: Token, msg: &'static str) -> Span {
        let (next_token, span) = self.next_token();
        if next_token != expected {
            self.error(span, ParserErr::UnexpectedToken(expected, next_token, msg));
        }
        span
    }
    /// Reads the `>` that closes a type-argument or type-parameter list.
    ///
    /// The two closing `>` of `Box<Box<int>>` lex as one `>>`, the shift
    /// operator, because the lexer reads the longest token it can and knows
    /// nothing of where it stands. The inner list takes the first half here and
    /// leaves the second for the list around it.
    fn next_type_list_close(&mut self, msg: &'static str) -> Span {
        if self.peek_token_opt() == Some(Token::OpShr) {
            let (_, span) = self.next_token();
            let middle = span.start + 1;
            self.pending_gt = Some((middle, span.end).into());
            self.last_token_end = middle as usize;
            return (span.start, middle).into();
        }
        self.next_token_expect(Token::OpSup, msg)
    }
    #[inline(always)]
    fn next_token_expect_closer(
        &mut self,
        opener: Token,
        opener_span: Span,
        expected_closer: Token,
    ) -> u32 {
        if let Some(t) = self.peek_token_opt() {
            if t == expected_closer {
                self.next_token().1.end
            } else {
                let span = self.peek_token_span();
                error_unclosed_delimiter(
                    self,
                    opener,
                    opener_span,
                    expected_closer,
                    Some((t, span)),
                );
            }
        } else {
            error_unclosed_delimiter(self, opener, opener_span, expected_closer, None);
        }
    }
    #[inline(never)]
    #[cold]
    pub fn throw_parser_err<'b, F: Fn() -> Report<'b, (&'b str, core::ops::Range<usize>)>>(
        &self,
        report: F,
        span: Span,
        message: &str,
        code: &str,
    ) -> ! {
        if crate::errors::diagnostics_enabled() {
            crate::errors::emit_diagnostic(
                self.ctx.src.filename.as_str(),
                (span.start as usize)..(span.end as usize),
                crate::errors::strip_ansi(message),
                code,
            );
        }
        let report = report();

        let _ = report.write(
            (
                self.ctx.src.filename.as_str(),
                ariadne::Source::from(self.ctx.src.contents.as_str()),
            ),
            candela_vm::captured_output::stderr(),
        );

        crash();
    }
}

// Call after DoubleColon is skipped
// Returns end
fn parse_namespace(parser: &mut Parser<'_>, initial: SmolStr) -> (Box<[SmolStr]>, u32) {
    let mut namespace: Vec<SmolStr> = Vec::with_capacity(2);
    namespace.push(initial);
    let end = extend_namespace(parser, &mut namespace);
    (Box::from(namespace), end)
}

/// Reads the segments behind a `::` that is already consumed, appending each to
/// `namespace` and answering where the last one ends. A path whose middle
/// segment carries type arguments (`g::Slot<int>::Filled`) comes through here
/// twice, once for each side of the list.
fn extend_namespace(parser: &mut Parser<'_>, namespace: &mut Vec<SmolStr>) -> u32 {
    let mut end: u32;
    loop {
        let (next_token, span) = parser.next_token();
        if let Token::Identifier(i) = next_token {
            namespace.push(SmolStr::new(i));
            end = span.end;
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    next_token,
                    "Wrong namespace syntax",
                ),
            );
        }
        if parser.peek_token() == Token::DoubleColon {
            // Consume the separator so the next turn of the loop reads the
            // segment behind it. Without this the loop takes the `::` itself
            // for a segment, which caps a path at two of them.
            parser.next_token();
            continue;
        }
        return end;
    }
}

// Must be called after LParen is skipped
fn parse_args(parser: &mut Parser<'_>) -> (Box<[Expr]>, Box<[Span]>, u32) {
    let mut args = Vec::with_capacity(4);
    let mut arg_markers: Vec<Span> = Vec::with_capacity(4);
    loop {
        if parser.peek_token() == Token::RParen {
            let end = parser.next_token().1.end;
            return (Box::from(args), Box::from(arg_markers), end);
        }
        let arg_start: u32 = parser.peek_token_span().start;
        args.push(parse_expr(parser));
        arg_markers.push((arg_start, parser.peek_token_span().start).into());
        if parser.peek_token() == Token::Comma {
            parser.next_token();
        } else if !(parser.peek_token() == Token::RParen) {
            cold_path();
            let span = parser.peek_token_span();
            parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
        }
    }
}

/// Expands one `name!( ... )` invocation and parses what the expander returned
/// in its place. `span` covers the whole invocation.
fn expand_macro(parser: &Parser<'_>, region: MacroToken<'_>, span: Span) -> Expr {
    let Some(body) = region.body else {
        cold_path();
        parser.error(span, ParserErr::UnterminatedMacroRegion(region.name));
    };
    // Code a `@cfg` drops is never compiled, so what the macro would expand
    // to does not matter, and the expander may not exist in this build.
    if parser.inactive > 0 {
        return Expr::Null;
    }
    // Held until this expansion is parsed, so a macro reached through a chain
    // of other macros counts the whole chain. At the cap `span` is already the
    // outermost invocation, which is the one the reader wrote.
    let Some(_level) = macros::Depth::enter() else {
        parser.error(span, ParserErr::MacroRecursionLimit(region.name));
    };
    match macros::expand(region.name, body) {
        Expansion::Text(expansion) => parse_expansion(parser, region.name, &expansion, span),
        Expansion::Null => Expr::Null,
        Expansion::Unknown => {
            cold_path();
            parser.error(span, ParserErr::UnknownMacro(region.name));
        }
        Expansion::Failed(error) => {
            cold_path();
            // `name!(` is the name plus two characters, all ASCII, so the body
            // starts that many bytes into the invocation.
            let body_start = span.start + region.name.len() as u32 + 2;
            let error_span = match error.offset {
                Some(offset) => {
                    let start = body_start + (offset.min(body.len()) as u32);
                    (start, (start + 1).min(body_start + body.len() as u32)).into()
                }
                None => span,
            };
            parser.error(
                error_span,
                ParserErr::MacroExpansionFailed(region.name, error.message),
            );
        }
    }
}

/// Parses `expansion` as a single expression, placed at the macro invocation
/// `span`. The expanded text is not in any file, so every span it produces is
/// the invocation's own; a program compiled from it reports against the macro
/// the reader wrote, not against text they cannot see.
fn parse_expansion(parser: &Parser<'_>, macro_name: &str, expansion: &str, span: Span) -> Expr {
    let mut sub = Parser {
        input: Token::lexer(expansion).spanned().peekable(),
        ctx: ParserCtx {
            src: parser.ctx.src,
            cfg: parser.ctx.cfg,
        },
        last_token_end: span.end as usize,
        span_override: Some(span),
        nesting: parser.nesting,
        pending_gt: None,
        inactive: parser.inactive,
    };
    let expr = parse_expr(&mut sub);
    if sub.peek_token_opt().is_some() {
        cold_path();
        sub.error(span, ParserErr::MacroExpansionTrailingTokens(macro_name));
    }
    expr
}

fn parse_statement(parser: &mut Parser<'_>) -> Option<Expr> {
    let mut token = parser.peek_token_opt()?;
    // A statement a `@cfg` turns off is parsed and dropped, and the statement
    // after it stands in its place.
    while token == Token::At {
        let active = parse_attributes(parser);
        if matches!(parser.peek_token_opt(), None | Some(Token::RBrace)) {
            cold_path();
            let span = parser
                .peek_token_opt_span()
                .unwrap_or_else(|| parser.eof_span());
            parser.error(span, ParserErr::AttributeWithoutItem);
        }
        if active {
            return parse_statement(parser);
        }
        parser.inactive += 1;
        let _ = parse_statement(parser);
        parser.inactive -= 1;
        token = parser.peek_token_opt()?;
    }
    let t_span = parser.peek_token_span();
    match token {
        Token::If => Some(parse_condition_block(parser, t_span.start)),
        Token::While => Some(parse_while_block(parser)),
        Token::For => Some(parse_for_loop(parser)),
        Token::Match => Some(parse_match(parser)),
        Token::LBrace => Some(parse_eval_block(parser)),
        // Only `parse_file` accepts a function declaration. Reaching one here
        // means it was written inside a block, where it would otherwise parse
        // and then be dropped without ever being registered.
        Token::Function => {
            cold_path();
            parser.error(t_span, ParserErr::NestedFunctionDeclaration);
        }
        Token::Loop => Some(parse_loop_block(parser)),
        Token::Try => Some(parse_try_catch_block(parser)),
        Token::Struct => Some(parse_struct_declare(parser)),
        Token::Enum => Some(parse_enum_declare(parser)),
        Token::RBrace => None,
        t => Some(parse_line(parser, t)),
    }
}

fn parse_var_declare(parser: &mut Parser<'_>) -> Expr {
    let (t, _) = parser.next_token();
    debug_assert_eq!(t, Token::Let);
    let (t, span) = parser.next_token();
    let var_name = if let Token::Identifier(id) = t {
        SmolStr::new(id)
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(
                Token::Identifier(""),
                t,
                "Variable names must be identifiers.",
            ),
        );
    };
    parser.next_token_expect(
        Token::Equals,
        "Variable declarations need a '=' to separate the name from the value.",
    );
    let var_value = parse_expr(parser);
    Expr::VarDeclare(var_name, Box::new(var_value))
}

fn parse_var_assign(input: &mut Parser<'_>, e: Expr, e_start: u32) -> Expr {
    let (t, _) = input.next_token();
    debug_assert_eq!(t, Token::Equals);
    let e_end = input.peek_token_span().end;
    let v_start = input.peek_token_span().start;
    let v = parse_expr(input);
    let v_end = input.peek_token_span().start;
    var_assign(e, v, (e_start, e_end).into(), (v_start, v_end).into())
}

fn parse_op_var_assign(input: &mut Parser<'_>, e: Expr, e_start: u32, op: Token<'_>) -> Expr {
    let operand_end = input.last_token_end as u32;
    let (t, _) = input.next_token();
    debug_assert_eq!(t, op);
    let e_end = input.peek_token_span().end;
    let v_start = input.peek_token_span().start;
    let v = parse_expr(input);
    let v_end = input.last_token_end as u32;
    let op = match op {
        Token::AssignOpAdd => Token::OpAdd,
        Token::AssignOpSub => Token::OpSub,
        Token::AssignOpMul => Token::OpMul,
        Token::AssignOpDiv => Token::OpDiv,
        Token::AssignOpMod => Token::OpMod,
        Token::AssignOpPow => Token::OpPow,
        Token::AssignOpBitAnd => Token::OpBitAnd,
        Token::AssignOpBitOr => Token::Pipe,
        Token::AssignOpBitXor => Token::OpBitXor,
        Token::AssignOpShl => Token::OpShl,
        Token::AssignOpShr => Token::OpShr,
        _ => unsafe { unreachable_unchecked() },
    };
    var_assign(
        e.clone(),
        add_op(
            input,
            op,
            e,
            v,
            (e_start, operand_end).into(),
            (v_start, v_end).into(),
        ),
        (e_start, e_end).into(),
        (v_start, v_end).into(),
    )
}

fn parse_return(input: &mut Parser<'_>) -> Expr {
    let (t, _) = input.next_token();
    debug_assert_eq!(t, Token::Return);
    if input.peek_token_opt() == Some(Token::SemiColon) {
        Expr::ReturnVal(Box::new(None))
    } else {
        let e = parse_expr(input);
        Expr::ReturnVal(Box::new(Some(e)))
    }
}

fn parse_line(input: &mut Parser<'_>, peek: Token<'_>) -> Expr {
    let line_code = match peek {
        Token::Let => parse_var_declare(input),
        Token::Return => parse_return(input),
        Token::Break => {
            input.next_token();
            Expr::Break
        }
        Token::Continue => {
            input.next_token();
            Expr::Continue
        }
        _ => {
            let e_start = input.peek_token_span().start;
            let e = parse_expr(input);
            let peek_token = input.peek_token_opt();
            match peek_token {
                Some(Token::Equals) => parse_var_assign(input, e, e_start),
                Some(
                    op @ (Token::AssignOpAdd
                    | Token::AssignOpSub
                    | Token::AssignOpMul
                    | Token::AssignOpDiv
                    | Token::AssignOpMod
                    | Token::AssignOpPow
                    | Token::AssignOpBitAnd
                    | Token::AssignOpBitOr
                    | Token::AssignOpBitXor
                    | Token::AssignOpShl
                    | Token::AssignOpShr),
                ) => parse_op_var_assign(input, e, e_start, op),
                _ => e,
            }
        }
    };
    if input.peek_token_opt() != Some(Token::SemiColon) {
        error_missing_semicolon(input);
    }
    input.next_token();
    // input.next_token_expect(Token::SemiColon, "Lines must end with a ';'.");
    line_code
}

#[cold]
#[inline(never)]
fn error_unclosed_delimiter(
    parser: &Parser<'_>,
    opener_token: Token,
    opener_span: Span,
    expected_closer_token: Token,
    end: Option<(Token, Span)>,
) -> ! {
    let message = if let Some((actual_closer_token, _)) = end {
        format!(
            "This {opener_token} is never closed: expected {expected_closer_token} but found {actual_closer_token}"
        )
    } else {
        format!(
            "This {opener_token} is never closed: expected {expected_closer_token} but the file ends here"
        )
    };
    parser.throw_parser_err(
        || {
            let mut report = Report::build(
                ariadne::ReportKind::Error,
                (parser.ctx.src.filename.as_str(), opener_span.into()),
            )
            .with_message("Unclosed delimiter")
            .with_label(
                Label::new((parser.ctx.src.filename.as_str(), opener_span.into()))
                    .with_message(format_args!("This {opener_token} is never closed"))
                    .with_color(ariadne::Color::Red),
            );

            if let Some((actual_closer_token, actual_closer_token_span)) = end {
                report = report.with_label(
                    Label::new((
                        parser.ctx.src.filename.as_str(),
                        actual_closer_token_span.into(),
                    ))
                    .with_message(format_args!(
                        "Expected {expected_closer_token} but found {actual_closer_token}"
                    ))
                    .with_color(ariadne::Color::Red),
                );
            } else {
                report = report
                    .with_label(
                        Label::new((parser.ctx.src.filename.as_str(), parser.eof_span().into()))
                            .with_message(format_args!(
                                "Expected {expected_closer_token} but the file ends here"
                            ))
                            .with_color(ariadne::Color::Red),
                    )
                    .with_help(format_args!(
                        "Add a {} here to close it",
                        blue(expected_closer_token)
                    ));
            }

            report.finish()
        },
        opener_span,
        &message,
        "unclosed_delimiter",
    )
}

#[cold]
#[inline(never)]
fn error_missing_semicolon(parser: &Parser<'_>) -> ! {
    let span: Span = (parser.last_token_end as u32, parser.last_token_end as u32).into();
    parser.throw_parser_err(
        || {
            Report::build(
                ariadne::ReportKind::Error,
                (
                    parser.ctx.src.filename.as_str(),
                    (parser.last_token_end..parser.last_token_end),
                ),
            )
            .with_message("Missing semicolon")
            .with_label(
                Label::new((
                    parser.ctx.src.filename.as_str(),
                    (parser.last_token_end..parser.last_token_end),
                ))
                .with_message(format_args!("Add a {} here", blue(';')))
                .with_color(ariadne::Color::Blue),
            )
            .with_help("All statements end with a ';'")
            .finish()
        },
        span,
        "Missing semicolon",
        "missing_semicolon",
    )
}

fn parse_code(input: &mut Parser<'_>) -> Vec<Expr> {
    let mut output: Vec<Expr> = Vec::with_capacity(4);
    while let Some(e) = parse_statement(input) {
        output.push(e);
    }
    output
}

fn parse_file_import(parser: &mut Parser<'_>) -> Expr {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Import);
    let (next_token, span) = parser.next_token();
    // One import form: a quoted path.
    //   * A path ending in `.cdl` is a file import, resolved next to the
    //     importing file (with the shipped library directory as fallback).
    //   * A path with no extension is a library import: the resolver appends
    //     `.cdl` and looks it up in the shipped library directory only
    //     (`import "std/string";` -> `std/string.cdl`).
    let (path, is_logical, mut end) = if let Token::String(s) = next_token {
        let raw = parse_string(s);
        if raw.ends_with(".cdl") {
            (raw, false, span.end)
        } else {
            let last_segment = raw.rsplit(['/', '\\']).next().unwrap_or(raw.as_str());
            if last_segment.contains('.') {
                cold_path();
                parser.error(span, ParserErr::ImportPathBadExtension);
            }
            let mut with_ext = String::with_capacity(raw.len() + 4);
            with_ext.push_str(raw.as_str());
            with_ext.push_str(".cdl");
            (SmolStr::new(&with_ext), true, span.end)
        }
    } else if let Token::Identifier(first) = next_token {
        // The old namespaced form (`import std::string;`) parses far enough to
        // suggest the exact replacement, then errors.
        let mut segments = String::from(first);
        let mut end = span.end;
        while parser.peek_token_opt() == Some(Token::DoubleColon) {
            parser.next_token();
            let (seg_token, seg_span) = parser.next_token();
            if let Token::Identifier(seg) = seg_token {
                segments.push('/');
                segments.push_str(seg);
                end = seg_span.end;
            } else {
                break;
            }
        }
        cold_path();
        parser.error(
            (span.start, end).into(),
            ParserErr::LegacyNamespacedImport(segments),
        );
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(
                Token::String(""),
                next_token,
                "An import is a quoted path: import \"./local.cdl\"; for a file, import \"std/string\"; for a library.",
            ),
        );
    };
    let peek_token = parser.peek_token_opt();
    if peek_token == Some(Token::As) {
        parser.next_token();
        let (next_token, span) = parser.next_token();
        let alias = if let Token::Identifier(id) = next_token {
            SmolStr::new(id)
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    next_token,
                    "Module aliases must be identifiers.",
                ),
            );
        };
        end = span.end;
        parser.next_token_expect(
            Token::SemiColon,
            "Import statements must end with a semicolon",
        );
        Expr::ImportFile(path, Some(alias), is_logical, (start, end).into())
    } else {
        parser.next_token_expect(
            Token::SemiColon,
            "Import statements must end with a semicolon",
        );
        Expr::ImportFile(path, None, is_logical, (start, end).into())
    }
}

fn parse_type(parser: &mut Parser<'_>) -> TypeExpr {
    let span = parser.peek_token_span();
    parser.enter(span);
    let t = parse_type_inner(parser);
    parser.leave();
    t
}

fn parse_type_inner(parser: &mut Parser<'_>) -> TypeExpr {
    let t = parse_atomic_type(parser);
    if parser.peek_token() == Token::Pipe {
        let mut poly = Vec::with_capacity(2);
        poly.push(t);
        while parser.peek_token() == Token::Pipe {
            parser.next_token();
            poly.push(parse_atomic_type(parser));
        }
        TypeExpr::Union(poly.into_boxed_slice())
    } else {
        t
    }
}

fn parse_atomic_type(parser: &mut Parser<'_>) -> TypeExpr {
    let (next_token, span) = parser.next_token();
    let mut t = if next_token == Token::Function {
        // `fn(A, B) -> R`: a position that holds a function of that shape. The
        // return type runs to the end of the type, so the `[]` in
        // `fn(int) -> int[]` belongs to it; parenthesise the whole type to make
        // a list of functions.
        parser.next_token_expect(
            Token::LParen,
            "A function type needs a parameter list: fn(int, int) -> int",
        );
        let mut args: Vec<TypeExpr> = Vec::new();
        while parser.peek_token() != Token::RParen {
            args.push(parse_type(parser));
            if parser.peek_token() == Token::Comma {
                parser.next_token();
            } else {
                break;
            }
        }
        parser.next_token_expect(Token::RParen, "Unmatched '(' in a function type");
        let return_type = if parser.peek_token() == Token::Arrow {
            parser.next_token();
            Some(parse_type(parser))
        } else {
            None
        };
        TypeExpr::Fn(Box::new(FnTypeExpr {
            args: args.into_boxed_slice(),
            return_type,
            span: (span.start, parser.last_token_end as u32).into(),
        }))
    } else if next_token == Token::LParen {
        // A parenthesised type, which is what puts a function type under a
        // list suffix: `(fn(int) -> int)[]`.
        let inner = parse_type(parser);
        parser.next_token_expect(Token::RParen, "Unmatched '(' in a type");
        inner
    } else if next_token == Token::LBrace {
        let key_t = parse_type(parser);
        parser.next_token_expect(
            Token::Colon,
            "A colon must separate key and value types in map types",
        );
        let value_t = parse_type(parser);
        parser.next_token_expect(Token::RBrace, "Unmatched '{'");
        TypeExpr::Map(Box::new(key_t), Box::new(value_t))
    } else if let Token::Identifier(i) = next_token {
        if parser.peek_token() == Token::DoubleColon {
            parser.next_token();
            let (path, end) = parse_namespace(parser, SmolStr::new(i));
            // A generic type a module declares carries its arguments on the
            // name at the end of the path, `g::Slot<int>`, the same place a
            // name written on its own carries them. What is left of the path
            // is the module it comes from.
            if parser.peek_token() == Token::OpInf {
                let mut path = path.into_vec();
                let name = path.pop().unwrap_or_else(|| SmolStr::new(i));
                let args = parse_type_args(parser);
                TypeExpr::Generic(Box::new(GenericType {
                    namespace: Box::from(path),
                    name,
                    args,
                    span: (span.start, parser.last_token_end as u32).into(),
                }))
            } else {
                TypeExpr::NamespacedIdentifier(path, (span.start, end).into())
            }
        } else if parser.peek_token() == Token::OpInf {
            // A generic type applied to its arguments. A type position is never
            // ambiguous, so this needs no lookahead: `<` here can only open a
            // type-argument list.
            let args = parse_type_args(parser);
            TypeExpr::Generic(Box::new(GenericType {
                namespace: Box::from([]),
                name: SmolStr::new(i),
                args,
                span: (span.start, parser.last_token_end as u32).into(),
            }))
        } else {
            TypeExpr::Identifier(SmolStr::new(i), span)
        }
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(Token::Identifier(""), next_token, "Invalid type"),
        );
    };
    loop {
        if parser.peek_token() == Token::LBracket {
            parser.next_token();
            parser.next_token_expect(Token::RBracket, "Unmatched '['");
            t = TypeExpr::Array(Box::new(t));
        } else {
            break;
        }
    }
    t
}

/// Walks a copy of the token stream to decide whether the `<` an identifier is
/// followed by opens a type-argument list or is a comparison.
///
/// `a < b` and `a < b && c > d` are comparisons and must stay comparisons, so
/// the walk accepts only well-formed type tokens and commits only when the list
/// closes with `>` immediately followed by `(` (a call), `{` (a struct literal,
/// where one is allowed) or `::` (a variant of a generic enum). It reports by
/// returning: the parser cannot fail and retry, so a walk that raised an error
/// would abort the compile over an ordinary comparison.
struct TypeArgScan<'a> {
    input: TokenIter<'a>,
    /// The second half of a `>>` the walk has already read. See
    /// [`Parser::next_type_list_close`], which splits the same token when the
    /// list is parsed for real.
    pending_gt: bool,
}

impl<'a> TypeArgScan<'a> {
    fn peek(&mut self) -> Option<Token<'a>> {
        if self.pending_gt {
            return Some(Token::OpSup);
        }
        self.input.peek().and_then(|(t, _)| t.ok())
    }

    fn eat(&mut self, expected: Token<'a>) -> bool {
        if self.peek() == Some(expected) {
            if self.pending_gt {
                self.pending_gt = false;
            } else {
                self.input.next();
            }
            true
        } else {
            false
        }
    }

    /// The `>` that closes a list, which is the first half of a `>>` where two
    /// lists close at once.
    fn eat_gt(&mut self) -> bool {
        if self.eat(Token::OpSup) {
            return true;
        }
        if self.peek() == Some(Token::OpShr) {
            self.input.next();
            self.pending_gt = true;
            return true;
        }
        false
    }

    fn identifier(&mut self) -> bool {
        if matches!(self.peek(), Some(Token::Identifier(_))) {
            self.input.next();
            true
        } else {
            false
        }
    }

    /// `<Type, Type, ...>`, the `<` included.
    fn args(&mut self) -> bool {
        if !self.eat(Token::OpInf) {
            return false;
        }
        loop {
            if !self.type_expr() {
                return false;
            }
            if !self.eat(Token::Comma) {
                return self.eat_gt();
            }
        }
    }

    fn type_expr(&mut self) -> bool {
        loop {
            if !self.atomic_type() {
                return false;
            }
            if !self.eat(Token::Pipe) {
                return true;
            }
        }
    }

    fn atomic_type(&mut self) -> bool {
        if self.eat(Token::LBrace) {
            if !self.type_expr() || !self.eat(Token::Colon) || !self.type_expr() {
                return false;
            }
            if !self.eat(Token::RBrace) {
                return false;
            }
        } else {
            if !self.identifier() {
                return false;
            }
            while self.eat(Token::DoubleColon) {
                if !self.identifier() {
                    return false;
                }
            }
            if self.peek() == Some(Token::OpInf) && !self.args() {
                return false;
            }
        }
        while self.eat(Token::LBracket) {
            if !self.eat(Token::RBracket) {
                return false;
            }
        }
        true
    }
}

/// What a type-argument list may be followed by at the position it is read.
///
/// The list only counts as one when the `>` closes right before one of these,
/// which is what tells `Cell<int>{ .. }` from the comparison `a < b`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TypeArgFollow {
    /// A term: a call, a struct literal, or an enum-variant path.
    Term,
    /// A term in a position where a struct literal cannot start, which is the
    /// header of an `if`, `while` or `for`.
    TermNoStruct,
    /// A method call, which is always the call parentheses.
    Call,
}

/// Whether the `<` ahead opens a type-argument list. See [`TypeArgScan`].
///
/// Kept out of line: the walk's own state would otherwise enlarge every
/// `parse_term` frame, and expression parsing recurses once per nesting level.
#[inline(never)]
fn type_args_ahead(parser: &Parser<'_>, follow: TypeArgFollow) -> bool {
    debug_assert!(
        parser.pending_gt.is_none(),
        "the half of a '>>' left over from a type list is read before any expression"
    );
    let mut scan = TypeArgScan {
        input: parser.input.clone(),
        pending_gt: false,
    };
    if !scan.args() {
        return false;
    }
    match scan.peek() {
        Some(Token::LParen) => true,
        Some(Token::DoubleColon) => follow != TypeArgFollow::Call,
        Some(Token::LBrace) => follow == TypeArgFollow::Term,
        _ => false,
    }
}

/// Parses `<Type, Type, ...>` where a type-argument list is already known to
/// start. The opening `<` has not been consumed.
fn parse_type_args(parser: &mut Parser<'_>) -> Box<[TypeExpr]> {
    parser.next_token_expect(Token::OpInf, "A type argument list starts with '<'.");
    let mut args: Vec<TypeExpr> = Vec::with_capacity(2);
    loop {
        args.push(parse_type(parser));
        if parser.peek_token() == Token::Comma {
            parser.next_token();
        } else {
            break;
        }
    }
    parser.next_type_list_close("A type argument list ends with '>'.");
    Box::from(args)
}

/// Parses the `<T, U>` a declaration may carry after its name, naming the type
/// parameters its body may use. Returns an empty list when there is none.
fn parse_type_params(parser: &mut Parser<'_>) -> TypeParams {
    if parser.peek_token_opt() != Some(Token::OpInf) {
        return Box::from([]);
    }
    parser.next_token();
    let mut params: Vec<SmolStr> = Vec::with_capacity(2);
    loop {
        let (next_token, span) = parser.next_token();
        if let Token::Identifier(name) = next_token {
            params.push(SmolStr::new(name));
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    next_token,
                    "Type parameters must be identifiers.",
                ),
            );
        }
        if parser.peek_token() == Token::Comma {
            parser.next_token();
        } else {
            break;
        }
    }
    parser.next_type_list_close("A type parameter list ends with '>'.");
    Box::from(params)
}

fn parse_dylib_import(parser: &mut Parser<'_>) -> Expr {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Dylib);
    let (next_token, span) = parser.next_token();
    let path = if let Token::String(s) = next_token {
        SmolStr::new(parse_string(s))
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(Token::String(""), next_token, "Paths must be strings."),
        );
    };
    parser.next_token_expect(Token::LBrace, "Blocks need to start with '{'.");
    let (fn_signatures, end) = parse_fn_signature_block(parser, None);
    Expr::ImportDylib(path, fn_signatures, (start, end).into())
}

/// Parses a brace-delimited block of typed function signatures shared by
/// `dylib "..." { ... }` and `host "..." { ... }`. The opening `{` must already
/// have been consumed; this consumes through the closing `}` and returns the
/// parsed signatures together with the end offset of the `}`.
///
/// `structs` is where a `host` block collects the structs it declares; a
/// `dylib` block passes `None` and declares none.
fn parse_fn_signature_block(
    parser: &mut Parser<'_>,
    mut structs: Option<&mut Vec<Expr>>,
) -> (Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>, u32) {
    let mut fn_signatures: Vec<(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)> = Vec::new();
    let end: u32;
    loop {
        if parser.peek_token() == Token::RBrace {
            end = parser.next_token().1.end;
            break;
        }
        if parser.peek_token() == Token::Struct
            && let Some(structs) = structs.as_deref_mut()
        {
            let declared = parse_struct_declare(parser);
            if let Expr::StructDeclare(_, _, span, type_params, _) = &declared
                && !type_params.is_empty()
            {
                cold_path();
                parser.error(
                    *span,
                    ParserErr::UnexpectedToken(
                        Token::LBrace,
                        Token::OpInf,
                        "A struct a host block declares takes no type parameters.",
                    ),
                );
            }
            structs.push(declared);
            continue;
        }

        let type_start = parser.peek_token();
        let span = parser.peek_token_span();
        let first = parse_type(parser);
        let fn_name_span: Span;
        let (return_type, fn_name) = if parser.peek_token() == Token::LParen {
            if let TypeExpr::Identifier(name, span) = first {
                fn_name_span = span;
                (
                    TypeExpr::Identifier(SmolStr::new_static("null"), span),
                    name,
                )
            } else {
                parser.error(
                    span,
                    ParserErr::UnexpectedToken(
                        Token::Identifier(""),
                        type_start,
                        "Function names must be identifiers.",
                    ),
                );
            }
        } else {
            let (next_token, span) = parser.next_token();
            fn_name_span = span;
            let fn_name = if let Token::Identifier(name) = next_token {
                SmolStr::new(name)
            } else {
                cold_path();
                parser.error(
                    span,
                    ParserErr::UnexpectedToken(
                        Token::Identifier(""),
                        next_token,
                        "Function names must be identifiers.",
                    ),
                );
            };
            (first, fn_name)
        };
        parser.next_token_expect(
            Token::LParen,
            "Function arguments must be delimited by parentheses",
        );
        let mut args: Vec<TypeExpr> = Vec::with_capacity(2);
        loop {
            if parser.peek_token() == Token::RParen {
                break;
            }
            // `...` marks a variadic host function: it accepts any number of
            // arguments of any type, delivered to the registered closure as a
            // slice. Carried through as a reserved sentinel type the host
            // resolver recognises (meaningless in a `dylib` block).
            if parser.peek_token() == Token::Ellipsis {
                let span = parser.peek_token_span();
                parser.next_token();
                args.push(TypeExpr::Identifier(SmolStr::new_static("..."), span));
                break;
            }
            args.push(parse_type(parser));
            if parser.peek_token() == Token::Comma {
                parser.next_token();
            } else if !(parser.peek_token() == Token::RParen) {
                cold_path();
                let span = parser.peek_token_span();
                parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
            }
        }
        parser.next_token_expect(Token::RParen, "Unmatched ')'");
        parser.next_token_expect(
            Token::SemiColon,
            "Function definitions must end with a semicolon",
        );
        fn_signatures.push((fn_name, Box::from(args), return_type, fn_name_span));
    }
    (fn_signatures.into_boxed_slice(), end)
}

/// Parses a `host "namespace" { signatures... }` block. The namespace names a
/// table of Rust closures registered on the embedding [`crate::Engine`]; the
/// signatures are type-checked at compile time exactly like `dylib` signatures.
fn parse_host_block(parser: &mut Parser<'_>) -> Expr {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Host);
    let (next_token, span) = parser.next_token();
    let namespace = if let Token::String(s) = next_token {
        SmolStr::new(parse_string(s))
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(
                Token::String(""),
                next_token,
                "Host namespaces must be strings.",
            ),
        );
    };
    parser.next_token_expect(Token::LBrace, "Blocks need to start with '{'.");
    let mut structs: Vec<Expr> = Vec::new();
    let (fn_signatures, end) = parse_fn_signature_block(parser, Some(&mut structs));
    Expr::HostBlock(
        namespace,
        fn_signatures,
        (start, end).into(),
        structs.into_boxed_slice(),
    )
}

#[inline(always)]
fn parse_file(parser: &mut Parser<'_>, impls: &mut Vec<ImplTemplate>) -> Vec<Expr> {
    let mut output: Vec<Expr> = Vec::with_capacity(2);
    // parse file statements
    while parser.peek_token_opt().is_some() {
        if parser.peek_token() == Token::At {
            let active = parse_attributes(parser);
            if parser.peek_token_opt().is_none() {
                cold_path();
                let span = parser.eof_span();
                parser.error(span, ParserErr::AttributeWithoutItem);
            }
            if active {
                parse_item(parser, &mut output, impls);
            } else {
                parser.inactive += 1;
                parse_item(parser, &mut Vec::new(), &mut Vec::new());
                parser.inactive -= 1;
            }
            continue;
        }
        parse_item(parser, &mut output, impls);
    }
    output
}

/// Parses one top-level declaration into `output`.
fn parse_item(parser: &mut Parser<'_>, output: &mut Vec<Expr>, impls: &mut Vec<ImplTemplate>) {
    let t = parser.peek_token();
    // An `impl` block lowers to several top-level function declarations, so
    // it is expanded directly into `output` rather than yielding one Expr.
    // A block on a generic type is kept as a template instead.
    if t == Token::Impl {
        parse_impl_block(parser, output, impls);
        return;
    }
    output.push(match t {
        Token::Function => parse_function(parser),
        Token::Import => parse_file_import(parser),
        Token::Struct => parse_struct_declare(parser),
        Token::Enum => parse_enum_declare(parser),
        Token::Dylib => parse_dylib_import(parser),
        Token::Host => parse_host_block(parser),
        unexpected => {
            cold_path();
            let span = parser.peek_token_span();
            parser.error(span, ParserErr::UnexpectedTokenStr("'fn' (function declaration), 'import', 'struct' (struct declaration), 'enum' (enum declaration), 'impl' (method block), 'dylib' (dynamic library import), or 'host' (host function block)", unexpected, "Invalid file statement."));
        }
    });
}

/// Reads the run of attributes in front of an item or a statement, and says
/// whether the code they apply to is compiled: true when every `@cfg` among
/// them holds.
fn parse_attributes(parser: &mut Parser<'_>) -> bool {
    let mut active = true;
    while parser.peek_token_opt() == Some(Token::At) {
        parser.next_token();
        let (t, span) = parser.next_token();
        match t {
            Token::Identifier("cfg") => {}
            Token::Identifier(name) => {
                cold_path();
                parser.error(span, ParserErr::UnknownAttribute(name));
            }
            other => {
                cold_path();
                parser.error(
                    span,
                    ParserErr::UnexpectedToken(
                        Token::Identifier("cfg"),
                        other,
                        "An attribute is '@' followed by its name, as in @cfg(web).",
                    ),
                );
            }
        }
        let open = parser.next_token_expect(
            Token::LParen,
            "A cfg attribute holds its condition in parentheses, as in @cfg(web).",
        );
        let holds = parse_cfg_predicate(parser);
        parser.next_token_expect_closer(Token::LParen, open, Token::RParen);
        active &= holds;
    }
    active
}

/// Reads one `@cfg` condition and evaluates it against the active
/// configuration: a flag, `key = "value"`, or `not(...)`, `any(...)` and
/// `all(...)` around further conditions.
fn parse_cfg_predicate(parser: &mut Parser<'_>) -> bool {
    const EXPECTED: &str = "a flag, key = \"value\", not(...), any(...), or all(...)";
    let (t, span) = parser.next_token();
    let Token::Identifier(name) = t else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedTokenStr(EXPECTED, t, "This is the condition of a cfg attribute."),
        );
    };
    match parser.peek_token() {
        Token::LParen if matches!(name, "not" | "any" | "all") => {
            let (_, open) = parser.next_token();
            parser.enter(open);
            let mut results = Vec::new();
            while parser.peek_token() != Token::RParen {
                results.push(parse_cfg_predicate(parser));
                if parser.peek_token() == Token::Comma {
                    parser.next_token();
                } else {
                    break;
                }
            }
            parser.leave();
            parser.next_token_expect_closer(Token::LParen, open, Token::RParen);
            match name {
                "not" => {
                    if results.len() != 1 {
                        cold_path();
                        parser.error(
                            (span.start, parser.last_token_end as u32).into(),
                            ParserErr::UnexpectedTokenStr(
                                "exactly one condition",
                                Token::RParen,
                                "not(...) inverts one condition.",
                            ),
                        );
                    }
                    !results[0]
                }
                "any" => results.contains(&true),
                _ => !results.contains(&false),
            }
        }
        Token::Equals => {
            parser.next_token();
            let (value, value_span) = parser.next_token();
            let Token::String(value) = value else {
                cold_path();
                parser.error(
                    value_span,
                    ParserErr::UnexpectedToken(
                        Token::String(""),
                        value,
                        "The value a cfg key is compared with is a quoted string.",
                    ),
                );
            };
            parser.ctx.cfg.has_value(name, &parse_string(value))
        }
        _ => parser.ctx.cfg.is_enabled(name),
    }
}

/// A parsed file: its top-level statements and its generic `impl` blocks.
///
/// A block written against a generic type carries no type of its own to attach
/// to, so it travels beside the statements and is lowered per instantiation.
pub struct ParsedFile {
    pub code: Vec<Expr>,
    pub impls: Vec<ImplTemplate>,
}

#[must_use]
pub fn parse(input: &str, src: &Source) -> ParsedFile {
    let mut impls: Vec<ImplTemplate> = Vec::new();
    let cfg = cfg::active();
    let code = parse_file(
        &mut Parser {
            input: Token::lexer(input).spanned().peekable(),
            ctx: ParserCtx { src, cfg: &cfg },
            last_token_end: 0,
            span_override: None,
            nesting: 0,
            pending_gt: None,
            inactive: 0,
        },
        &mut impls,
    );
    ParsedFile { code, impls }
}

/// What one line of source is, as far as the grammar can tell from its opening
/// tokens.
///
/// The REPL builds a file out of the lines typed at it, and where a line goes
/// depends on which of these it is. Only the grammar knows which tokens open a
/// declaration, so the answer is read here rather than guessed from the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A top-level declaration: one of the forms [`parse`] accepts at file
    /// scope, which a block does not.
    Declaration,
    /// A statement: it belongs in a block and yields no value.
    Statement,
    /// Anything else, which may be an expression with a value to show. The
    /// reading is a guess: the opening tokens rule out the other two, and only
    /// a compile settles the rest.
    Expression,
}

/// Reads `line` as far as its first tokens, and says which of [`LineKind`] it
/// opens.
///
/// A line whose first token is unknown to the lexer reads as a statement, so
/// the error it raises is the one its own text earns rather than one about
/// text wrapped around it.
#[must_use]
pub fn classify_line(line: &str) -> LineKind {
    let mut tokens = Token::lexer(line);
    let Some(Ok(mut first)) = tokens.next() else {
        return LineKind::Statement;
    };
    // Attributes in front of a line say nothing about what it is: the line
    // is what follows them.
    while first == Token::At {
        let mut depth = 0_u32;
        let mut after = None;
        for token in tokens.by_ref() {
            let Ok(token) = token else {
                return LineKind::Statement;
            };
            match token {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        after = tokens.next();
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(Ok(next)) = after else {
            return LineKind::Statement;
        };
        first = next;
    }
    match first {
        Token::Function
        | Token::Import
        | Token::Struct
        | Token::Enum
        | Token::Impl
        | Token::Dylib
        | Token::Host => LineKind::Declaration,
        // Every form `parse_statement` reads on its own, plus `{`, which opens
        // an evaluation block there and not a map literal.
        Token::Let
        | Token::If
        | Token::Else
        | Token::While
        | Token::For
        | Token::Match
        | Token::Loop
        | Token::Try
        | Token::Catch
        | Token::Return
        | Token::Break
        | Token::Continue
        | Token::LBrace => LineKind::Statement,
        // An assignment is a statement wherever its target ends, and `=` never
        // stands anywhere else in an expression: a struct literal and a map
        // both separate with ':', and a call's arguments with ','. A string
        // holding an '=' is one token, so its contents cannot be mistaken for
        // one.
        //
        // The scan stops at the first '(' or '{'. An assignment target is a
        // name, a field or an index, so its '=' comes before either of those;
        // everything past one belongs to a call's arguments or a literal's
        // body, where the '=' of a `let` inside a closure passed as an
        // argument says nothing about the line holding it.
        _ => {
            let assigns = tokens
                .take_while(|t| !matches!(t, Ok(Token::LParen | Token::LBrace)))
                .any(|t| {
                    matches!(
                        t,
                        Ok(Token::Equals
                            | Token::AssignOpAdd
                            | Token::AssignOpSub
                            | Token::AssignOpMul
                            | Token::AssignOpDiv
                            | Token::AssignOpMod
                            | Token::AssignOpPow
                            | Token::AssignOpBitAnd
                            | Token::AssignOpBitOr
                            | Token::AssignOpBitXor
                            | Token::AssignOpShl
                            | Token::AssignOpShr)
                    )
                });
            if assigns {
                LineKind::Statement
            } else {
                LineKind::Expression
            }
        }
    }
}

/// How a line at the prompt writes the way out of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitLine {
    /// The name on its own. It is not a call, so there is nothing to compile
    /// and nothing to work a status out of.
    Bare,
    /// A call, with an argument or without one. The status is whatever the
    /// call names, which the session it is typed into is what works out.
    Call,
}

/// Whether the only statement `line` holds is a call to `exit`, and in which
/// of its two forms.
///
/// The prompt answers `exit` itself: the built-in of that name ends the
/// process compiling the session, so a session holding the line would print
/// nothing ever again. Reading the line from its tokens is what lets
/// `exit(code)` leave with the value of `code` rather than only the forms
/// whose status is written out.
#[must_use]
pub fn exit_line(line: &str) -> Option<ExitLine> {
    let mut tokens = Token::lexer(line);
    if !matches!(tokens.next(), Some(Ok(Token::Identifier("exit")))) {
        return None;
    }
    let form = match tokens.next() {
        None => return Some(ExitLine::Bare),
        Some(Ok(Token::SemiColon)) => ExitLine::Bare,
        Some(Ok(Token::LParen)) => {
            // The argument can be any expression, parentheses of its own
            // included, so the list ends where the depth comes back to zero.
            let mut depth = 1u32;
            loop {
                match tokens.next() {
                    Some(Ok(Token::LParen)) => depth += 1,
                    Some(Ok(Token::RParen)) => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    // Unbalanced, or a token the lexer refuses: the line is
                    // one to compile, and it reports what its own text earns.
                    _ => return None,
                }
            }
            match tokens.next() {
                None => return Some(ExitLine::Call),
                Some(Ok(Token::SemiColon)) => ExitLine::Call,
                _ => return None,
            }
        }
        _ => return None,
    };
    // Anything after the statement makes the line more than the way out.
    tokens.next().is_none().then_some(form)
}
