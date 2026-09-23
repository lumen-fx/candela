//! Compile-time configuration: the names `@cfg(...)` tests.
//!
//! An item or statement written after `@cfg(predicate)` is compiled only when
//! the predicate holds, and is dropped before name resolution otherwise, so the
//! code it holds never has to resolve. candela names no configuration of its
//! own. The program embedding it decides which flags are on, the way a host
//! compiling for a browser turns on `web`:
//!
//! ```no_run
//! use candela::Cfg;
//!
//! let mut cfg = Cfg::new();
//! cfg.enable("web");
//! cfg.enable_value("target", "wasm32");
//! let engine = candela::Engine::new().with_cfg(cfg);
//! ```
//!
//! A compile that does not go through an [`crate::Engine`] runs inside
//! [`Cfg::scope`] instead. A flag nobody enabled is false.

use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use std::cell::RefCell;

/// The flags and `key = "value"` pairs a compile treats as true.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cfg {
    flags: FxHashSet<SmolStr>,
    values: FxHashSet<(SmolStr, SmolStr)>,
}

impl Cfg {
    /// A configuration with nothing enabled: every `@cfg(flag)` is false.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns `flag` on, so `@cfg(flag)` holds.
    pub fn enable(&mut self, flag: &str) {
        self.flags.insert(SmolStr::new(flag));
    }

    /// Turns the pair on, so `@cfg(key = "value")` holds. A key may carry
    /// several values at once; each one is its own pair.
    pub fn enable_value(&mut self, key: &str, value: &str) {
        self.values.insert((SmolStr::new(key), SmolStr::new(value)));
    }

    /// Whether `@cfg(flag)` holds.
    #[must_use]
    pub fn is_enabled(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }

    /// Whether `@cfg(key = "value")` holds.
    #[must_use]
    pub fn has_value(&self, key: &str, value: &str) -> bool {
        self.values
            .contains(&(SmolStr::new(key), SmolStr::new(value)))
    }

    /// Runs `f` with this configuration active, and returns what it returns.
    ///
    /// Anything compiled inside `f` reads its `@cfg` attributes against this
    /// configuration, including the files it imports. The previous one is
    /// restored afterwards, including when `f` unwinds.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        let _active = Active::push(self.clone());
        f()
    }
}

/// Each name enables one flag.
impl<S: AsRef<str>> FromIterator<S> for Cfg {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        let mut cfg = Self::new();
        for flag in iter {
            cfg.enable(flag.as_ref());
        }
        cfg
    }
}

thread_local! {
    /// The configurations installed by the [`Cfg::scope`] calls currently on
    /// the stack; the last one is the active one.
    static ACTIVE: RefCell<Vec<Cfg>> = const { RefCell::new(Vec::new()) };
}

/// Pops the configuration it pushed when it is dropped, so a compile that
/// unwinds leaves the stack as it found it.
struct Active;

impl Active {
    fn push(cfg: Cfg) -> Self {
        ACTIVE.with(|stack| stack.borrow_mut().push(cfg));
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

/// The configuration a compile starting now reads: the innermost
/// [`Cfg::scope`], or nothing enabled outside of one.
pub(crate) fn active() -> Cfg {
    ACTIVE.with(|stack| stack.borrow().last().cloned().unwrap_or_default())
}
