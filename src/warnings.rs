//! Compile warnings: findings that do not stop a build.
//!
//! The error funnel is fatal on the first error, so it has one slot. A warning
//! leaves the compile running, so any number of them can be raised, and they
//! travel on a channel of their own. [`collect_warnings`] gathers the warnings
//! raised while it runs, the way `collect_diagnostic` gathers an error; with no
//! collector active a warning is printed to stderr as a report, which is what
//! `candela check` and `candela build` show.
//!
//! This lives on the compiler side. The VM raises no warnings, so candela-vm
//! carries none of it.

use crate::Diagnostic;
use crate::rt::Source;
use ariadne::Color;
use ariadne::Label;
use ariadne::Report;
use ariadne::ReportKind;
use candela_vm::captured_output;
use std::cell::RefCell;

thread_local! {
    /// The warnings raised under the innermost [`collect_warnings`], or `None`
    /// when no collector is active.
    static WARNINGS: RefCell<Option<Vec<Diagnostic>>> = const { RefCell::new(None) };
}

/// Puts the enclosing collector's warnings back when a nested one ends, even
/// when it ends by unwinding out of a compile error.
struct Restore(Option<Vec<Diagnostic>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        WARNINGS.with(|w| *w.borrow_mut() = previous);
    }
}

/// Runs `f` and returns what it produced together with every warning raised
/// while it ran, in the order they were raised. Nothing is printed.
///
/// Collectors nest: an inner one takes the warnings raised inside it, and the
/// outer one sees none of them.
pub fn collect_warnings<R>(f: impl FnOnce() -> R) -> (R, Vec<Diagnostic>) {
    let previous = WARNINGS.with(|w| w.borrow_mut().replace(Vec::new()));
    let restore = Restore(previous);
    let result = f();
    let warnings = WARNINGS.with(|w| w.borrow_mut().take()).unwrap_or_default();
    drop(restore);
    (result, warnings)
}

/// Raises `warning`. Under [`collect_warnings`] it is recorded; otherwise it
/// is printed to stderr as a report titled `title`, labelled at its span in
/// the source it names.
pub fn emit_warning(sources: &[Source], title: &str, warning: Diagnostic) {
    let unrecorded = WARNINGS.with(|w| match w.borrow_mut().as_mut() {
        Some(warnings) => {
            warnings.push(warning);
            None
        }
        None => Some(warning),
    });
    let Some(warning) = unrecorded else {
        return;
    };
    let contents = sources
        .iter()
        .find(|source| source.filename == warning.filename)
        .map_or("", |source| source.contents.as_str());
    let filename = warning.filename.as_str();
    let report = Report::build(ReportKind::Warning, (filename, warning.span.clone()))
        .with_message(title)
        .with_label(
            Label::new((filename, warning.span.clone()))
                .with_message(warning.message.as_str())
                .with_color(Color::Yellow),
        )
        .finish();
    // A stderr that refuses the report costs the report, not the build.
    let _ = report.write(
        (filename, ariadne::Source::from(contents)),
        &mut captured_output::stderr(),
    );
}
