// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use super::ParserErr;
use super::lexer::Token;
use super::lexer::parse_string;
use super::parse_expr;
use super::parser_expr;
use super::parser_expr::PREFIX_PRECEDENCE;
use super::parser_expr::parse_expr_with_precedence;
use crate::cold_path;
use crate::compiler::expr::Expr;
use crate::compiler::expr::PatternLet;
use crate::compiler::expr::Span;
use crate::compiler::expr::StructRest;
use crate::compiler::expr::tuple_literal;
use crate::compiler::patterns::Pattern;
use crate::compiler::type_system::TypeExpr;
use crate::parser::Parser;
use crate::parser::TypeArgFollow;
use crate::parser::block_ahead;
use crate::parser::blocks::parse_condition_expr;
use crate::parser::blocks::parse_fn_body;
use crate::parser::blocks::parse_match_arms;
use crate::parser::blocks::parse_value_block;
use crate::parser::blocks::parse_value_block_after;
use crate::parser::expand_macro;
use crate::parser::expand_macro_path;
use crate::parser::extend_namespace;
use crate::parser::parse_args;
use crate::parser::parse_namespace;
use crate::parser::parse_type_args;
use crate::parser::pattern::parse_pattern;
use crate::parser::type_args_ahead;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;

// Must be called right after LParen is skipped
// Identifier LParen Expr RParen
// Parses: Expr RParen
fn parse_fn_call(
    parser: &mut Parser<'_>,
    namespace: Box<[SmolStr]>,
    span: Span,
    type_args: Box<[TypeExpr]>,
) -> Expr {
    let (args, arg_markers, _) = parse_args(parser);
    Expr::FunctionCall(args, namespace, span, arg_markers, type_args)
}

// Must be called right after LParen is skipped
fn parse_struct(
    parser: &mut Parser<'_>,
    namespace: Box<[SmolStr]>,
    start: u32,
    type_args: Box<[TypeExpr]>,
) -> Expr {
    let mut fields: Vec<(SmolStr, Expr, Span, Span)> = Vec::with_capacity(4);
    let mut base: Option<Box<(StructRest, Span)>> = None;
    let end: u32;
    // `Tag {}`: a struct with no fields.
    if parser.peek_token() == Token::RBrace {
        let close = parser.next_token().1.end;
        return Expr::Struct(
            namespace,
            Box::from(fields),
            (start, close).into(),
            type_args,
            base,
        );
    }
    loop {
        // `..base` closes the literal: every field it does not write is taken
        // from `base`, the way Rust's struct update syntax reads. A bare `..`
        // takes them from the struct's default.
        if parser.peek_token() == Token::RangeDot {
            let dots = parser.next_token().1;
            base = Some(Box::new(if parser.peek_token() == Token::RBrace {
                (StructRest::Default, dots)
            } else {
                let base_start = parser.peek_token_span().start;
                let base_expr = parse_expr(parser);
                (
                    StructRest::Base(base_expr),
                    (base_start, parser.last_token_end as u32).into(),
                )
            }));
            let (next_token, span) = parser.next_token();
            if next_token != Token::RBrace {
                cold_path();
                parser.error(
                    span,
                    ParserErr::UnexpectedToken(
                        Token::RBrace,
                        next_token,
                        "The `..` of a struct literal comes last.",
                    ),
                );
            }
            end = span.end;
            break;
        }
        let (next_token, field_name_span) = parser.next_token();
        let field_name = if let Token::Identifier(i) = next_token {
            SmolStr::new(i)
        } else {
            cold_path();
            parser.error(
                field_name_span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    next_token,
                    "Struct field names must be identifiers.",
                ),
            )
        };
        // `Vec2 { x, y }`: a field written alone takes the variable of its
        // name, the way `x: x` would.
        if matches!(parser.peek_token(), Token::Comma | Token::RBrace) {
            fields.push((
                field_name.clone(),
                Expr::Var(field_name, field_name_span),
                field_name_span,
                field_name_span,
            ));
        } else {
            parser.next_token_expect(
                Token::Colon,
                "A colon separates a field from its value; a field written alone takes the variable of its name.",
            );
            let field_start: u32 = parser.peek_token_span().start;
            let field_value = parse_expr(parser);
            fields.push((
                field_name,
                field_value,
                field_name_span,
                (field_start, parser.peek_token_span().start).into(),
            ));
        }
        let (next_token, span) = parser.next_token();
        if next_token == Token::RBrace {
            end = span.end;
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
            end = parser.next_token().1.end;
            break;
        }
    }

    Expr::Struct(
        namespace,
        Box::from(fields),
        (start, end).into(),
        type_args,
        base,
    )
}

/// Parses what follows a committed type-argument list: a call, a struct
/// literal, or a variant of a generic enum. `path` is what the type arguments
/// were written on, so the enum of a module bound with `as`
/// (`g::Slot<int>::Filled(4)`) reaches the same three shapes as a local one.
#[inline(never)]
fn parse_generic_term(
    parser: &mut Parser<'_>,
    path: Box<[SmolStr]>,
    type_args: Box<[TypeExpr]>,
    start: u32,
    allow_struct: bool,
) -> Expr {
    let span: Span = (start, parser.last_token_end as u32).into();
    match parser.peek_token() {
        Token::LParen => {
            parser.next_token();
            parse_fn_call(parser, path, span, type_args)
        }
        Token::LBrace => {
            parser.next_token();
            parse_struct(parser, path, start, type_args)
        }
        // `Slot<int>::Filled(x)` / `Slot<int>::Empty`: a variant of one
        // instantiation of a generic enum.
        _ => {
            parser.next_token_expect(Token::DoubleColon, "");
            let mut namespace = path.into_vec();
            let end = extend_namespace(parser, &mut namespace);
            let namespace: Box<[SmolStr]> = Box::from(namespace);
            let span: Span = (start, end).into();
            if parser.peek_token_opt() == Some(Token::LParen) {
                parser.next_token();
                parse_fn_call(parser, namespace, span, type_args)
            } else if parser.peek_token_opt() == Some(Token::LBrace) && allow_struct {
                // `Slot<int>::Filled { value: 1 }`: a variant with named fields.
                parser.next_token();
                parse_struct(parser, namespace, start, type_args)
            } else {
                Expr::NamespacedRef(namespace, span, type_args)
            }
        }
    }
}

pub fn parse_term(parser: &mut Parser<'_>, allow_struct: bool) -> Expr {
    let span = parser.peek_token_span();
    parser.enter(span);
    let term = parse_term_inner(parser, allow_struct);
    parser.leave();
    term
}

fn parse_term_inner(parser: &mut Parser<'_>, allow_struct: bool) -> Expr {
    let (t, t_span) = parser.next_token();
    match t {
        // The lexer reads one value past the top of the range so the smallest
        // `int` can be written with a minus in front; on its own that literal
        // is out of range like its neighbours.
        Token::Int(i64::MIN) => parser.error(t_span, ParserErr::IntLiteralOutOfRange),
        Token::Int(i) => Expr::Int(i),
        Token::Float(f) => Expr::Float(f),
        // `.5`: the float with nothing before its point. The digits cannot
        // overflow, so the text always reads as a float.
        Token::DotDigits(s) => Expr::Float(s.parse().unwrap_or_default()),
        Token::String(s) => Expr::String(parse_string(s)),
        // MACRO: name!( raw region ), expanded by the embedder and parsed here
        Token::Macro(region) => expand_macro(parser, region, t_span),
        Token::True => Expr::Bool(true),
        Token::False => Expr::Bool(false),
        Token::Null => Expr::Null,
        Token::Identifier(s) => {
            let start = t_span.start;
            match parser.peek_token_opt() {
                // FUNCTION CALL:
                // Identifier LParen Expr RParen
                Some(Token::LParen) => {
                    parser.next_token();
                    parse_fn_call(parser, Box::new([s.to_smolstr()]), t_span, Box::from([]))
                }
                // STRUCT
                Some(Token::LBrace) if allow_struct => {
                    parser.next_token();
                    parse_struct(parser, Box::from([SmolStr::new(s)]), start, Box::from([]))
                }
                // GENERIC CALL, STRUCT LITERAL OR ENUM VARIANT:
                // Identifier '<' Type,... '>' ('(' | '{' | '::')
                //
                // `<` is also the comparison operator, so it opens a type
                // argument list only when the whole list parses and closes into
                // one of those three; see `type_args_ahead`.
                Some(Token::OpInf)
                    if type_args_ahead(
                        parser,
                        if allow_struct {
                            TypeArgFollow::Term
                        } else {
                            TypeArgFollow::TermNoStruct
                        },
                    ) =>
                {
                    let type_args = parse_type_args(parser);
                    parse_generic_term(
                        parser,
                        Box::from([SmolStr::new(s)]),
                        type_args,
                        start,
                        allow_struct,
                    )
                }
                // A MACRO BEHIND A MODULE'S NAME: m::name!( ... )
                Some(Token::DoubleColon)
                    if matches!(parser.input.clone().nth(1), Some((Ok(Token::Macro(_)), _))) =>
                {
                    parser.next_token();
                    let (Token::Macro(region), m_span) = parser.next_token() else {
                        unreachable!("the token after '::' was read as a macro");
                    };
                    expand_macro_path(parser, s, region, (t_span.start, m_span.end).into())
                }
                // NAMESPACE
                Some(Token::DoubleColon) => {
                    parser.next_token();
                    let (namespace, end) = parse_namespace(parser, SmolStr::new(s));
                    match parser.peek_token_opt() {
                        // FUNCTION CALL WITH NAMESPACE (also an enum variant with
                        // payload, `Enum::Variant(args)`, disambiguated by the
                        // compiler):
                        // (Identifier DoubleColon)+ Identifier LParen Expr RParen
                        Some(Token::LParen) => {
                            parser.next_token();
                            parse_fn_call(
                                parser,
                                namespace,
                                (t_span.start, end).into(),
                                Box::from([]),
                            )
                        }
                        // STRUCT WITH NAMESPACE. Gated on `allow_struct` so a
                        // brace that opens a following block (for example a
                        // `match Enum::Variant { ... }` body) is not mistaken for
                        // a struct literal.
                        Some(Token::LBrace) if allow_struct => {
                            parser.next_token();
                            parse_struct(parser, namespace, start, Box::from([]))
                        }
                        // A GENERIC TYPE INSIDE THE NAMESPACE, whose type
                        // arguments sit on a segment in the middle of the path:
                        // `g::Slot<int>::Filled(4)` for a module bound with
                        // `as`. What follows the list is a call, a struct
                        // literal or a variant, the same three shapes a local
                        // generic name takes.
                        Some(Token::OpInf)
                            if type_args_ahead(
                                parser,
                                if allow_struct {
                                    TypeArgFollow::Term
                                } else {
                                    TypeArgFollow::TermNoStruct
                                },
                            ) =>
                        {
                            let type_args = parse_type_args(parser);
                            parse_generic_term(parser, namespace, type_args, start, allow_struct)
                        }
                        // A bare namespaced identifier: a nullary enum-variant
                        // construction (`Color::Red`), resolved by the compiler.
                        _ => Expr::NamespacedRef(
                            namespace,
                            (t_span.start, end).into(),
                            Box::from([]),
                        ),
                    }
                }
                _ => Expr::Var(SmolStr::new(s), (t_span.start, t_span.end).into()),
            }
        }
        Token::LBracket => {
            let mut elem_spans: Vec<Span> = Vec::with_capacity(2);
            elem_spans.push((t_span.start, 0u32).into());
            let mut elems = Vec::with_capacity(4);
            loop {
                if parser.peek_token_opt() == Some(Token::RBracket) {
                    elem_spans[0].end = parser.next_token().1.end;
                    break;
                }
                let start = parser.peek_token_span().start;
                elems.push(parse_expr(parser));
                let end = parser.peek_token_span().end - 1;
                elem_spans.push((start, end).into());
                if parser.peek_token_opt() == Some(Token::Comma) {
                    parser.next_token();
                } else if !(parser.peek_token_opt() == Some(Token::RBracket)) {
                    cold_path();
                    let span = unsafe { parser.peek_token_opt_span().unwrap_unchecked() };
                    parser.error(span, ParserErr::ArrayElementsMissingComma);
                }
            }
            Expr::Array(Box::from(elems), Box::from(elem_spans))
        }
        // LParen Expr RParen, or a tuple: LParen Expr (Comma Expr)+ RParen
        Token::LParen => {
            let first_start = parser.peek_token_span().start;
            let v = parse_expr(parser);
            if parser.peek_token() != Token::Comma {
                parser.next_token_expect(Token::RParen, "Unmatched ')'");
                return v;
            }
            let mut elements = vec![(v, Span::from((first_start, parser.last_token_end as u32)))];
            while parser.peek_token() == Token::Comma {
                let comma = parser.next_token().1;
                // A comma before the `)` closes the list, the way it may
                // after a list's last element.
                if parser.peek_token() == Token::RParen {
                    if elements.len() == 1 {
                        cold_path();
                        parser.error(comma, ParserErr::TupleOfOne);
                    }
                    break;
                }
                let start = parser.peek_token_span().start;
                let element = parse_expr(parser);
                elements.push((element, (start, parser.last_token_end as u32).into()));
            }
            let end = parser.next_token_expect(Token::RParen, "Unmatched ')'").end;
            tuple_literal(elements, (t_span.start, end).into())
        }
        // - Expr
        Token::OpSub => {
            if parser.peek_token() == Token::Int(i64::MIN) {
                parser.next_token();
                return Expr::Int(i64::MIN);
            }
            let expr_start = parser.peek_token_span().start;
            match parse_expr_with_precedence(parser, PREFIX_PRECEDENCE, allow_struct) {
                Expr::Int(i) => Expr::Int(i.wrapping_neg()),
                Expr::Float(f) => Expr::Float(-f),
                other => Expr::Neg(
                    Box::new(other),
                    (t_span.start, parser.peek_token_span().start).into(),
                    (expr_start, parser.peek_token_span().start).into(),
                ),
            }
        }
        // ! Expr
        Token::OpNot => {
            let expr_start = parser.peek_token_span().start;
            match parse_expr_with_precedence(parser, PREFIX_PRECEDENCE, allow_struct) {
                Expr::Bool(b) => Expr::Bool(!b),
                other => Expr::BoolNeg(
                    Box::new(other),
                    (t_span.start, parser.peek_token_span().start).into(),
                    (expr_start, parser.peek_token_span().start).into(),
                ),
            }
        }
        // ~ Expr
        Token::OpBitNot => {
            let expr_start = parser.peek_token_span().start;
            match parse_expr_with_precedence(parser, PREFIX_PRECEDENCE, allow_struct) {
                Expr::Int(i) => Expr::Int(!i),
                other => Expr::BitNot(
                    Box::new(other),
                    (t_span.start, parser.peek_token_span().start).into(),
                    (expr_start, parser.peek_token_span().start).into(),
                ),
            }
        }
        // A match used as a value.
        Token::Match => parse_match_arms(parser, t_span.start, true),
        // Inline condition
        Token::If => {
            let (condition, condition_span) = parse_condition_expr(parser);
            let mut output_code: Vec<Expr> = parse_value_block(parser);
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
                    let (else_if_condition, else_if_span) = parse_condition_expr(parser);
                    let else_if_code = parse_value_block(parser);
                    output_code.push(Expr::ElseIfBlock(
                        Box::new(else_if_condition),
                        Box::from(else_if_code),
                        else_if_span,
                    ));
                } else if next_token == Some(Token::LBrace) {
                    let else_code = parse_value_block(parser);
                    output_code.push(Expr::ElseBlock(Box::from(else_code)));
                    break;
                } else {
                    break;
                }
            }
            if !matches!(output_code.last().unwrap(), Expr::ElseBlock(_)) {
                cold_path();
                parser.error(
                    (t_span.start, parser.last_token_end as u32).into(),
                    ParserErr::InlineConditionNoElseBlock,
                );
            }
            Expr::InlineCondition(
                Box::new(condition),
                Box::from(output_code),
                (t_span.start, parser.last_token_end as u32).into(),
                condition_span,
            )
        }
        // anonymous function
        Token::Function => {
            let start = t_span.start;
            parser.next_token_expect(
                Token::LParen,
                "Function arguments must be delimited by parentheses",
            );
            let mut args: Vec<SmolStr> = Vec::with_capacity(2);
            let mut lets: Vec<Expr> = Vec::new();
            // `fn()` takes nothing, the empty list a declaration accepts too.
            // The names are read only once there is one to read. A parameter
            // written as a pattern takes the argument under a name no program
            // can write, and the body opens with the `let` that takes it
            // apart.
            while parser.peek_token() != Token::RParen {
                // A name alone binds, whatever its case.
                let plain_name = matches!(parser.peek_token(), Token::Identifier(name) if name != "_")
                    && parser
                        .input
                        .clone()
                        .nth(1)
                        .is_some_and(|(t, _)| matches!(t, Ok(Token::Comma | Token::RParen)));
                let pattern = if plain_name {
                    let (Token::Identifier(name), span) = parser.next_token() else {
                        unreachable!()
                    };
                    Pattern::Bind(SmolStr::new(name), span)
                } else {
                    parse_pattern(parser, true)
                };
                match pattern {
                    Pattern::Bind(name, _) => args.push(name),
                    pattern => {
                        let span = pattern.span();
                        let name = SmolStr::from(format!("[param {}]", args.len()));
                        lets.push(Expr::LetPattern(Box::new(PatternLet {
                            pattern,
                            value: Expr::Var(name.clone(), span),
                            annotation: None,
                            otherwise: None,
                            span,
                        })));
                        args.push(name);
                    }
                }
                if parser.peek_token() != Token::Comma {
                    break;
                }
                parser.next_token();
            }
            parser.next_token_expect(
                Token::RParen,
                "Function arguments must be delimited by parentheses",
            );
            let next_token = parser.peek_token_opt();
            // if next_token == Token::Arrow {
            //     // return type
            //     let return_type = parse_type(parser);
            //     parser.next_token_expect(Token::LBrace, "Blocks must begin with a '{'.");
            //     let fn_code = parse_code(parser);
            //     let end = parser.next_token_expect_end(Token::RBrace, "Unmatched '}'");
            //     Expr::AnonymousFunction(
            //         Box::from(args),
            //         Box::from(fn_code),
            //         (start, end).into(),
            //     )
            // } else
            if matches!(next_token, Some(Token::LBrace | Token::FatArrow)) {
                parser.param_lets = lets;
                let fn_code = parse_fn_body(parser, false);
                Expr::AnonymousFunction(
                    Box::from(args),
                    Box::from(fn_code),
                    (start, parser.last_token_end as u32).into(),
                )
            } else {
                let span = parser.peek_token_span();
                parser.error(
                    span,
                    ParserErr::UnexpectedTokenStr(
                        "'{' (function body) OR '=>' (expression body)",
                        next_token.unwrap(),
                        "",
                    ),
                );
            }
        }
        // A block used as a value, `{ let t = 5; t * 2 }`.
        Token::LBrace if block_ahead(parser) => {
            Expr::EvalBlock(Box::from(parse_value_block_after(parser, t_span)))
        }
        // map
        Token::LBrace => {
            let mut kv_pairs: Vec<(Expr, Span, Expr, Span)> = Vec::with_capacity(2);
            let end: u32;
            // Empty map literal `{}`: no key-value pairs.
            if parser.peek_token() == Token::RBrace {
                end = parser.next_token().1.end;
                return Expr::Map(Box::from(kv_pairs), (t_span.start, end).into());
            }
            loop {
                let key_start = parser.peek_token_span().start;
                let key = parse_term(parser, allow_struct);
                let key_end = parser.last_token_end as u32;
                parser.next_token_expect(
                    Token::Colon,
                    "Key-value pairs must be separated by a colon",
                );
                let value_start = parser.peek_token_span().start;
                let value = parser_expr::parse_expr(parser);
                let value_end = parser.last_token_end as u32;
                kv_pairs.push((
                    key,
                    (key_start, key_end).into(),
                    value,
                    (value_start, value_end).into(),
                ));
                let peek_token = parser.peek_token();
                if peek_token == Token::Comma {
                    parser.next_token();
                } else if peek_token == Token::RBrace {
                    end = parser.next_token().1.end;
                    break;
                } else {
                    let span = parser.peek_token_span();
                    parser.error(
                        span,
                        ParserErr::UnexpectedTokenStr(
                            "',' (another key-value pair) or '}' (end of map)",
                            peek_token,
                            "",
                        ),
                    )
                }
            }
            Expr::Map(Box::from(kv_pairs), (t_span.start, end).into())
        }
        unexpected => {
            cold_path();
            parser.error(
                t_span,
                ParserErr::UnexpectedTokenStr("Term", unexpected, ""),
            );
        }
    }
}
