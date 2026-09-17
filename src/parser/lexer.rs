use crate::cold_path;
use crate::macros::region_len;
use logos::Lexer;
use logos::Logos;
use smol_strc::SmolStr;
use std::hint::unreachable_unchecked;

impl std::fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Macro(m) => write!(f, "the macro '{}!'", m.name),
            Token::Identifier(_) => write!(f, "an identifier"),
            Token::Int(_) => write!(f, "an integer"),
            Token::Float(_) => write!(f, "a float"),
            Token::String(_) => write!(f, "a string"),
            Token::LBrace => write!(f, "'{{'"),
            Token::RBrace => write!(f, "'}}'"),
            Token::LParen => write!(f, "'('"),
            Token::RParen => write!(f, "')'"),
            Token::LBracket => write!(f, "'['"),
            Token::RBracket => write!(f, "']'"),
            Token::FatArrow => write!(f, "'=>'"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// Why the lexer refused the text it was looking at.
///
/// [`LexErr::UnknownToken`] is what no rule matching leaves behind, and is the
/// default the lexer fills in. The other variants come from a rule that matched
/// and then rejected what it read, so the parser reports the mistake the
/// literal made rather than a bare unknown token.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum LexErr {
    #[default]
    UnknownToken,
    /// An integer literal outside what `int` holds.
    IntOutOfRange,
    /// A number whose exponent marker carries no digits, such as `1e`.
    FloatExponentMissingDigits,
}

#[derive(Logos, Debug, PartialEq, Clone, Copy)]
#[logos(error = LexErr)]
#[logos(skip r"[ \t\r\n\f]+")] // Ignore whitespace
#[logos(skip(r"//[^\n\r]*", allow_greedy = true))] // Ignore comments
pub enum Token<'a> {
    // ASSIGNEMENT OPS
    #[token("+=")]
    AssignOpAdd,
    #[token("-=")]
    AssignOpSub,
    #[token("*=")]
    AssignOpMul,
    #[token("/=")]
    AssignOpDiv,
    #[token("%=")]
    AssignOpMod,
    #[token("^=")]
    AssignOpPow,

    // OPS
    #[token("||")]
    OpOr,
    #[token("|")]
    Pipe,
    #[token("&&")]
    OpAnd,
    #[token("==")]
    OpEq,
    #[token("!=")]
    OpNEq,
    #[token("<=")]
    OpInfEq,
    #[token("<")]
    OpInf,
    #[token(">=")]
    OpSupEq,
    #[token(">")]
    OpSup,
    #[token("+")]
    OpAdd,
    #[token("-")]
    OpSub,
    #[token("*")]
    OpMul,
    #[token("/")]
    OpDiv,
    #[token("%")]
    OpMod,
    #[token("^")]
    OpPow,
    #[token("!")]
    OpNot,
    #[token("=")]
    Equals,
    #[token("null")]
    Null,
    #[token("false")]
    False,
    #[token("true")]
    True,
    #[token("dylib")]
    Dylib,
    #[token("host")]
    Host,
    #[token("import")]
    Import,
    #[token("as")]
    As,
    #[token("fn")]
    Function,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("match")]
    Match,
    #[token("while")]
    While,
    #[token("for")]
    For,
    #[token("in")]
    In,
    #[token("try")]
    Try,
    #[token("catch")]
    Catch,
    #[token("struct")]
    Struct,
    #[token("enum")]
    Enum,
    #[token("impl")]
    Impl,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,
    #[token("loop")]
    Loop,
    #[token("let")]
    Let,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token(",")]
    Comma,
    #[token("...")]
    Ellipsis,
    #[token("..")]
    RangeDot,
    #[token(".")]
    Dot,
    #[token("::")]
    DoubleColon,
    #[token(":")]
    Colon,
    #[token(";")]
    SemiColon,
    #[token("=>")]
    /// =>
    FatArrow,
    #[token("->")]
    /// ->
    Arrow,

    #[regex(r#"\"(?:[^\"\\]|\\.)*\""#, |lex| lex.slice())]
    String(&'a str),

    /// A macro invocation, `name!( ... )`. The pattern matches the head; the
    /// callback consumes the region, which no other token rule can describe
    /// because its end is found by balancing parentheses rather than by a
    /// regular expression.
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*!\(", lex_macro)]
    Macro(MacroToken<'a>),

    #[regex("[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice())]
    Identifier(&'a str),

    // Three spellings of the same type: a decimal point, an exponent, or both.
    // The exponent form is what lets a constant like 6.02e23 be written at the
    // scale it is quoted at.
    #[regex(r"[0-9]*[.][0-9]+", |lex| {
        let slice = lex.slice();
        lexical_core::parse::<f64>(slice.as_bytes()).unwrap()
    })]
    #[regex(r"([0-9]*[.])?[0-9]+[eE][+-]?[0-9]+", |lex| {
        let slice = lex.slice();
        lexical_core::parse::<f64>(slice.as_bytes()).unwrap()
    })]
    // An exponent marker with nothing after it. The rule exists so the number
    // is refused as a number, rather than read as a smaller one followed by a
    // name the file never declared.
    #[regex(r"([0-9]*[.])?[0-9]+[eE][+-]?", exponent_without_digits)]
    Float(f64),

    // `9223372036854775808` on its own is one past `int`, and is accepted
    // because the only way to write the smallest `int` is to negate it: the
    // '-' is a separate token, so the literal is read before the sign reaches
    // it.
    #[regex(r"[0-9]+", |lex| {
        let slice = lex.slice();
        match lexical_core::parse::<i64>(slice.as_bytes()) {
            Ok(v) => Ok(v),
            _ => match lexical_core::parse::<u64>(slice.as_bytes()) {
                Ok(v) if v == (i64::MAX as u64) + 1 => Ok(i64::MIN),
                _ => {
                    cold_path();
                    Err(LexErr::IntOutOfRange)
                }
            },
        }
    })]
    Int(i64),
}

/// A macro invocation as the lexer sees it: the name, and the raw text of the
/// region it opens. `body` is `None` for a region that the file ends before
/// closing, which the parser reports against the invocation.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MacroToken<'a> {
    pub name: &'a str,
    pub body: Option<&'a str>,
}

/// Refuses a number whose exponent marker has no digits after it.
const fn exponent_without_digits<'a>(_: &mut Lexer<'a, Token<'a>>) -> Result<f64, LexErr> {
    cold_path();
    Err(LexErr::FloatExponentMissingDigits)
}

/// Consumes the region opened by a `name!(` head and yields it whole. The token
/// the lexer emits therefore spans the entire invocation, so the parser can
/// place both the region body and any error inside it in the file.
fn lex_macro<'a>(lex: &mut Lexer<'a, Token<'a>>) -> MacroToken<'a> {
    let head = lex.slice();
    let name = &head[..head.len() - 2];
    let rest = lex.remainder();
    let Some(len) = region_len(rest) else {
        cold_path();
        lex.bump(rest.len());
        return MacroToken { name, body: None };
    };
    lex.bump(len + 1);
    MacroToken {
        name,
        body: Some(&rest[..len]),
    }
}

/// Strips the surrounding quotes & processes escape sequences \n \t \r \\ \" \0
pub fn parse_string(s: &str) -> SmolStr {
    let inner = &s[1..s.len() - 1]; // Strip the surrounding quotes

    // Return the stripped string directly if it doesn't contain any escape sequences
    let Some(first_escape) = memchr::memchr(b'\\', inner.as_bytes()) else {
        return SmolStr::from(inner);
    };
    let mut processed = String::with_capacity(inner.len());

    // Find returns the first occurence, so we know that inner[..first_escape] does not contain any escape sequence
    processed.push_str(&inner[..first_escape]);
    let mut to_process = &inner[first_escape..];
    loop {
        match memchr::memchr(b'\\', to_process.as_bytes()) {
            None => {
                // there are no escape sequences left
                processed.push_str(to_process);
                break;
            }
            Some(escape_seq_idx) => {
                processed.push_str(&to_process[..escape_seq_idx]);
                let after = &to_process[escape_seq_idx + 1..];
                if after.is_empty() {
                    processed.push('\\');
                    break;
                }
                let escape_seq = after.as_bytes()[0];
                if escape_seq == b'n'
                    || escape_seq == b't'
                    || escape_seq == b'r'
                    || escape_seq == b'\\'
                    || escape_seq == b'"'
                    || escape_seq == b'0'
                {
                    processed.push(match escape_seq {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'\\' => '\\',
                        b'"' => '"',
                        b'0' => '\0',
                        _ => unsafe { unreachable_unchecked() },
                    });
                    to_process = &after[1..];
                } else {
                    // chars() is used to correctly handle multi-byte characters
                    let c = after.chars().next().unwrap();
                    processed.push('\\');
                    processed.push(c);
                    to_process = &after[c.len_utf8()..];
                }
            }
        }
    }
    SmolStr::from(processed)
}
