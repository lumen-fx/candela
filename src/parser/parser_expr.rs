// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use super::ParserErr;
use super::lexer::Token;
use super::term::parse_term;
use crate::cold_path;
use crate::compiler::expr::Expr;
use crate::compiler::expr::IsTarget;
use crate::compiler::expr::Span;
use crate::parser::Parser;
use crate::parser::TypeArgFollow;
use crate::parser::parse_args;
use crate::parser::parse_type;
use crate::parser::parse_type_args;
use crate::parser::type_args_ahead;
use crate::vm::shift_count_in_range;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::hint::unreachable_unchecked;

pub fn parse_expr_with_precedence(
    input: &mut Parser<'_>,
    min_precedence: u8,
    allow_struct: bool,
) -> Expr {
    let lhs_start = input.peek_token_span().start;
    let mut end = input.peek_token_span().end;
    let mut lhs = parse_term(input, allow_struct);
    if let Some(s) = input.peek_token_opt_span() {
        end = s.end;
    }
    lhs = parse_postfix_op(input, lhs, (lhs_start, end).into());
    let mut lhs_end = input.last_token_end as u32;
    while let Some(peek) = input.peek_token_opt() {
        // `is` and `as` bind at the level of the comparisons and take a type
        // (or, after `is`, a variant pattern) on their right rather than an
        // expression, so `v as int + 1` is `(v as int) + 1` and
        // `a + b as int` is `(a + b) as int`.
        if matches!(peek, Token::Is | Token::As) && COMPARISON_PRECEDENCE > min_precedence {
            input.next_token();
            let rhs_start = input.peek_token_span().start;
            let operand_span: Span = (lhs_start, lhs_end).into();
            if input.peek_token_opt() == Some(Token::Null) {
                cold_path();
                let span = input.peek_token_span();
                input.error(
                    span,
                    ParserErr::UnexpectedTokenStr(
                        "a type",
                        Token::Null,
                        "null is a value, not a type: compare with `== null`.",
                    ),
                );
            }
            lhs = if peek == Token::Is {
                let target = parse_is_target(input);
                let target_span = (rhs_start, input.last_token_end as u32).into();
                Expr::Is(Box::new(lhs), Box::new(target), operand_span, target_span)
            } else {
                let target = parse_type(input);
                let target_span = (rhs_start, input.last_token_end as u32).into();
                Expr::Cast(Box::new(lhs), Box::new(target), operand_span, target_span)
            };
            lhs_end = input.last_token_end as u32;
            continue;
        }
        let Some((op, op_precedence)) = check_op(peek, min_precedence) else {
            break;
        };
        input.next_token();
        let rhs_start = input.peek_token_span().start;
        let rhs = parse_expr_with_precedence(input, op_precedence, allow_struct);
        let rhs_end = input.last_token_end as u32;
        lhs = add_op(
            input,
            op,
            lhs,
            rhs,
            (lhs_start, lhs_end).into(),
            (rhs_start, rhs_end).into(),
        );
        lhs_end = rhs_end;
    }
    lhs
}

pub fn add_op(
    parser: &Parser<'_>,
    op: Token,
    lhs: Expr,
    rhs: Expr,
    span_l: Span,
    span_r: Span,
) -> Expr {
    match op {
        // Only two `bool` literals fold. Folding `x && true` away to `x` would
        // also drop the check that `x` is a `bool`, and folding `false && x`
        // away would drop the check on `x` entirely.
        Token::OpOr => match (lhs, rhs) {
            (Expr::Bool(x), Expr::Bool(y)) => Expr::Bool(x || y),
            (lhs, rhs) => Expr::BoolOr(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpAnd => match (lhs, rhs) {
            (Expr::Bool(x), Expr::Bool(y)) => Expr::Bool(x && y),
            (lhs, rhs) => Expr::BoolAnd(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpEq => Expr::Eq(Box::new(lhs), Box::new(rhs), span_l, span_r),
        Token::OpNEq => Expr::NotEq(Box::new(lhs), Box::new(rhs), span_l, span_r),
        Token::OpInf => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Bool(x < y),
            (Expr::Float(x), Expr::Float(y)) => Expr::Bool(x < y),
            (lhs, rhs) => Expr::Inf(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpInfEq => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Bool(x <= y),
            (Expr::Float(x), Expr::Float(y)) => Expr::Bool(x <= y),
            (lhs, rhs) => Expr::InfEq(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpSup => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Bool(x > y),
            (Expr::Float(x), Expr::Float(y)) => Expr::Bool(x > y),
            (lhs, rhs) => Expr::Sup(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpSupEq => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Bool(x >= y),
            (Expr::Float(x), Expr::Float(y)) => Expr::Bool(x >= y),
            (lhs, rhs) => Expr::SupEq(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpAdd => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x.wrapping_add(y)),
            (Expr::Float(x), Expr::Float(y)) => Expr::Float(x + y),
            (Expr::String(x), Expr::String(y)) => Expr::String(format_args!("{x}{y}").to_smolstr()),
            (lhs, rhs) => Expr::Add(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpSub => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x.wrapping_sub(y)),
            (Expr::Float(x), Expr::Float(y)) => Expr::Float(x - y),
            (lhs, rhs) => Expr::Sub(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpMul => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x.wrapping_mul(y)),
            (Expr::Float(x), Expr::Float(y)) => Expr::Float(x * y),
            (lhs, rhs) => Expr::Mul(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpDiv => match (lhs, rhs) {
            // Float division follows IEEE 754 and yields an infinity rather
            // than raising, so a `0.0` divisor is folded like any other.
            (Expr::Float(x), Expr::Float(y)) => Expr::Float(x / y),
            (ref lhs, Expr::Int(0)) if int_zero_divisor_applies(lhs) => {
                cold_path();
                parser.error(span_l.extend(span_r), ParserErr::DivisionByZero);
            }
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x.wrapping_div(y)),
            (lhs, rhs) => Expr::Div(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpMod => match (lhs, rhs) {
            (Expr::Float(x), Expr::Float(y)) => Expr::Float(x % y),
            (ref lhs, Expr::Int(0)) if int_zero_divisor_applies(lhs) => {
                cold_path();
                parser.error(span_l.extend(span_r), ParserErr::ModuloByZero);
            }
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x.wrapping_rem(y)),
            (lhs, rhs) => Expr::Mod(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpPow => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => {
                if y >= 0 {
                    Expr::Int(x.pow(y as u32))
                } else {
                    cold_path();
                    parser.error(span_l.extend(span_r), ParserErr::IntegerNegativeExponent);
                }
            }
            (Expr::Float(x), Expr::Float(y)) => Expr::Float(x.powf(y)),
            (lhs, rhs) => Expr::Pow(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpBitAnd => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x & y),
            (lhs, rhs) => Expr::BitAnd(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::Pipe => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x | y),
            (lhs, rhs) => Expr::BitOr(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpBitXor => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => Expr::Int(x ^ y),
            (lhs, rhs) => Expr::BitXor(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpShl => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => {
                Expr::Int(x << shift_count(parser, y, span_l.extend(span_r)))
            }
            (lhs, rhs) => Expr::Shl(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        Token::OpShr => match (lhs, rhs) {
            (Expr::Int(x), Expr::Int(y)) => {
                Expr::Int(x >> shift_count(parser, y, span_l.extend(span_r)))
            }
            (lhs, rhs) => Expr::Shr(Box::new(lhs), Box::new(rhs), span_l, span_r),
        },
        _ => unsafe { unreachable_unchecked() },
    }
}

/// The count a literal shift moves by, reported where it cannot be one.
///
/// `int` holds 64 bits, so a shift by 64 or more, or by a negative count, names
/// no result. The pair of literals is folded here, so the mistake is a compile
/// error rather than the runtime one a count only known while the program runs
/// earns.
fn shift_count(parser: &Parser<'_>, count: i64, span: Span) -> u32 {
    if !shift_count_in_range(count) {
        cold_path();
        parser.error(span, ParserErr::ShiftCountOutOfRange(count));
    }
    count as u32
}

/// Whether a literal `0` on the right of `/` or `%` makes the expression an
/// integer division or remainder, and so a compile error.
///
/// Operand types never mix, so an `int` zero on the right forces an `int` left
/// operand however that operand is spelled. The exception is a left operand
/// that is itself a literal of some other type: that is a type error, and
/// reporting it as division by zero would name the wrong mistake.
const fn int_zero_divisor_applies(lhs: &Expr) -> bool {
    !matches!(
        lhs,
        Expr::Float(_) | Expr::String(_) | Expr::Bool(_) | Expr::Null
    )
}

/// The precedence a prefix operator reads its operand at, which is past every
/// binary operator: `-a ^ 2` is `(-a) ^ 2` and `~a + 1` is `(~a) + 1`.
pub const PREFIX_PRECEDENCE: u8 = 12;

/// The operator ahead and the precedence it binds at, or `None` where the token
/// is not a binary operator or binds looser than the expression being read.
///
/// The levels follow Rust, loosest first: `||`, `&&`, equality, comparison,
/// `|`, `^^`, `&`, the shifts, `+ -`, `* / %`, `^`. So `a | b == c` is
/// `(a | b) == c`, and the four levels the language had before the bitwise
/// operators are the four it has now. The table in
/// docs/src/reference/operators.md is the same one, and so is `PREC` in
/// editors/tree-sitter/grammar.js.
#[inline(always)]
const fn check_op(op: Token, min_precedence: u8) -> Option<(Token, u8)> {
    let (op_precedence, is_right_assoc) = match op {
        Token::OpOr => (1, false),
        Token::OpAnd => (2, false),
        Token::OpEq | Token::OpNEq => (3, false),
        Token::OpInf | Token::OpInfEq | Token::OpSup | Token::OpSupEq => {
            (COMPARISON_PRECEDENCE, false)
        }
        Token::Pipe => (PIPE_PRECEDENCE, false),
        Token::OpBitXor => (6, false),
        Token::OpBitAnd => (7, false),
        Token::OpShl | Token::OpShr => (8, false),
        Token::OpAdd | Token::OpSub => (9, false),
        Token::OpMul | Token::OpDiv | Token::OpMod => (10, false),
        Token::OpPow => (11, true),
        _ => return None,
    };
    if (op_precedence > min_precedence) || (is_right_assoc && (op_precedence == min_precedence)) {
        Some((op, op_precedence))
    } else {
        None
    }
}

fn parse_postfix_op(parser: &mut Parser<'_>, mut base: Expr, mut base_span: Span) -> Expr {
    loop {
        match parser.peek_token_opt() {
            // Index or slice
            Some(Token::LBracket) => {
                parser.next_token();
                if parser.peek_token_opt() == Some(Token::RangeDot) {
                    // slice starting at 0
                    parser.next_token();
                    let upper_bound = parse_expr(parser);
                    parser.next_token_expect(Token::RBracket, "Unmatched ']'. Invalid slice.");
                    let end = parser.last_token_end as u32;
                    base_span.end = end;
                    base = Expr::ArrayGetSlice(
                        Box::new(base),
                        Box::from(Expr::Int(0)),
                        Some(Box::new(upper_bound)),
                        base_span,
                    );
                } else {
                    let lower_bound = parse_expr(parser);
                    let (next_token, next_token_span) = parser.next_token();
                    if next_token == Token::RBracket {
                        // array index
                        base_span.end = next_token_span.end;
                        base =
                            Expr::ArrayGetIndex(Box::new(base), Box::new(lower_bound), base_span);
                    } else if next_token == Token::RangeDot {
                        // `a[i..]` runs to the end; `a[i..j]` stops before `j`.
                        let upper_bound = if parser.peek_token_opt() == Some(Token::RBracket) {
                            None
                        } else {
                            Some(Box::new(parse_expr(parser)))
                        };
                        parser.next_token_expect(Token::RBracket, "Unmatched ']'. Invalid slice.");
                        let end = parser.last_token_end as u32;
                        base_span.end = end;
                        base = Expr::ArrayGetSlice(
                            Box::new(base),
                            Box::from(lower_bound),
                            upper_bound,
                            base_span,
                        );
                    } else {
                        cold_path();
                        parser.error(
                            next_token_span,
                            ParserErr::UnexpectedToken(
                                Token::RBracket,
                                next_token,
                                "An index ends with ']', a slice reads `a[i..j]` or `a[i..]`.",
                            ),
                        );
                    }
                }
            }
            // `x?` hands a `None` or an `Err` back to the caller and reads the
            // value out of anything else.
            Some(Token::Question) => {
                let (_, question) = parser.next_token();
                base_span.end = question.end;
                base = Expr::Propagate(Box::new(base), base_span);
            }
            // A call attaches to the expression in front of it, so the
            // closure another call or an index hands back is called where it
            // stands instead of having to be bound first.
            Some(Token::LParen) => {
                parser.next_token();
                let (args, arg_markers, end) = parse_args(parser);
                let call = Expr::CallValue(Box::new(base), args, base_span, arg_markers);
                base_span.end = end;
                base = call;
            }
            // Struct field access or ObjfunctionCall
            Some(Token::Dot) => {
                parser.next_token();

                let (id_token, id_span) = parser.next_token();
                let Token::Identifier(id) = id_token else {
                    cold_path();
                    parser.error(
                        id_span,
                        ParserErr::UnexpectedToken(Token::Identifier(""), id_token, ""),
                    );
                };
                let peek_token = parser.peek_token_opt();
                // A method call takes explicit type arguments the same way a
                // free call does: `b.tagged<string>("hi")` binds the method's
                // own type parameters. The `<` is told from a comparison by the
                // same look-ahead walk, restricted here to a list that closes
                // right before the call parentheses.
                let type_args = if peek_token == Some(Token::OpInf)
                    && type_args_ahead(parser, TypeArgFollow::Call)
                {
                    parse_type_args(parser)
                } else {
                    Box::from([])
                };
                let peek_token = if type_args.is_empty() {
                    peek_token
                } else {
                    parser.peek_token_opt()
                };
                if peek_token == Some(Token::LParen) {
                    // ObjFunctionCall
                    parser.next_token();
                    let (args, arg_markers, end) = parse_args(parser);
                    let obj_function_call = Expr::ObjFunctionCall(
                        Box::new(base),
                        args,
                        Box::new([SmolStr::new(id)]),
                        base_span,
                        (id_span.start, end).into(),
                        arg_markers,
                        type_args,
                    );
                    base_span.end = end;
                    base = obj_function_call;
                } else if peek_token == Some(Token::DoubleColon) {
                    // ObjFunctionCall with namespace
                    parser.next_token();
                    let mut namespace: Vec<SmolStr> = Vec::with_capacity(2);
                    namespace.push(SmolStr::new(id));
                    loop {
                        let (next_token, span) = parser.next_token();
                        if let Token::Identifier(i) = next_token {
                            namespace.push(SmolStr::new(i));
                        } else {
                            cold_path();
                            parser.error(
                                span,
                                ParserErr::UnexpectedToken(Token::Identifier(""), id_token, ""),
                            );
                        }
                        let (next_token, span) = parser.next_token();
                        if next_token == Token::LParen {
                            break;
                        } else if next_token != Token::DoubleColon {
                            cold_path();
                            parser.error(
                                span,
                                ParserErr::UnexpectedTokenStr(
                                    "'(' (function call) or '::' (namespace)",
                                    next_token,
                                    "",
                                ),
                            );
                        }
                    }
                    let (args, arg_markers, end) = parse_args(parser);
                    let obj_function_call = Expr::ObjFunctionCall(
                        Box::new(base),
                        args,
                        Box::from(namespace),
                        base_span,
                        (id_span.start, end).into(),
                        arg_markers,
                        Box::from([]),
                    );
                    base_span.end = end;
                    base = obj_function_call;
                } else {
                    let get_struct_field =
                        Expr::GetStructField(Box::new(base), SmolStr::new(id), base_span, id_span);
                    base_span.end = id_span.end;
                    base = get_struct_field;
                }
            }
            _ => break,
        }
    }
    base
}

/// The binding strength of `|`. A match pattern is read above it, so the `|`
/// between two alternatives is never taken for bitwise or.
pub const PIPE_PRECEDENCE: u8 = 5;

/// The precedence of `<`, `<=`, `>`, `>=`, and of `is` and `as`, which bind at
/// the same level.
const COMPARISON_PRECEDENCE: u8 = 4;

/// The right side of `is`: a variant pattern written the way a `match` arm
/// writes one (`Some(x)`, `Shape::Circle(r)`), or a type. A name with a
/// parenthesised list after it can only be a variant, since no type is
/// written that way; anything else is read as a type, and a bare name that
/// turns out to be a variant with no payload (`None`) is resolved as one by
/// the compiler.
fn parse_is_target(parser: &mut Parser<'_>) -> IsTarget {
    if variant_pattern_ahead(parser) {
        IsTarget::Variant(parse_term(parser, false))
    } else {
        IsTarget::Type(parse_type(parser))
    }
}

/// Whether the tokens ahead are `Name(`, or `A::B(` with any number of path
/// segments.
fn variant_pattern_ahead(parser: &Parser<'_>) -> bool {
    let mut tokens = parser.input.clone().map(|(t, _)| t.ok());
    let mut next = || tokens.next().flatten();
    if !matches!(next(), Some(Token::Identifier(_))) {
        return false;
    }
    loop {
        match next() {
            Some(Token::LParen) => return true,
            Some(Token::DoubleColon) => {
                if !matches!(next(), Some(Token::Identifier(_))) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

#[inline(always)]
pub fn parse_expr(parser: &mut Parser<'_>) -> Expr {
    parse_expr_with_precedence(parser, 0, true)
}

#[inline(always)]
pub fn parse_expr_no_struct(parser: &mut Parser<'_>) -> Expr {
    parse_expr_with_precedence(parser, 0, false)
}
