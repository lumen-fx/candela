//! Macros: source regions an embedder expands.
//!
//! A macro invocation is written `name!( ... )`. Everything between the
//! parentheses is a raw region that candela does not interpret. The region is
//! handed to the expander registered under `name`, and the candela source the
//! expander returns is parsed in its place, as one expression.
//!
//! candela never learns what a macro means. It finds where a region starts and
//! ends, calls out, and parses what comes back. The meaning belongs to the host
//! that registers the expander.
//!
//! ```no_run
//! use candela::macros::MacroError;
//!
//! let mut engine = candela::Engine::new();
//! engine.register_macro("shout", |body: &str| {
//!     Ok::<String, MacroError>(format!("\"{}!\"", body.trim().to_uppercase()))
//! });
//! ```
//!
//! [`scan_regions`] exposes the same region scanner the lexer uses, so a build
//! tool can find every invocation of one macro in a file without compiling it.

use crate::cold_path;
use rustc_hash::FxHashMap;
use smol_strc::SmolStr;
use std::cell::Cell;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

/// Why an expander refused the region it was handed.
///
/// `offset` is a byte offset into the region body. When it is set, the error is
/// reported at that position in the file the macro was written in; otherwise it
/// is reported against the whole invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroError {
    pub message: String,
    pub offset: Option<usize>,
}

impl MacroError {
    /// An error covering the whole macro invocation.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            offset: None,
        }
    }

    /// An error at `offset` bytes into the region body.
    #[must_use]
    pub fn at(message: impl Into<String>, offset: usize) -> Self {
        Self {
            message: message.into(),
            offset: Some(offset),
        }
    }
}

/// One `name!( ... )` invocation found by [`scan_regions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region<'a> {
    /// The raw text between the parentheses.
    pub body: &'a str,
    /// Byte offset of `body` in the scanned source.
    pub body_start: usize,
    /// Byte range of the whole invocation, from the first byte of the name
    /// through the closing parenthesis.
    pub span: Range<usize>,
}

/// Finds every `name!( ... )` invocation in `src`, in source order.
///
/// The scan is the one the lexer runs: a region ends at the parenthesis that
/// balances the one that opened it, parentheses inside a string literal or a
/// `//` comment do not count, and a `name!(` written inside a string literal or
/// a comment is not an invocation. Scanning stops at a region that is never
/// closed, so every region returned is complete.
#[must_use]
pub fn scan_regions<'a>(src: &'a str, name: &str) -> Vec<Region<'a>> {
    let bytes = src.as_bytes();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            let Some(after) = skip_string(bytes, i) else {
                break;
            };
            i = after;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = skip_line_comment(bytes, i);
        } else if is_ident_start(b) {
            let start = i;
            let mut end = i + 1;
            while end < bytes.len() && is_ident_continue(bytes[end]) {
                end += 1;
            }
            if &src[start..end] == name
                && bytes.get(end) == Some(&b'!')
                && bytes.get(end + 1) == Some(&b'(')
            {
                let body_start = end + 2;
                let Some(len) = region_len(&src[body_start..]) else {
                    break;
                };
                let body_end = body_start + len;
                regions.push(Region {
                    body: &src[body_start..body_end],
                    body_start,
                    span: start..body_end + 1,
                });
                i = body_end + 1;
            } else {
                i = end;
            }
        } else {
            i += 1;
        }
    }
    regions
}

/// Length of the region body that begins at the start of `rest`, which is the
/// text just past an opening `(`. `None` when the region is never closed.
pub(crate) fn region_len(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut depth = 1usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = skip_string(bytes, i)?,
            b'/' if bytes.get(i + 1) == Some(&b'/') => i = skip_line_comment(bytes, i),
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

/// Byte index just past the string literal that opens at `start`. `None` when
/// the literal is never closed.
const fn skip_string(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// Byte index of the line ending that ends the `//` comment at `start`, or the
/// end of the input. Both `\n` and `\r` end a comment, as they do for the
/// lexer.
const fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 2;
    while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
        i += 1;
    }
    i
}

const fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

const fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

type Expander = dyn Fn(&str) -> Result<String, MacroError>;

/// The macros a compilation may use, and what an unregistered one does.
///
/// [`crate::Engine`] holds one of these and fills it through
/// [`crate::Engine::register_macro`]. A tool that compiles without the
/// embedder's expanders builds one directly, turns [`MacroEnv::allow_unknown`]
/// on, and compiles inside [`MacroEnv::scope`].
#[derive(Clone, Default)]
pub struct MacroEnv {
    expanders: FxHashMap<SmolStr, Rc<Expander>>,
    allow_unknown: bool,
}

impl MacroEnv {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `expander` as the meaning of `name!( ... )`.
    ///
    /// The expander receives the raw region body and returns candela source for
    /// one expression, or a [`MacroError`] that becomes a compile error at the
    /// macro site. Registering a name twice keeps the last expander.
    pub fn register<F>(&mut self, name: &str, expander: F)
    where
        F: Fn(&str) -> Result<String, MacroError> + 'static,
    {
        self.expanders.insert(SmolStr::new(name), Rc::new(expander));
    }

    /// Sets what an unregistered macro does: a compile error naming it (the
    /// default), or, when `allow` is true, a `null` expression.
    ///
    /// Permitting unknown macros is for tooling that reads source it cannot
    /// expand, such as the language server, which has no access to the host's
    /// expanders and must not report the host's macros as errors.
    pub const fn allow_unknown(&mut self, allow: bool) {
        self.allow_unknown = allow;
    }

    #[must_use]
    pub fn is_registered(&self, name: &str) -> bool {
        self.expanders.contains_key(name)
    }

    /// Runs `f` with this environment active, and returns what it returns.
    ///
    /// Anything compiled inside `f` expands its macros through this
    /// environment, including the files it imports. The previous environment is
    /// restored afterwards, including when `f` unwinds.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        let _active = Active::push(self.clone());
        f()
    }
}

thread_local! {
    /// The environments installed by the [`MacroEnv::scope`] calls currently on
    /// the stack; the last one is the active one.
    static ACTIVE: RefCell<Vec<MacroEnv>> = const { RefCell::new(Vec::new()) };
}

/// Pops the environment it pushed when it is dropped, so a compile that unwinds
/// leaves the stack as it found it.
struct Active;

impl Active {
    fn push(env: MacroEnv) -> Self {
        ACTIVE.with(|stack| stack.borrow_mut().push(env));
        Self
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// How far one macro may expand into another. An expansion is candela source
/// like any other, so it may use a macro itself; a macro that expands to a use
/// of itself would do that forever. Nesting this deep is past anything a host
/// generates, and short of the stack the compiler runs on.
pub(crate) const MAX_EXPANSION_DEPTH: u32 = 32;

thread_local! {
    /// How many expansions are being parsed right now.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// One level of macro expansion, held while the source it produced is parsed.
/// The level is given back when the guard drops, including when the compile
/// unwinds.
pub(crate) struct Depth;

impl Depth {
    /// Takes a level, or nothing when [`MAX_EXPANSION_DEPTH`] are already held.
    pub(crate) fn enter() -> Option<Self> {
        DEPTH.with(|depth| {
            let held = depth.get();
            if held >= MAX_EXPANSION_DEPTH {
                cold_path();
                return None;
            }
            depth.set(held + 1);
            Some(Self)
        })
    }
}

impl Drop for Depth {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

/// What the parser does with a macro region.
pub(crate) enum Expansion {
    /// Parse this source in place of the invocation.
    Text(String),
    /// No expander, and the active environment allows that: the invocation is a
    /// `null` expression.
    Null,
    /// No expander, and an unregistered macro is an error.
    Unknown,
    /// The expander refused.
    Failed(MacroError),
}

/// Expands `body` through the expander registered for `name` in the active
/// environment.
pub(crate) fn expand(name: &str, body: &str) -> Expansion {
    // The expander runs outside the borrow: it may compile candela itself, and
    // that pushes another environment onto this same stack.
    let active = ACTIVE.with(|stack| {
        stack
            .borrow()
            .last()
            .map(|env| (env.expanders.get(name).cloned(), env.allow_unknown))
    });
    match active {
        Some((Some(expander), _)) => match expander(body) {
            Ok(source) => Expansion::Text(source),
            Err(error) => Expansion::Failed(error),
        },
        Some((None, true)) => Expansion::Null,
        _ => Expansion::Unknown,
    }
}
