// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use super::ParserErr;
use super::lexer::Token;
use super::lexer::parse_string;
use super::parser_expr::parse_expr;
use super::parser_expr::parse_expr_no_struct;
use super::parser_expr::range_literal_ends;
use crate::cold_path;
use crate::compiler::attributes::AttrOn;
use crate::compiler::attributes::AttributeTargetKind;
use crate::compiler::attributes::AttributeUse;
use crate::compiler::expr::Expr;
use crate::compiler::expr::MatchArm;
use crate::compiler::expr::Operand;
use crate::compiler::expr::ParamDefaults;
use crate::compiler::expr::PatternLet;
use crate::compiler::expr::RECEIVER;
use crate::compiler::expr::Span;
use crate::compiler::expr::UNARY_MINUS_METHOD;
use crate::compiler::expr::mangle_method;
use crate::compiler::patterns::Pattern;
use crate::compiler::type_system::ImplTemplate;
use crate::compiler::type_system::ReturnAnnotation;
use crate::parser::BlockEnd;
use crate::parser::Parser;
use crate::parser::Tail;
use crate::parser::TypeExpr;
use crate::parser::parse_attributes;
use crate::parser::parse_code;
use crate::parser::parse_jump;
use crate::parser::parse_type;
use crate::parser::parse_type_args;
use crate::parser::parse_type_params;
use crate::parser::pattern::parse_pattern;
use smol_strc::SmolStr;
use std::rc::Rc;

/// Parses a condition and reports the span it covers, which is what a
/// condition whose type is not `bool` is reported against.
pub fn parse_condition_expr(parser: &mut Parser<'_>) -> (Expr, Span) {
    let start = parser.peek_token_span().start;
    let condition = parse_expr_no_struct(parser);
    (condition, (start, parser.last_token_end as u32).into())
}

// call right after peeking Token::If
/// Reads an `if` statement and how it ends the block it is in.
///
/// As the last statement of a block used as a value (`tail`), an `if` whose
/// branches end in values is that block's value: it reads as an `if` used as
/// a value, so it has an `else`, and every branch gives a value or leaves.
pub fn parse_condition_block(parser: &mut Parser<'_>, start: u32, tail: Tail) -> (Expr, BlockEnd) {
    let t = parser.next_token();
    debug_assert_eq!(t.0, Token::If);
    let branch_tail = if tail == Tail::Value {
        Tail::Value
    } else {
        Tail::No
    };
    let (condition, condition_span) = parse_condition_expr(parser);
    let (mut output_code, first_end, _) = parse_block_in(parser, branch_tail);
    let mut ends = vec![first_end];
    let mut has_else = false;
    loop {
        let next_token = parser.peek_token_opt();
        if next_token != Some(Token::Else) {
            break;
        }
        parser.next_token();
        // if -> else if
        // lbrace -> else
        // else -> end
        let next_token = parser.peek_token_opt();
        if next_token == Some(Token::If) {
            parser.next_token();
            let (else_if_condition, else_if_condition_span) = parse_condition_expr(parser);
            let (else_if_code, end, _) = parse_block_in(parser, branch_tail);
            ends.push(end);
            output_code.push(Expr::ElseIfBlock(
                Box::new(else_if_condition),
                Box::from(else_if_code),
                else_if_condition_span,
            ));
        } else if next_token == Some(Token::LBrace) {
            let (else_code, end, _) = parse_block_in(parser, branch_tail);
            ends.push(end);
            has_else = true;
            output_code.push(Expr::ElseBlock(Box::from(else_code)));
            break;
        } else {
            break;
        }
    }
    let span: Span = (start, parser.last_token_end as u32).into();
    if let Some(at) = value_end(&ends) {
        if !has_else {
            cold_path();
            parser.error(span, ParserErr::InlineConditionNoElseBlock);
        }
        return as_tail(
            parser,
            Expr::InlineCondition(
                Box::new(condition),
                Box::from(output_code),
                span,
                condition_span,
            ),
            &ends,
            at,
            span,
        );
    }
    let statement = Expr::Condition(
        Box::new(condition),
        Box::from(output_code),
        span,
        condition_span,
    );
    let end = BlockEnd::of(&statement);
    (statement, end)
}

/// Where the first of `ends` that is a value leaves its `;` off, if any is.
fn value_end(ends: &[BlockEnd]) -> Option<u32> {
    ends.iter().find_map(|end| match end {
        BlockEnd::Value(at) => Some(*at),
        _ => None,
    })
}

/// A construct one of whose branches ended in a value, which makes it the
/// value of the block it is the last statement of. Followed by anything but
/// the block's `}`, the value is a statement missing its `;`. Every branch
/// ends in a value or leaves.
fn as_tail(
    parser: &mut Parser<'_>,
    value: Expr,
    ends: &[BlockEnd],
    at: u32,
    span: Span,
) -> (Expr, BlockEnd) {
    if parser.peek_token_opt() != Some(Token::RBrace) {
        cold_path();
        crate::parser::error_missing_semicolon_at(parser, at, None);
    }
    if ends.contains(&BlockEnd::Statement) {
        cold_path();
        parser.error(span, ParserErr::ValueBlockWithoutValue);
    }
    (value, BlockEnd::Value(parser.last_token_end as u32))
}

/// LBrace Code RBrace
#[inline(always)]
pub fn parse_block(parser: &mut Parser<'_>) -> Vec<Expr> {
    parse_block_in(parser, Tail::No).0
}

/// A block used as a value, `{ let t = 5; t * 2 }`: its statements, the last
/// of which is its value or leaves (`return`, `throw`, `break`, `continue`).
pub fn parse_value_block(parser: &mut Parser<'_>) -> Vec<Expr> {
    let (code, end, span) = parse_block_in(parser, Tail::Value);
    if end == BlockEnd::Statement {
        cold_path();
        parser.error(span, ParserErr::ValueBlockWithoutValue);
    }
    code
}

/// Reads a block, `{` to `}`, and how its last statement ends it, and the
/// span of the block.
pub fn parse_block_in(parser: &mut Parser<'_>, tail: Tail) -> (Vec<Expr>, BlockEnd, Span) {
    let opener_token_span =
        parser.next_token_expect(Token::LBrace, "Blocks need to start with '{'");
    parse_block_rest(parser, opener_token_span, tail)
}

/// [`parse_value_block`] once its `{`, read at `opener`, is.
pub fn parse_value_block_after(parser: &mut Parser<'_>, opener: Span) -> Vec<Expr> {
    let (code, end, span) = parse_block_rest(parser, opener, Tail::Value);
    if end == BlockEnd::Statement {
        cold_path();
        parser.error(span, ParserErr::ValueBlockWithoutValue);
    }
    code
}

/// [`parse_block_in`] once the `{`, read at `opener_token_span`, is.
fn parse_block_rest(
    parser: &mut Parser<'_>,
    opener_token_span: Span,
    tail: Tail,
) -> (Vec<Expr>, BlockEnd, Span) {
    parser.enter(opener_token_span);
    let (code, end) = parse_code(parser, tail);
    parser.leave();
    let close = parser.next_token_expect_closer(Token::LBrace, opener_token_span, Token::RBrace);
    (code, end, (opener_token_span.start, close).into())
}

/// The body of a function: a block, or `=> expr` for one that evaluates to a
/// single expression. The arrow form is the block `{ return expr; }`, so the
/// rest of the compiler sees one shape of body. A declaration ends its arrow
/// body with `;`, the way a statement ends; an anonymous function's arrow
/// body ends where the expression does.
///
/// A block body hands back its value with `return`; unlike a block used as a
/// value, it does not end in one.
///
/// The `let`s that take apart the parameters written as patterns open it.
pub fn parse_fn_body(parser: &mut Parser<'_>, declaration: bool) -> Vec<Expr> {
    let mut code = std::mem::take(&mut parser.param_lets);
    // A loop around an anonymous function is not one its body can leave.
    let loops = std::mem::take(&mut parser.loops);
    code.extend(parse_fn_body_in(parser, declaration));
    parser.loops = loops;
    code
}

fn parse_fn_body_in(parser: &mut Parser<'_>, declaration: bool) -> Vec<Expr> {
    if parser.peek_token_opt() != Some(Token::FatArrow) {
        return parse_block_in(parser, Tail::FnBody).0;
    }
    parser.next_token();
    let value = parse_expr(parser);
    if declaration {
        parser.next_token_expect(
            Token::SemiColon,
            "A function declared with `=>` ends its expression with ';'.",
        );
    }
    vec![Expr::ReturnVal(Box::new(Some(value)))]
}

/// The block a loop repeats, where `break` and `continue` reach the loop.
fn parse_loop_body(parser: &mut Parser<'_>) -> Vec<Expr> {
    parser.loops += 1;
    let code = parse_block(parser);
    parser.loops -= 1;
    code
}

pub fn parse_while_block(input: &mut Parser<'_>) -> Expr {
    let t = input.next_token();
    debug_assert_eq!(t.0, Token::While);
    let (while_condition, condition_span) = parse_condition_expr(input);
    let while_code = parse_loop_body(input);
    Expr::WhileBlock(
        Box::new(while_condition),
        Box::from(while_code),
        condition_span,
    )
}

/// The rest of a `for` whose binding is a pattern, the pattern read: the
/// element goes into a variable no program can name, and the body opens with
/// the `let` that takes it apart.
fn parse_for_pattern(parser: &mut Parser<'_>, pattern: Pattern) -> Expr {
    let pattern_span = pattern.span();
    let element = SmolStr::from(format!("[for {} {}]", pattern_span.start, pattern_span.end));
    parser.next_token_expect(Token::In, "");
    let start = parser.peek_token_span().start;
    let for_collection = parse_expr_no_struct(parser);
    let end = parser.last_token_end as u32;
    let mut code = vec![Expr::LetPattern(Box::new(PatternLet {
        pattern,
        value: Expr::Var(element.clone(), pattern_span),
        annotation: None,
        otherwise: None,
        span: pattern_span,
    }))];
    code.extend(parse_loop_body(parser));
    Expr::ForLoop(
        element,
        Box::new(for_collection),
        Box::from(code),
        (start, end).into(),
    )
}

/// Parses ForLoop and IntForLoop
/// for Identifier in Expr LBrace Code RBrace
/// for Identifier in Expr RangeDot Expr LBrace Code Rbrace
/// for Identifier in RangeDot Expr LBrace Code RBrace
pub fn parse_for_loop(parser: &mut Parser<'_>) -> Expr {
    let t = parser.next_token();
    debug_assert_eq!(t.0, Token::For);
    // A name alone binds, whatever its case.
    let pattern = match parser.peek_token() {
        Token::Identifier(name)
            if name != "_"
                && parser
                    .input
                    .clone()
                    .nth(1)
                    .is_some_and(|(t, _)| t == Ok(Token::In)) =>
        {
            let span = parser.next_token().1;
            Pattern::Bind(SmolStr::new(name), span)
        }
        _ => parse_pattern(parser, true),
    };
    // `for (a, b) in xs`: the element goes into a variable no program can
    // name, and the body opens with `let (a, b) = element;`.
    let id = match pattern {
        Pattern::Bind(id, _) => id,
        Pattern::Wildcard(_) => SmolStr::new_static("_"),
        pattern => return parse_for_pattern(parser, pattern),
    };
    parser.next_token_expect(Token::In, "");
    let start = parser.peek_token_span().start;
    let peek_token = parser.peek_token();
    if peek_token == Token::RangeDot {
        // shorthand IntForLoop
        parser.next_token();
        let start2 = parser.peek_token_span().start;
        let upper_bound = parse_expr_no_struct(parser);
        let end2 = parser.last_token_end as u32;
        let for_loop_code = parse_loop_body(parser);
        Expr::IntForLoop(
            id,
            Box::new(Expr::Int(0)),
            Box::new(upper_bound),
            Box::from(for_loop_code),
            (start, start).into(),
            (start2, end2).into(),
        )
    } else {
        let for_collection = parse_expr_no_struct(parser);
        let end = parser.last_token_end as u32;
        // `for i in a..b` counts in a register and builds no range.
        if let Some((low, high)) = range_literal_ends(&for_collection) {
            let for_loop_code = parse_loop_body(parser);
            Expr::IntForLoop(
                id,
                Box::new(low.clone()),
                Box::new(high.clone()),
                Box::from(for_loop_code),
                (start, start).into(),
                (start, end).into(),
            )
        } else {
            let for_loop_code = parse_loop_body(parser);
            Expr::ForLoop(
                id,
                Box::new(for_collection),
                Box::from(for_loop_code),
                (start, end).into(),
            )
        }
    }
}

/// A block written where a statement goes. As the last statement of a block
/// used as a value (`tail`), one that ends in a value is that block's value.
pub fn parse_eval_block(parser: &mut Parser<'_>, tail: Tail) -> (Expr, BlockEnd) {
    let inner = if tail == Tail::Value {
        Tail::Value
    } else {
        Tail::No
    };
    let (code, end, span) = parse_block_in(parser, inner);
    let block = Expr::EvalBlock(Box::from(code));
    match end {
        BlockEnd::Value(at) => as_tail(parser, block, &[end], at, span),
        _ => (block, end),
    }
}

pub fn parse_function(parser: &mut Parser<'_>) -> Expr {
    let (t, _) = parser.next_token();
    debug_assert_eq!(t, Token::Function);
    let (t_fn_id, span) = parser.next_token();
    let fn_name = if let Token::Identifier(fn_name) = t_fn_id {
        SmolStr::new(fn_name)
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(Token::Identifier(""), t_fn_id, "Invalid function name."),
        );
    };
    let type_params = parse_type_params(parser);
    let (args, defaults) = parse_params(parser, false);
    let return_type = parse_return_annotation(parser);
    let fn_code = parse_fn_body(parser, true);
    Expr::FunctionDecl(
        fn_name,
        Box::from(args),
        std::rc::Rc::from(fn_code),
        span,
        return_type,
        type_params,
        defaults,
    )
}

/// Reads the attributes in front of a field, a variant or a parameter. Code a
/// `@cfg` among them turns off is parsed as inactive until [`end_part`].
fn parse_part_attributes(parser: &mut Parser<'_>) -> (bool, Vec<AttributeUse>) {
    if parser.peek_token() != Token::At {
        return (true, Vec::new());
    }
    let (active, uses) = parse_attributes(parser);
    if !active {
        parser.inactive += 1;
    }
    (active, uses)
}

/// Ends a part [`parse_part_attributes`] opened: a part a `@cfg` turned off is
/// dropped with `drop_part`, and the attributes on one that stays are
/// recorded on it.
fn end_part(
    parser: &mut Parser<'_>,
    active: bool,
    uses: Vec<AttributeUse>,
    on: AttrOn,
    drop_part: impl FnOnce(),
) {
    if !active {
        parser.inactive -= 1;
        drop_part();
    } else if !uses.is_empty() {
        parser.part_uses.push((on, uses.into_boxed_slice()));
    }
}

/// Parses a declaration's parameter list, `(a, b: int)`, from its `(`.
///
/// `self` names the receiver: it is a parameter only of an `impl` function
/// (`in_impl`), and only the first one, which is what makes that function a
/// method.
///
/// A parameter may declare a default, `size: int = 12`, which a call that
/// leaves the argument out passes instead. The defaults come back one per
/// parameter, or empty when none declares one.
pub fn parse_params(
    parser: &mut Parser<'_>,
    in_impl: bool,
) -> (Vec<(SmolStr, Option<TypeExpr>)>, ParamDefaults) {
    parser.next_token_expect(
        Token::LParen,
        "Function parameters must be delimited by parentheses",
    );
    let mut args: Vec<(SmolStr, Option<TypeExpr>)> = Vec::with_capacity(4);
    let mut defaults: Vec<Option<(Expr, Span)>> = Vec::new();
    let mut lets: Vec<Expr> = Vec::new();
    loop {
        if parser.peek_token() == Token::RParen {
            parser.next_token();
            break;
        }
        let (active, uses) = parse_part_attributes(parser);
        let kept = (args.len(), defaults.len(), lets.len());
        // A parameter that is a pattern, `(a, b): (int, int)`, takes the
        // argument under a name no program can write, and the body opens
        // with the `let` that takes it apart.
        let plain_name = matches!(parser.peek_token(), Token::Identifier(name) if name != "_")
            && parser.input.clone().nth(1).is_some_and(|(t, _)| {
                matches!(
                    t,
                    Ok(Token::Colon | Token::Comma | Token::RParen | Token::Equals)
                )
            });
        if !plain_name {
            let pattern = parse_pattern(parser, true);
            let span = pattern.span();
            let name = SmolStr::from(format!("[param {}]", args.len()));
            lets.push(Expr::LetPattern(Box::new(PatternLet {
                pattern,
                value: Expr::Var(name.clone(), span),
                annotation: None,
                otherwise: None,
                span,
            })));
            let ty = if parser.peek_token() == Token::Colon {
                parser.next_token();
                Some(parse_type(parser))
            } else {
                None
            };
            args.push((name, ty));
            end_part(parser, active, uses, AttrOn::Param(kept.0 as u16), || {
                args.truncate(kept.0);
                lets.truncate(kept.2);
            });
            if parser.peek_token() == Token::Comma {
                parser.next_token();
            } else if parser.peek_token() != Token::RParen {
                cold_path();
                let span = parser.peek_token_span();
                parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
            }
            continue;
        }
        let (arg, span) = parser.next_token();
        if let Token::Identifier(arg) = arg {
            if arg == RECEIVER && (!in_impl || !args.is_empty()) {
                cold_path();
                parser.error(span, ParserErr::MisplacedReceiver(in_impl));
            }
            args.push((
                SmolStr::new(arg),
                if parser.peek_token() == Token::Colon {
                    parser.next_token();
                    Some(parse_type(parser))
                } else {
                    None
                },
            ));
            if parser.peek_token() == Token::Equals {
                parser.next_token();
                let start = parser.peek_token_span().start;
                let value = parse_expr(parser);
                defaults.resize(args.len() - 1, None);
                defaults.push(Some((value, (start, parser.last_token_end as u32).into())));
            }
            end_part(parser, active, uses, AttrOn::Param(kept.0 as u16), || {
                args.truncate(kept.0);
                defaults.truncate(kept.1.min(kept.0));
            });
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(Token::Identifier(""), arg, "Invalid parameter."),
            );
        }
        if parser.peek_token() == Token::Comma {
            parser.next_token();
        } else if parser.peek_token() != Token::RParen {
            cold_path();
            let span = parser.peek_token_span();
            parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
        }
    }
    if !defaults.is_empty() {
        defaults.resize(args.len(), None);
    }
    parser.param_lets = lets;
    (args, defaults.into_boxed_slice())
}

/// Parses the optional `-> Type` return annotation that may follow a function's
/// parameter list, returning it with the span of the annotated type.
///
/// The annotation is checked against what the body returns; see
/// `compile_function`. Leaving it off keeps the return type inferred.
/// `const NAME: Type = value;` at the top of a file: a function of no
/// arguments, with the declared type as its return type, that a bare `NAME`
/// reads. The `const` keyword is not yet read.
pub fn parse_const(parser: &mut Parser<'_>) -> Expr {
    parser.next_token();
    let (t, name_span) = parser.next_token();
    let Token::Identifier(name) = t else {
        cold_path();
        parser.error(
            name_span,
            ParserErr::UnexpectedToken(Token::Identifier(""), t, "A constant needs a name."),
        );
    };
    parser.next_token_expect(
        Token::Colon,
        "A constant declares its type: const NAME: Type = value;",
    );
    let type_start = parser.peek_token_span().start;
    let ty = parse_type(parser);
    let type_span: Span = (type_start, parser.last_token_end as u32).into();
    parser.next_token_expect(
        Token::Equals,
        "A constant needs a value: const NAME: Type = value;",
    );
    let value = parse_expr(parser);
    parser.next_token_expect(Token::SemiColon, "A constant ends with ';'.");
    Expr::FunctionDecl(
        SmolStr::new(name),
        Box::from([]),
        Rc::from([Expr::ReturnVal(Box::new(Some(value)))]),
        name_span,
        Some(Box::new((ty, type_span))),
        Box::from([]),
        Box::from([]),
    )
}

fn parse_return_annotation(parser: &mut Parser<'_>) -> ReturnAnnotation {
    if parser.peek_token() != Token::Arrow {
        return None;
    }
    parser.next_token();
    let type_start = parser.peek_token_span().start;
    let return_type = parse_type(parser);
    Some(Box::new((
        return_type,
        (type_start, parser.last_token_end as u32).into(),
    )))
}

pub fn parse_try_catch_block(parser: &mut Parser<'_>) -> Expr {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Try);
    let try_code = parse_block(parser);
    let mut has_catch = false;
    let mut catch_blocks: Vec<(SmolStr, Vec<Expr>)> = Vec::with_capacity(1);
    let mut catch_all_var = SmolStr::new_static("e");
    let mut catch_all_code = None;
    let end: u32;
    loop {
        let token_peek = parser.peek_token();
        if token_peek != Token::Catch {
            end = parser.peek_token_span().end;
            break;
        }
        parser.next_token();
        let (next_token, next_span) = parser.next_token();
        if let Token::Identifier(i) = next_token {
            // catch-all
            catch_all_var = SmolStr::new(i);
            catch_all_code = Some(parse_block(parser));
            end = parser.peek_token_span().start;
            has_catch = true;
            break;
        } else if let Token::String(s) = next_token {
            catch_blocks.push((SmolStr::new(parse_string(s)), parse_block(parser)));
            has_catch = true;
        } else {
            cold_path();
            parser.error(
                next_span,
                ParserErr::UnexpectedTokenStr(
                    "a quoted error kind or a name",
                    next_token,
                    "A catch handles one kind, `catch \"kind\" { ... }`, or binds any kind to a name, `catch e { ... }`.",
                ),
            );
        }
    }
    if !has_catch {
        cold_path();
        parser.error((start, end).into(), ParserErr::TryBlockNoCatch);
    }
    let usr_var = Expr::Var(catch_all_var.clone(), (start, end).into());
    let else_code: Box<[Expr]> = if let Some(c) = catch_all_code {
        Box::from(c)
    } else {
        Box::from([Expr::FunctionCall(
            Box::new([usr_var]),
            Box::from([SmolStr::new("throw")]),
            (start, end).into(),
            Box::from([]),
            Box::from([]),
        )])
    };

    if catch_blocks.is_empty() {
        return Expr::TryCatchBlock(Box::from(try_code), catch_all_var, else_code);
    }

    let mut output_code: Vec<Expr> = Vec::with_capacity(2);
    let mut main_condition = Expr::Null;

    let catch_span: Span = (start, end).into();
    let mut first = true;
    for (e, c) in catch_blocks {
        if first {
            first = false;
            main_condition = Expr::Eq(
                Operand::new(Expr::String(e)),
                Operand::new(Expr::Var(catch_all_var.clone(), catch_span)),
                catch_span,
                catch_span,
            );
            output_code.extend(c);
        } else {
            output_code.push(Expr::ElseIfBlock(
                Box::new(Expr::Eq(
                    Operand::new(Expr::String(e)),
                    Operand::new(Expr::Var(catch_all_var.clone(), catch_span)),
                    catch_span,
                    catch_span,
                )),
                Box::from(c),
                catch_span,
            ));
        }
    }
    output_code.push(Expr::ElseBlock(else_code));
    Expr::TryCatchBlock(
        Box::from(try_code),
        catch_all_var,
        Box::from([Expr::Condition(
            Box::from(main_condition),
            Box::from(output_code),
            catch_span,
            catch_span,
        )]),
    )
}

pub fn parse_struct_declare(parser: &mut Parser<'_>) -> Expr {
    let (t, _) = parser.next_token();
    debug_assert_eq!(t, Token::Struct);
    let (next_token, span) = parser.next_token();
    let struct_name = if let Token::Identifier(id) = next_token {
        SmolStr::new(id)
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(Token::Identifier(""), next_token, ""),
        );
    };
    let type_params = parse_type_params(parser);
    parser.next_token_expect(Token::LBrace, "Expected '{'");
    let mut fields: Vec<(SmolStr, TypeExpr, Span)> = Vec::with_capacity(4);
    let mut defaults: Vec<Option<(Expr, Span)>> = Vec::with_capacity(4);
    // `struct Tag {}`: a struct with no fields, such as a marker.
    let empty = parser.peek_token() == Token::RBrace;
    if empty {
        parser.next_token();
    }
    loop {
        if empty {
            break;
        }
        let (active, uses) = parse_part_attributes(parser);
        let (next_token, _) = parser.next_token();
        let field_name = if let Token::Identifier(i) = next_token {
            SmolStr::new(i)
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    next_token,
                    "Struct field names must be identifiers.",
                ),
            );
        };
        parser.next_token_expect(Token::Colon, "A colon must separate a field from its type.");
        let field_type_start = parser.peek_token_span().start;
        let field_type = parse_type(parser);
        let field_type_end = parser.peek_token_span().end;
        fields.push((
            field_name,
            field_type,
            (field_type_start, field_type_end).into(),
        ));
        // `name: Type = value` gives the field the value `S::default()` and a
        // bare `..` fill it with.
        if parser.peek_token() == Token::Equals {
            parser.next_token();
            let value_start = parser.peek_token_span().start;
            let value = parse_expr(parser);
            let value_end = parser.last_token_end as u32;
            defaults.push(Some((value, (value_start, value_end).into())));
        } else {
            defaults.push(None);
        }
        end_part(
            parser,
            active,
            uses,
            AttrOn::Field(fields.len() as u16 - 1),
            || {
                fields.pop();
                defaults.pop();
            },
        );
        let (next_token, span) = parser.next_token();
        if next_token == Token::RBrace {
            break;
        } else if next_token != Token::Comma {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Comma,
                    next_token,
                    "In structs, fields must be separated by a comma.",
                ),
            );
        } else if parser.peek_token() == Token::RBrace {
            parser.next_token();
            break;
        }
    }
    let defaults: Box<[Option<(Expr, Span)>]> = if defaults.iter().any(Option::is_some) {
        Box::from(defaults)
    } else {
        Box::from([])
    };
    Expr::StructDeclare(struct_name, Box::from(fields), span, type_params, defaults)
}

/// Parses an `impl Type { fn method(self, ...) { ... } ... }` block.
///
/// Each method is lowered on the spot to an ordinary top-level function whose
/// name is mangled per type (`Type#method`, see [`mangle_method`]) and whose
/// first parameter is the receiver. There is no dedicated `impl`/method AST
/// node: the lowered [`Expr::FunctionDecl`]s are pushed straight into `output`,
/// so downstream compilation treats a method exactly like a free function and
/// the VM only ever sees ordinary calls. The receiver's static type resolves a
/// `recv.method(...)` call site back to the matching mangled symbol.
///
/// A block whose header names type arguments (`impl<T> Cell<T>`, `impl Cell<int>`)
/// has no instantiated type to mangle against yet: it is kept as a template in
/// `impls` and lowered the same way once the type is instantiated.
pub fn parse_impl_block(
    parser: &mut Parser<'_>,
    output: &mut Vec<Expr>,
    impls: &mut Vec<ImplTemplate>,
) {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Impl);
    let params_start = parser.peek_token_span().start;
    let params = parse_type_params(parser);
    let params_span: Span = (params_start, parser.last_token_end as u32).into();
    let (next_token, type_span) = parser.next_token();
    let type_name = if let Token::Identifier(id) = next_token {
        SmolStr::new(id)
    } else {
        cold_path();
        parser.error(
            type_span,
            ParserErr::UnexpectedToken(
                Token::Identifier(""),
                next_token,
                "An impl block must name the type it implements methods for.",
            ),
        );
    };
    let type_args: Box<[TypeExpr]> = if parser.peek_token() == Token::OpInf {
        parse_type_args(parser)
    } else {
        Box::from([])
    };
    // Every parameter the block declares is one of the type's arguments,
    // which is what binds it: `impl<T> Cell<T>`.
    if let Some(unused) = params.iter().find(|param| {
        !type_args
            .iter()
            .any(|arg| matches!(arg, TypeExpr::Identifier(name, _) if name == *param))
    }) {
        cold_path();
        parser.error(params_span, ParserErr::ImplParamUnbound(unused.to_string()));
    }
    let header_span: Span = (start, parser.last_token_end as u32).into();
    parser.next_token_expect(Token::LBrace, "impl blocks must start with '{'.");
    let mut methods: Vec<Expr> = Vec::with_capacity(4);
    // The attributes on each method and on its parameters, by the method's
    // index in `methods`.
    let mut uses: Vec<(usize, AttrOn, Box<[AttributeUse]>)> = Vec::new();
    loop {
        if parser.peek_token() == Token::RBrace {
            parser.next_token();
            break;
        }
        let (active, method_uses) = if parser.peek_token() == Token::At {
            let run = parse_attributes(parser);
            if parser.peek_token() == Token::RBrace {
                cold_path();
                let span = parser.peek_token_span();
                parser.error(span, ParserErr::AttributeWithoutItem);
            }
            run
        } else {
            (true, Vec::new())
        };
        if active {
            let index = methods.len();
            methods.push(parse_method(parser));
            if !method_uses.is_empty() {
                uses.push((index, AttrOn::Item, method_uses.into_boxed_slice()));
            }
            uses.extend(parser.part_uses.drain(..).map(|(on, u)| (index, on, u)));
        } else {
            parser.inactive += 1;
            let _ = parse_method(parser);
            parser.inactive -= 1;
            parser.part_uses.clear();
        }
    }
    let generic = !type_args.is_empty();
    parser.impl_on = Some(if generic {
        AttrOn::Generic(AttributeTargetKind::Impl)
    } else {
        AttrOn::Impl(TypeExpr::Identifier(type_name.clone(), type_span))
    });
    let first = output.len();
    for (index, on, run) in uses {
        let on = match on {
            AttrOn::Item if generic => AttrOn::Generic(AttributeTargetKind::Method),
            AttrOn::Param(_) if generic => AttrOn::Generic(AttributeTargetKind::Param),
            on => on,
        };
        parser.impl_uses.push((first + index, on, run));
    }
    if generic {
        impls.push(ImplTemplate {
            type_name,
            params,
            args: type_args,
            methods: Box::from(methods),
            // Stamped with the file it came from when it is registered.
            file_idx: 0,
            span: header_span,
        });
    } else {
        for method in methods {
            output.push(mangled_method(method, &type_name));
        }
    }
}

/// Renames a parsed method to the mangled free-function symbol its call sites
/// resolve to.
fn mangled_method(method: Expr, type_name: &SmolStr) -> Expr {
    let Expr::FunctionDecl(name, args, code, name_span, return_type, type_params, defaults) =
        method
    else {
        cold_path();
        unreachable!("an impl block only ever parses method declarations")
    };
    Expr::FunctionDecl(
        mangle_method(type_name, &name),
        args,
        code,
        name_span,
        return_type,
        type_params,
        defaults,
    )
}

/// The operator a method declaration names, for `fn +(self, other)` inside an
/// `impl` block.
///
/// `Ok` is an operator a type defines. `Err` is an operator token that cannot
/// be a method name, carried so the report can name it and say what derives it
/// where something does. A token that is not an operator at all answers `None`
/// and the name has to be an identifier, which is what `fn` takes everywhere
/// else in the language.
const fn operator_method_name(token: Token<'_>) -> Option<Result<&'static str, &'static str>> {
    Some(match token {
        Token::OpAdd => Ok("+"),
        Token::OpSub => Ok("-"),
        Token::OpMul => Ok("*"),
        Token::OpDiv => Ok("/"),
        Token::OpMod => Ok("%"),
        Token::OpPow => Ok("^"),
        Token::OpBitAnd => Ok("&"),
        Token::Pipe => Ok("|"),
        Token::OpBitXor => Ok("^^"),
        Token::OpShl => Ok("<<"),
        Token::OpShr => Ok(">>"),
        Token::OpEq => Ok("=="),
        Token::OpInf => Ok("<"),
        Token::OpInfEq => Ok("<="),
        Token::OpBitNot => Ok("~"),
        Token::OpNEq => Err("!="),
        Token::OpSup => Err(">"),
        Token::OpSupEq => Err(">="),
        Token::OpNot => Err("!"),
        Token::OpAnd => Err("&&"),
        Token::OpOr => Err("||"),
        Token::Equals => Err("="),
        Token::AssignOpAdd => Err("+="),
        Token::AssignOpSub => Err("-="),
        Token::AssignOpMul => Err("*="),
        Token::AssignOpDiv => Err("/="),
        Token::AssignOpMod => Err("%="),
        Token::AssignOpPow => Err("^="),
        Token::AssignOpBitAnd => Err("&="),
        Token::AssignOpBitOr => Err("|="),
        Token::AssignOpBitXor => Err("^^="),
        Token::AssignOpShl => Err("<<="),
        Token::AssignOpShr => Err(">>="),
        _ => return None,
    })
}

/// Parses a single `fn method(self, ...) [-> Type] { ... }` inside an impl block
/// into an [`Expr::FunctionDecl`] carrying the plain method name.
///
/// The name is an identifier or, here alone, an operator symbol: a method
/// named `+` is what `a + b` reaches when `a` is of the type the block
/// implements. An operator method takes no type parameters of its own, so a
/// `<` after the symbol is read as the start of the parameter list and
/// reported there.
fn parse_method(parser: &mut Parser<'_>) -> Expr {
    let (t, t_span) = parser.next_token();
    if t != Token::Function {
        cold_path();
        parser.error(
            t_span,
            ParserErr::UnexpectedToken(
                Token::Function,
                t,
                "impl blocks may only contain method definitions ('fn ...').",
            ),
        );
    }
    let (t_id, name_span) = parser.next_token();
    let operator = operator_method_name(t_id);
    let method_name = match (operator, t_id) {
        (Some(Ok(symbol)), _) => symbol,
        (Some(Err(symbol)), _) => {
            cold_path();
            parser.error(name_span, ParserErr::OperatorMethodUnknown(symbol));
        }
        (None, Token::Identifier(name)) => name,
        (None, other) => {
            cold_path();
            parser.error(
                name_span,
                ParserErr::UnexpectedToken(Token::Identifier(""), other, "Invalid method name."),
            );
        }
    };
    let type_params = if operator.is_some() {
        Box::from([])
    } else {
        parse_type_params(parser)
    };
    // A parameter left un-annotated (including the receiver) is inferred per
    // call site: candela specialises each method on the actual argument types,
    // so `self` takes the receiver's concrete struct type automatically. An
    // annotation pins the parameter instead, exactly as it does on a free
    // function.
    let (args, defaults) = parse_params(parser, true);
    let return_type = parse_return_annotation(parser);
    let code = parse_fn_body(parser, true);
    // `-` has a binary and a unary form and a type may define both, so the
    // parameter count picks which name this declaration takes. A count that
    // fits neither keeps the symbol it was written with, and the arity check
    // reports it against that symbol.
    let method_name = if method_name == "-" && args.len() == 1 {
        UNARY_MINUS_METHOD
    } else {
        method_name
    };
    Expr::FunctionDecl(
        SmolStr::new(method_name),
        Box::from(args),
        Rc::from(code),
        name_span,
        return_type,
        type_params,
        defaults,
    )
}

pub fn parse_loop_block(input: &mut Parser<'_>) -> Expr {
    let (t, _) = input.next_token();
    debug_assert_eq!(t, Token::Loop);
    Expr::LoopBlock(Box::from(parse_loop_body(input)))
}

/// Parses a `match` statement into an [`Expr::Match`]: the scrutinee, the arm
/// patterns (as raw expressions) and their bodies, and an optional wildcard
/// body. The compiler picks the lowering by the scrutinee's static type: an
/// enum scrutinee gives variant-pattern matching with payload binding; any
/// other scrutinee gives the equality-chain behavior.
///
/// As the last statement of a block used as a value (`tail`), a `match` whose
/// arms give values is that block's value, the way a `match` written where a
/// value goes is.
pub fn parse_match(parser: &mut Parser<'_>, tail: Tail) -> (Expr, BlockEnd) {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Match);
    let mode = if tail == Tail::Value {
        MatchMode::MaybeValue
    } else {
        MatchMode::Statement
    };
    let (statement, ends) = parse_match_in(parser, start, mode);
    let Expr::Match(scrutinee, arms, wildcard, span, _) = statement else {
        unreachable!()
    };
    let braced_value = value_end(&ends.iter().filter_map(|(_, end)| *end).collect::<Vec<_>>());
    let is_last = parser.peek_token_opt() == Some(Token::RBrace);
    if tail == Tail::Value && is_last {
        // Each arm written as an expression is a value too, here.
        let as_values: Vec<BlockEnd> = ends
            .iter()
            .map(|(expr_end, braced)| braced.unwrap_or(*expr_end))
            .collect();
        if let Some(at) = value_end(&as_values)
            && (braced_value.is_some() || !as_values.contains(&BlockEnd::Statement))
        {
            return as_tail(
                parser,
                Expr::Match(scrutinee, arms, wildcard, span, true),
                &as_values,
                at,
                span,
            );
        }
    } else if let Some(at) = braced_value {
        cold_path();
        crate::parser::error_missing_semicolon_at(parser, at, None);
    }
    let statement = Expr::Match(scrutinee, arms, wildcard, span, false);
    let end = BlockEnd::of(&statement);
    (statement, end)
}

/// How a `match`'s arms are read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    /// A statement: a braced arm is a block of statements.
    Statement,
    /// A statement that may turn out to be the value of the block around it:
    /// a braced arm may end in a value.
    MaybeValue,
    /// Written where a value goes: every arm gives a value or leaves.
    Value,
}

/// Parses what follows the `match` keyword, which started at `start`.
///
/// An arm is `pattern => { ... }` or `pattern => expr`, and an expression arm
/// is separated from the next by `,`. A pattern is one or more alternatives
/// separated by `|`. In a `match` used as a value (`is_value`) a braced arm
/// is a block whose last expression is the arm's value.
pub fn parse_match_arms(parser: &mut Parser<'_>, start: u32, is_value: bool) -> Expr {
    let mode = if is_value {
        MatchMode::Value
    } else {
        MatchMode::Statement
    };
    parse_match_in(parser, start, mode).0
}

/// [`parse_match_arms`], with how each arm ends: what an arm written as an
/// expression gives (a value, or a jump), and for a braced arm how its block
/// ends.
fn parse_match_in(
    parser: &mut Parser<'_>,
    start: u32,
    mode: MatchMode,
) -> (Expr, Vec<(BlockEnd, Option<BlockEnd>)>) {
    let match_obj = parse_expr_no_struct(parser);
    parser.next_token_expect(Token::LBrace, "Blocks must be delimited by braces");
    let mut arms: Vec<MatchArm> = Vec::with_capacity(2);
    let mut ends = Vec::with_capacity(2);
    let mut wildcard: Option<Box<[Expr]>> = None;
    let end: u32;
    loop {
        if parser.peek_token() == Token::RBrace {
            if arms.is_empty() {
                cold_path();
                let span = (start, parser.peek_token_span().end).into();
                parser.error(span, ParserErr::MatchBlockZeroArms);
            }
            end = parser.peek_token_span().end;
            parser.next_token();
            break;
        }
        let pattern = parse_pattern(parser, true);
        let guard = if parser.peek_token() == Token::If {
            parser.next_token();
            Some(parse_expr(parser))
        } else {
            None
        };
        parser.next_token_expect(Token::FatArrow, "Expected '=>'");
        // `_` with no guard is the wildcard, which comes last.
        if let (Pattern::Wildcard(wildcard_span), None) = (&pattern, &guard) {
            if arms.is_empty() {
                cold_path();
                let span = (start, wildcard_span.end).into();
                parser.error(span, ParserErr::MatchBlockNoNonWildcardArm);
            }
            let (code, arm_end) = parse_match_arm_body(parser, mode, true);
            ends.push(arm_end);
            end = parser.peek_token_span().end;
            parser.next_token_expect(
                Token::RBrace,
                "The wildcard must be the last statement in a match",
            );
            wildcard = Some(Box::from(code));
            break;
        }
        let (code, arm_end) = parse_match_arm_body(parser, mode, false);
        ends.push(arm_end);
        arms.push(MatchArm {
            pattern,
            guard,
            body: Box::from(code),
        });
    }
    (
        Expr::Match(
            Box::new(match_obj),
            Box::from(arms),
            wildcard,
            (start, end).into(),
            mode == MatchMode::Value,
        ),
        ends,
    )
}

/// The body after an arm's `=>`: a braced block, or an expression followed by
/// the `,` that separates it from the next arm. The expression can be
/// `return`, `throw`, `break` or `continue`. The `,` is optional before the
/// closing brace, and `last` says the wildcard arm is being read, which the
/// closing brace always follows.
///
/// Says how the arm ends: for an expression, whether it leaves or is a value;
/// for a braced block, how the block ends too.
fn parse_match_arm_body(
    parser: &mut Parser<'_>,
    mode: MatchMode,
    last: bool,
) -> (Vec<Expr>, (BlockEnd, Option<BlockEnd>)) {
    if parser.peek_token() == Token::LBrace {
        return match mode {
            MatchMode::Value => {
                let code = parse_value_block(parser);
                let end = code.last().map_or(BlockEnd::Statement, |last| {
                    if last.diverges() {
                        BlockEnd::Jump
                    } else {
                        BlockEnd::Value(0)
                    }
                });
                (code, (end, Some(end)))
            }
            MatchMode::MaybeValue => {
                let (code, end, _) = parse_block_in(parser, Tail::Value);
                (code, (end, Some(end)))
            }
            MatchMode::Statement => (parse_block(parser), (BlockEnd::Statement, None)),
        };
    }
    let peek = parser.peek_token();
    let value = parse_jump(parser, peek).unwrap_or_else(|| parse_expr(parser));
    let at = parser.last_token_end as u32;
    match parser.peek_token_opt() {
        Some(Token::Comma) => {
            parser.next_token();
        }
        Some(Token::RBrace) => {}
        _ if last => {}
        _ => {
            cold_path();
            let span = parser
                .peek_token_opt_span()
                .unwrap_or_else(|| parser.eof_span());
            parser.error(span, ParserErr::MatchArmMissingComma);
        }
    }
    let end = if value.diverges() {
        BlockEnd::Jump
    } else {
        BlockEnd::Value(at)
    };
    (vec![value], (end, None))
}

pub fn parse_enum_declare(parser: &mut Parser<'_>) -> Expr {
    let (t, _) = parser.next_token();
    debug_assert_eq!(t, Token::Enum);
    let (next_token, span) = parser.next_token();
    let enum_name = if let Token::Identifier(id) = next_token {
        SmolStr::new(id)
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(
                Token::Identifier(""),
                next_token,
                "Enum names must be identifiers.",
            ),
        );
    };
    let type_params = parse_type_params(parser);
    parser.next_token_expect(Token::LBrace, "Expected '{'");
    let mut variants: Vec<(SmolStr, Box<[TypeExpr]>, Span)> = Vec::with_capacity(4);
    let mut field_names: Vec<(u16, Box<[SmolStr]>)> = Vec::new();
    loop {
        if parser.peek_token() == Token::RBrace {
            parser.next_token();
            break;
        }
        let (active, uses) = parse_part_attributes(parser);
        let (next_token, v_span) = parser.next_token();
        let variant_name = if let Token::Identifier(i) = next_token {
            SmolStr::new(i)
        } else {
            cold_path();
            parser.error(
                v_span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    next_token,
                    "Enum variant names must be identifiers.",
                ),
            );
        };
        // Optional payload types: `Variant(T, U)`.
        let mut payload: Vec<TypeExpr> = Vec::new();
        if parser.peek_token() == Token::LParen {
            parser.next_token();
            loop {
                if parser.peek_token() == Token::RParen {
                    break;
                }
                payload.push(parse_type(parser));
                if parser.peek_token() == Token::Comma {
                    parser.next_token();
                } else if parser.peek_token() != Token::RParen {
                    cold_path();
                    let span = parser.peek_token_span();
                    parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
                }
            }
            parser.next_token_expect(Token::RParen, "Unmatched '('");
        } else if parser.peek_token() == Token::LBrace {
            // Named fields: `Click { x: int, y: int }`. The payload keeps
            // their order, and the names are what a literal and a pattern
            // write them by.
            parser.next_token();
            let mut names: Vec<SmolStr> = Vec::new();
            loop {
                if parser.peek_token() == Token::RBrace {
                    break;
                }
                let (field, field_span) = parser.next_token();
                let Token::Identifier(field) = field else {
                    cold_path();
                    parser.error(
                        field_span,
                        ParserErr::UnexpectedToken(
                            Token::Identifier(""),
                            field,
                            "Variant field names must be identifiers.",
                        ),
                    );
                };
                parser.next_token_expect(
                    Token::Colon,
                    "A colon must separate a field from its type.",
                );
                names.push(SmolStr::new(field));
                payload.push(parse_type(parser));
                if parser.peek_token() == Token::Comma {
                    parser.next_token();
                } else if parser.peek_token() != Token::RBrace {
                    cold_path();
                    let span = parser.peek_token_span();
                    parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
                }
            }
            parser.next_token_expect(Token::RBrace, "Unmatched '{'");
            field_names.push((variants.len() as u16, Box::from(names)));
        }
        variants.push((variant_name, Box::from(payload), v_span));
        let index = variants.len() - 1;
        end_part(parser, active, uses, AttrOn::Variant(index as u16), || {
            variants.pop();
            if field_names
                .last()
                .is_some_and(|(at, _)| *at as usize == index)
            {
                field_names.pop();
            }
        });
        let (sep, sep_span) = parser.next_token();
        if sep == Token::RBrace {
            break;
        } else if sep != Token::Comma {
            cold_path();
            parser.error(
                sep_span,
                ParserErr::UnexpectedToken(
                    Token::Comma,
                    sep,
                    "In enums, variants must be separated by a comma.",
                ),
            );
        } else if parser.peek_token() == Token::RBrace {
            parser.next_token();
            break;
        }
    }
    Expr::EnumDeclare(
        enum_name,
        Box::from(variants),
        span,
        type_params,
        Box::from(field_names),
    )
}
