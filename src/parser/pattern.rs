//! Reads a pattern: the one grammar `match` arms, `is`, `let`, `for` and
//! parameters share. See [`crate::compiler::patterns::Pattern`].

use super::Parser;
use super::ParserErr;
use super::TypeArgFollow;
use super::lexer::Token;
use super::lexer::parse_string;
use super::parse_type_args;
use super::type_args_ahead;
use crate::cold_path;
use crate::compiler::expr::Expr;
use crate::compiler::expr::Span;
use crate::compiler::patterns::Pattern;
use crate::compiler::patterns::Payload;
use crate::compiler::patterns::is_binding_name;
use smol_strc::SmolStr;

/// Reads a pattern with its `|` alternatives. Where a `{` opens a body
/// (`allow_struct` false), a struct or variant pattern with braces goes in
/// parentheses.
pub fn parse_pattern(parser: &mut Parser<'_>, allow_struct: bool) -> Pattern {
    let start = parser.peek_token_span().start;
    let first = parse_single_pattern(parser, allow_struct);
    if parser.peek_token_opt() != Some(Token::Pipe) {
        return first;
    }
    let mut alternatives = vec![first];
    while parser.peek_token_opt() == Some(Token::Pipe) {
        parser.next_token();
        if parser.peek_token() == Token::Identifier("_") {
            cold_path();
            let span = parser.peek_token_span();
            parser.error(span, ParserErr::MatchWildcardAlternative);
        }
        alternatives.push(parse_single_pattern(parser, allow_struct));
    }
    Pattern::Or(
        Box::from(alternatives),
        (start, parser.last_token_end as u32).into(),
    )
}

/// A pattern without `|` alternatives at its top.
pub fn parse_single_pattern(parser: &mut Parser<'_>, allow_struct: bool) -> Pattern {
    let span = parser.peek_token_span();
    parser.enter(span);
    let pattern = parse_pattern_inner(parser, allow_struct);
    parser.leave();
    pattern
}

fn parse_pattern_inner(parser: &mut Parser<'_>, allow_struct: bool) -> Pattern {
    let (t, t_span) = parser.next_token();
    match t {
        Token::Identifier("_") => Pattern::Wildcard(t_span),
        // `()`: the value of no value, as in `Ok(())`.
        Token::LParen if parser.peek_token() == Token::RParen => {
            let close = parser.next_token().1;
            Pattern::Value(Box::new(Expr::Null), t_span.extend(close))
        }
        Token::LParen => parse_tuple_pattern(parser, t_span),
        Token::Identifier(name) => parse_named_pattern(parser, name, t_span, allow_struct),
        _ => {
            let Some(value) = literal(parser, t, t_span) else {
                cold_path();
                parser.error(t_span, ParserErr::PatternExpected(t));
            };
            range_after(parser, value, t_span)
        }
    }
}

/// The literal `t` starts, read at `t_span`: a number (with a `-` in front),
/// a string, `true`, `false` or `null`.
fn literal(parser: &mut Parser<'_>, t: Token<'_>, t_span: Span) -> Option<Expr> {
    Some(match t {
        Token::Int(i64::MIN) => parser.error(t_span, ParserErr::IntLiteralOutOfRange),
        Token::Int(i) => Expr::Int(i),
        Token::Float(f) => Expr::Float(f),
        Token::DotDigits(s) => Expr::Float(s.parse().unwrap_or_default()),
        Token::String(s) => Expr::String(parse_string(s)),
        Token::True => Expr::Bool(true),
        Token::False => Expr::Bool(false),
        Token::Null => Expr::Null,
        Token::OpSub => match parser.next_token() {
            (Token::Int(i), _) => Expr::Int(i.wrapping_neg()),
            (Token::Float(f), _) => Expr::Float(-f),
            (Token::DotDigits(s), _) => Expr::Float(-s.parse::<f64>().unwrap_or_default()),
            (other, span) => {
                cold_path();
                parser.error(span, ParserErr::PatternExpected(other));
            }
        },
        _ => return None,
    })
}

/// A value pattern, or the range `value..high` or `value..=high` when a range
/// operator follows it.
fn range_after(parser: &mut Parser<'_>, low: Expr, start: Span) -> Pattern {
    let inclusive = match parser.peek_token_opt() {
        Some(Token::RangeDot) => false,
        Some(Token::RangeDotEq) => true,
        _ => {
            let span = (start.start, parser.last_token_end as u32).into();
            return Pattern::Value(Box::new(low), span);
        }
    };
    parser.next_token();
    let (t, t_span) = parser.next_token();
    let high = match t {
        Token::Identifier(name) => {
            let mut path = vec![SmolStr::new(name)];
            while parser.peek_token_opt() == Some(Token::DoubleColon) {
                parser.next_token();
                let (segment, span) = parser.next_token();
                let Token::Identifier(segment) = segment else {
                    cold_path();
                    parser.error(span, ParserErr::PatternExpected(segment));
                };
                path.push(SmolStr::new(segment));
            }
            path_value(
                path,
                t_span.extend((t_span.start, parser.last_token_end as u32).into()),
            )
        }
        _ => literal(parser, t, t_span).unwrap_or_else(|| {
            cold_path();
            parser.error(t_span, ParserErr::PatternExpected(t));
        }),
    };
    Pattern::Range(
        Box::new(low),
        Box::new(high),
        inclusive,
        (start.start, parser.last_token_end as u32).into(),
    )
}

/// The value a path names where a range bound goes: a constant.
fn path_value(path: Vec<SmolStr>, span: Span) -> Expr {
    if path.len() == 1 {
        Expr::Var(path.into_iter().next().unwrap(), span)
    } else {
        Expr::NamespacedRef(Box::from(path), span, Box::from([]))
    }
}

/// `(a, b)`, or `(p)`, which is `p` grouped. The `(` is read.
fn parse_tuple_pattern(parser: &mut Parser<'_>, open: Span) -> Pattern {
    let mut items = Vec::with_capacity(2);
    let mut trailing_comma = None;
    loop {
        items.push(parse_pattern(parser, true));
        if parser.peek_token() != Token::Comma {
            break;
        }
        let comma = parser.next_token().1;
        if parser.peek_token() == Token::RParen {
            trailing_comma = Some(comma);
            break;
        }
    }
    let close = parser.next_token_expect(Token::RParen, "Unmatched '(' in a pattern");
    if items.len() == 1 {
        if let Some(comma) = trailing_comma {
            cold_path();
            parser.error(comma, ParserErr::TupleOfOne);
        }
        return items.pop().unwrap();
    }
    Pattern::Tuple(Box::from(items), open.extend(close))
}

/// A pattern that starts with the name `first`: a name that binds, or a path
/// to a variant, a struct or a constant, with what it holds after it.
fn parse_named_pattern(
    parser: &mut Parser<'_>,
    first: &str,
    first_span: Span,
    allow_struct: bool,
) -> Pattern {
    let mut path = vec![SmolStr::new(first)];
    let mut type_args: Box<[crate::compiler::type_system::TypeExpr]> = Box::from([]);
    loop {
        match parser.peek_token_opt() {
            Some(Token::DoubleColon) => {
                parser.next_token();
                let (segment, span) = parser.next_token();
                let Token::Identifier(segment) = segment else {
                    cold_path();
                    parser.error(span, ParserErr::PatternExpected(segment));
                };
                path.push(SmolStr::new(segment));
            }
            Some(Token::OpInf)
                if type_args.is_empty()
                    && type_args_ahead(
                        parser,
                        if allow_struct {
                            TypeArgFollow::Term
                        } else {
                            TypeArgFollow::TermNoStruct
                        },
                    ) =>
            {
                type_args = parse_type_args(parser);
            }
            _ => break,
        }
    }
    let span_here =
        |parser: &Parser<'_>| -> Span { (first_span.start, parser.last_token_end as u32).into() };
    let payload = match parser.peek_token_opt() {
        Some(Token::LParen) => {
            parser.next_token();
            let mut items = Vec::with_capacity(2);
            while parser.peek_token() != Token::RParen {
                items.push(parse_pattern(parser, true));
                if parser.peek_token() != Token::Comma {
                    break;
                }
                parser.next_token();
            }
            parser.next_token_expect(Token::RParen, "Unmatched '(' in a pattern");
            Payload::Positional(Box::from(items))
        }
        Some(Token::LBrace) if allow_struct => {
            parser.next_token();
            parse_field_patterns(parser)
        }
        _ => {
            if path.len() == 1 && type_args.is_empty() && is_binding_name(first) {
                return Pattern::Bind(path.pop().unwrap(), first_span);
            }
            if matches!(
                parser.peek_token_opt(),
                Some(Token::RangeDot | Token::RangeDotEq)
            ) {
                let span = span_here(parser);
                return range_after(parser, path_value(path, span), span);
            }
            Payload::None
        }
    };
    Pattern::Named {
        path: Box::from(path),
        type_args,
        payload,
        span: span_here(parser),
    }
}

/// The fields of a braced pattern, `{ x, y: py, .. }`, the `{` read.
fn parse_field_patterns(parser: &mut Parser<'_>) -> Payload {
    let mut fields = Vec::with_capacity(2);
    let mut rest = false;
    loop {
        let (t, span) = parser.next_token();
        match t {
            Token::RBrace => break,
            Token::RangeDot => {
                rest = true;
                parser.next_token_expect(
                    Token::RBrace,
                    "'..' ends a pattern's field list, so '}' follows it",
                );
                break;
            }
            Token::Identifier(name) => {
                let name = SmolStr::new(name);
                let pattern = if parser.peek_token() == Token::Colon {
                    parser.next_token();
                    parse_pattern(parser, true)
                } else {
                    Pattern::Bind(name.clone(), span)
                };
                fields.push((name, pattern, span));
                match parser.next_token() {
                    (Token::Comma, _) => {}
                    (Token::RBrace, _) => break,
                    (other, span) => {
                        cold_path();
                        parser.error(
                            span,
                            ParserErr::UnexpectedTokenStr(
                                "',' (another field) or '}' (end of the pattern)",
                                other,
                                "",
                            ),
                        );
                    }
                }
            }
            other => {
                cold_path();
                parser.error(
                    span,
                    ParserErr::UnexpectedTokenStr("a field name, '..' or '}'", other, ""),
                );
            }
        }
    }
    Payload::Fields(Box::from(fields), rest)
}
