use super::ParserErr;
use super::lexer::Token;
use super::lexer::parse_string;
use super::parser_expr::parse_expr;
use super::parser_expr::parse_expr_no_struct;
use crate::cold_path;
use crate::compiler::expr::Expr;
use crate::compiler::expr::Span;
use crate::compiler::expr::mangle_method;
use crate::compiler::type_system::ImplTemplate;
use crate::compiler::type_system::ReturnAnnotation;
use crate::parser::Parser;
use crate::parser::TypeExpr;
use crate::parser::parse_code;
use crate::parser::parse_type;
use crate::parser::parse_type_args;
use crate::parser::parse_type_params;
use smol_strc::SmolStr;
use std::rc::Rc;

// call right after peeking Token::If
pub fn parse_condition_block(parser: &mut Parser<'_>, start: u32) -> Expr {
    let t = parser.next_token();
    debug_assert_eq!(t.0, Token::If);
    let condition = parse_expr_no_struct(parser);
    let mut output_code = parse_block(parser);
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
            let else_if_condition = parse_expr_no_struct(parser);
            let else_if_code = parse_block(parser);
            output_code.push(Expr::ElseIfBlock(
                Box::new(else_if_condition),
                Box::from(else_if_code),
            ));
        } else if next_token == Some(Token::LBrace) {
            let else_code = parse_block(parser);
            output_code.push(Expr::ElseBlock(Box::from(else_code)));
            break;
        } else {
            break;
        }
    }
    Expr::Condition(
        Box::new(condition),
        Box::from(output_code),
        (start, parser.last_token_end as u32).into(),
    )
}

/// LBrace Code RBrace
#[inline(always)]
pub fn parse_block(parser: &mut Parser<'_>) -> Vec<Expr> {
    let opener_token_span =
        parser.next_token_expect(Token::LBrace, "Blocks need to start with '{'");
    let code = parse_code(parser);
    parser.next_token_expect_closer(Token::LBrace, opener_token_span, Token::RBrace);
    code
}

/// LBrace Expr RBrace
#[inline(always)]
pub fn parse_block_expr(parser: &mut Parser<'_>) -> Expr {
    let opener_token_span =
        parser.next_token_expect(Token::LBrace, "Blocks need to start with '{'");
    let code = parse_expr(parser);
    parser.next_token_expect_closer(Token::LBrace, opener_token_span, Token::RBrace);
    code
}

pub fn parse_while_block(input: &mut Parser<'_>) -> Expr {
    let t = input.next_token();
    debug_assert_eq!(t.0, Token::While);
    let while_condition = parse_expr_no_struct(input);
    let while_code = parse_block(input);
    Expr::WhileBlock(Box::new(while_condition), Box::from(while_code))
}

/// Parses ForLoop and IntForLoop
/// for Identifier in Expr LBrace Code RBrace
/// for Identifier in Expr RangeDot Expr LBrace Code Rbrace
/// for Identifier in RangeDot Expr LBrace Code RBrace
pub fn parse_for_loop(parser: &mut Parser<'_>) -> Expr {
    let t = parser.next_token();
    debug_assert_eq!(t.0, Token::For);
    let (i_token, span) = parser.next_token();
    let id = if let Token::Identifier(id) = i_token {
        SmolStr::new(id)
    } else {
        cold_path();
        parser.error(
            span,
            ParserErr::UnexpectedToken(Token::Identifier(""), i_token, ""),
        );
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
        let for_loop_code = parse_block(parser);
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
        let peek_token = parser.peek_token();
        if peek_token == Token::RangeDot {
            parser.next_token();
            let start2 = parser.peek_token_span().start;
            let upper_bound = parse_expr_no_struct(parser);
            let end2 = parser.last_token_end as u32;
            let for_loop_code = parse_block(parser);
            Expr::IntForLoop(
                id,
                Box::new(for_collection),
                Box::new(upper_bound),
                Box::from(for_loop_code),
                (start, start).into(),
                (start2, end2).into(),
            )
        } else {
            let for_loop_code = parse_block(parser);
            Expr::ForLoop(
                id,
                Box::new(for_collection),
                Box::from(for_loop_code),
                (start, end).into(),
            )
        }
    }
}

#[inline(always)]
pub fn parse_eval_block(parser: &mut Parser<'_>) -> Expr {
    Expr::EvalBlock(Box::from(parse_block(parser)))
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
    parser.next_token_expect(
        Token::LParen,
        "Function arguments must be delimited by parentheses",
    );
    let mut args: Vec<(SmolStr, Option<TypeExpr>)> = Vec::with_capacity(4);
    loop {
        if parser.peek_token() == Token::RParen {
            parser.next_token();
            break;
        }
        let (arg, span) = parser.next_token();
        if let Token::Identifier(arg) = arg {
            args.push((
                SmolStr::new(arg),
                if parser.peek_token() == Token::Colon {
                    parser.next_token();
                    Some(parse_type(parser))
                } else {
                    None
                },
            ));
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(
                    Token::Identifier(""),
                    arg,
                    "Invalid function argument.",
                ),
            );
        }
        if parser.peek_token() == Token::Comma {
            parser.next_token();
        } else if !(parser.peek_token() == Token::RParen) {
            cold_path();
            let span = parser.peek_token_span();
            parser.error(span, ParserErr::ArgumentsMissingCommaSeparator);
        }
    }
    let return_type = parse_return_annotation(parser);
    let fn_code = parse_block(parser);
    Expr::FunctionDecl(
        fn_name,
        Box::from(args),
        std::rc::Rc::from(fn_code),
        span,
        return_type,
        type_params,
    )
}

/// Parses the optional `-> Type` return annotation that may follow a function's
/// parameter list, returning it with the span of the annotated type.
///
/// The annotation is checked against what the body returns; see
/// `compile_function`. Leaving it off keeps the return type inferred.
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
        let (next_token, _) = parser.next_token();
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
                Box::new(Expr::String(e)),
                Box::new(Expr::Var(catch_all_var.clone(), catch_span)),
            );
            output_code.extend(c);
        } else {
            output_code.push(Expr::ElseIfBlock(
                Box::new(Expr::Eq(
                    Box::new(Expr::String(e)),
                    Box::new(Expr::Var(catch_all_var.clone(), catch_span)),
                )),
                Box::from(c),
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
    loop {
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
    Expr::StructDeclare(struct_name, Box::from(fields), span, type_params)
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
/// A block whose header names type arguments (`impl Cell<T>`, `impl Cell<int>`)
/// has no instantiated type to mangle against yet: it is kept as a template in
/// `impls` and lowered the same way once the type is instantiated.
pub fn parse_impl_block(
    parser: &mut Parser<'_>,
    output: &mut Vec<Expr>,
    impls: &mut Vec<ImplTemplate>,
) {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Impl);
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
    let header_span: Span = (start, parser.last_token_end as u32).into();
    parser.next_token_expect(Token::LBrace, "impl blocks must start with '{'.");
    let mut methods: Vec<Expr> = Vec::with_capacity(4);
    loop {
        if parser.peek_token() == Token::RBrace {
            parser.next_token();
            break;
        }
        methods.push(parse_method(parser));
    }
    if type_args.is_empty() {
        for method in methods {
            output.push(mangled_method(method, &type_name));
        }
    } else {
        impls.push(ImplTemplate {
            type_name,
            args: type_args,
            methods: Box::from(methods),
            // Stamped with the file it came from when it is registered.
            file_idx: 0,
            span: header_span,
        });
    }
}

/// Renames a parsed method to the mangled free-function symbol its call sites
/// resolve to.
fn mangled_method(method: Expr, type_name: &SmolStr) -> Expr {
    let Expr::FunctionDecl(name, args, code, name_span, return_type, type_params) = method else {
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
    )
}

/// Parses a single `fn method(self, ...) [-> Type] { ... }` inside an impl block
/// into an [`Expr::FunctionDecl`] carrying the plain method name.
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
    let Token::Identifier(method_name) = t_id else {
        cold_path();
        parser.error(
            name_span,
            ParserErr::UnexpectedToken(Token::Identifier(""), t_id, "Invalid method name."),
        );
    };
    let type_params = parse_type_params(parser);
    parser.next_token_expect(
        Token::LParen,
        "Method arguments must be delimited by parentheses",
    );
    let mut args: Vec<(SmolStr, Option<TypeExpr>)> = Vec::with_capacity(4);
    loop {
        if parser.peek_token() == Token::RParen {
            parser.next_token();
            break;
        }
        let (arg, span) = parser.next_token();
        if let Token::Identifier(arg) = arg {
            // A parameter left un-annotated (including the receiver) is inferred
            // per call site: candela specialises each method on the actual
            // argument types, so `self` takes the receiver's concrete struct
            // type automatically. An annotation pins the parameter instead,
            // exactly as it does on a free function.
            args.push((
                SmolStr::new(arg),
                if parser.peek_token() == Token::Colon {
                    parser.next_token();
                    Some(parse_type(parser))
                } else {
                    None
                },
            ));
        } else {
            cold_path();
            parser.error(
                span,
                ParserErr::UnexpectedToken(Token::Identifier(""), arg, "Invalid method argument."),
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
    let return_type = parse_return_annotation(parser);
    let code = parse_block(parser);
    Expr::FunctionDecl(
        SmolStr::new(method_name),
        Box::from(args),
        Rc::from(code),
        name_span,
        return_type,
        type_params,
    )
}

pub fn parse_loop_block(input: &mut Parser<'_>) -> Expr {
    let (t, _) = input.next_token();
    debug_assert_eq!(t, Token::Loop);
    Expr::LoopBlock(Box::from(parse_block(input)))
}

/// Parses a `match` block into an [`Expr::Match`] carrying the scrutinee, the
/// arm patterns (as raw expressions) and their bodies, and an optional wildcard
/// body. The compiler picks the lowering by the scrutinee's static type: an
/// enum scrutinee gives variant-pattern matching with payload binding; any
/// other scrutinee gives the equality-chain behavior.
pub fn parse_match(parser: &mut Parser<'_>) -> Expr {
    let (t, Span { start, end: _ }) = parser.next_token();
    debug_assert_eq!(t, Token::Match);
    let match_obj = parse_expr_no_struct(parser);
    parser.next_token_expect(Token::LBrace, "Blocks must be delimited by braces");
    let mut arms: Vec<(Expr, Box<[Expr]>)> = Vec::with_capacity(2);
    let mut wildcard: Option<Box<[Expr]>> = None;
    let mut has_non_wildcard = false;
    let end: u32;
    loop {
        let peek_token = parser.peek_token();
        if peek_token == Token::Identifier("_") {
            if !has_non_wildcard {
                cold_path();
                let span = (start, parser.peek_token_span().end).into();
                parser.error(span, ParserErr::MatchBlockNoNonWildcardArm);
            }
            parser.next_token();
            parser.next_token_expect(Token::FatArrow, "Expected '=>'");
            let code = parse_block(parser);
            end = parser.peek_token_span().end;
            parser.next_token_expect(
                Token::RBrace,
                "The wildcard must be the last statement in a match",
            );
            wildcard = Some(Box::from(code));
            break;
        } else if peek_token == Token::RBrace {
            if !has_non_wildcard {
                cold_path();
                let span = (start, parser.peek_token_span().end).into();
                parser.error(span, ParserErr::MatchBlockZeroArms);
            }
            end = parser.peek_token_span().end;
            parser.next_token();
            break;
        } else {
            let pattern = parse_expr(parser);
            parser.next_token_expect(Token::FatArrow, "");
            let code = parse_block(parser);
            has_non_wildcard = true;
            arms.push((pattern, Box::from(code)));
        }
    }
    Expr::Match(
        Box::new(match_obj),
        Box::from(arms),
        wildcard,
        (start, end).into(),
    )
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
    loop {
        if parser.peek_token() == Token::RBrace {
            parser.next_token();
            break;
        }
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
        }
        variants.push((variant_name, Box::from(payload), v_span));
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
    Expr::EnumDeclare(enum_name, Box::from(variants), span, type_params)
}
