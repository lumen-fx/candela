//! Tuple patterns: `let (a, b) = t;` and a `match` arm written `(0, y)`.
//!
//! Neither needs an instruction of its own. A `let` pattern is declared by
//! the parser as one variable holding the tuple and one per name reading its
//! element (see `parse_tuple_let`); what is left here is checking that the
//! value has as many elements as the pattern has places. A `match` arm with a
//! tuple pattern joins the equality chain every non-enum `match` lowers to:
//! the arm's test compares the elements the pattern writes a value for, and
//! the arm's body starts by declaring the names the pattern binds.

use super::compiler_data::Ctx;
use super::compiler_data::State;
use super::compiler_errors;
use super::expr::Expr;
use super::expr::Span;
use super::expr::tuple_elements;
use super::type_system::DataType;
use super::type_system::tuple_field;
use smol_strc::SmolStr;

/// The element types of `t` when it is a tuple of `places` elements.
fn tuple_of(t: &DataType, places: usize, state: &State<'_>) -> Option<Vec<DataType>> {
    let DataType::Struct(id) = t else {
        return None;
    };
    let s = &state.structs[*id as usize];
    (s.is_tuple() && s.fields.len() == places)
        .then(|| s.fields.iter().map(|(_, ty, _)| ty.clone()).collect())
}

/// Reports a pattern of `places` places held against a value of type
/// `found` that is not a tuple of that size.
pub fn check_tuple_places(
    places: usize,
    found: &DataType,
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
) {
    if tuple_of(found, places, state).is_none() {
        compiler_errors::error_tuple_pattern_mismatch(
            ctx.file_idx,
            span,
            places,
            found,
            state.sources,
            state.type_names(),
        );
    }
}

/// Whether any `|` alternative of an arm is a tuple pattern.
#[must_use]
pub fn arm_has_tuple_pattern(patterns: &[Expr]) -> bool {
    patterns.iter().any(|p| tuple_elements(p).is_some())
}

/// What one place of a tuple pattern does: binds a name, ignores the element
/// (`_`), holds a nested pattern, or compares the element with a value.
enum Place<'a> {
    Bind(&'a SmolStr),
    Ignore,
    Nested(Vec<(&'a Expr, Span)>),
    Compare(&'a Expr),
}

fn place(element: &Expr) -> Place<'_> {
    if let Some(inner) = tuple_elements(element) {
        return Place::Nested(inner);
    }
    match element {
        Expr::Var(name, _) if name == "_" => Place::Ignore,
        Expr::Var(name, _) => Place::Bind(name),
        other => Place::Compare(other),
    }
}

/// The names one alternative binds, each with the type of the element it
/// binds, in name order. A tuple pattern is checked against `scrut_type` on
/// the way, nested patterns included; any other alternative binds nothing.
fn alternative_bindings(
    pattern: &Expr,
    scrut_type: &DataType,
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
    out: &mut Vec<(SmolStr, DataType)>,
) {
    let Some(elements) = tuple_elements(pattern) else {
        return;
    };
    let pattern_span = match pattern {
        Expr::Struct(_, _, s, _, _) => *s,
        _ => span,
    };
    let Some(types) = tuple_of(scrut_type, elements.len(), state) else {
        compiler_errors::error_tuple_pattern_mismatch(
            ctx.file_idx,
            pattern_span,
            elements.len(),
            scrut_type,
            state.sources,
            state.type_names(),
        );
    };
    for ((element, _), ty) in elements.iter().zip(types.iter()) {
        match place(element) {
            Place::Bind(name) => out.push((name.clone(), ty.clone())),
            Place::Nested(_) => alternative_bindings(element, ty, span, ctx, state, out),
            Place::Ignore | Place::Compare(_) => {}
        }
    }
}

/// The names an arm's patterns bind, with their types, for code that reads
/// the arm body without compiling it. Every `|` alternative has to bind the
/// same names at the same types, since the body reads them whichever
/// alternative matched; one that does not is reported here.
pub fn arm_bindings(
    patterns: &[Expr],
    scrut_type: &DataType,
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> Vec<(SmolStr, DataType)> {
    let mut expected = Vec::new();
    alternative_bindings(&patterns[0], scrut_type, span, ctx, state, &mut expected);
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    for pattern in &patterns[1..] {
        let mut found = Vec::new();
        alternative_bindings(pattern, scrut_type, span, ctx, state, &mut found);
        found.sort_by(|a, b| a.0.cmp(&b.0));
        if found != expected {
            let at = match pattern {
                Expr::Struct(_, _, s, _, _) | Expr::Var(_, s) => *s,
                _ => span,
            };
            compiler_errors::error_pattern_bindings_differ(
                &expected,
                &found,
                at,
                ctx.file_idx,
                state.sources,
                state.type_names(),
            );
        }
    }
    expected
}

/// Collects what one tuple pattern held against `path` tests and binds.
fn lower_pattern(
    path: &Expr,
    elements: &[(&Expr, Span)],
    span: Span,
    tests: &mut Vec<Expr>,
    binds: &mut Vec<(SmolStr, Expr)>,
) {
    for (i, (element, element_span)) in elements.iter().enumerate() {
        let element_path =
            Expr::GetStructField(Box::new(path.clone()), tuple_field(i), span, *element_span);
        match place(element) {
            Place::Bind(name) => binds.push((name.clone(), element_path)),
            Place::Ignore => {}
            Place::Nested(inner) => lower_pattern(&element_path, &inner, span, tests, binds),
            Place::Compare(value) => tests.push(Expr::Eq(
                Box::new(element_path),
                Box::new(value.clone()),
                span,
                *element_span,
            )),
        }
    }
}

/// `a && b && ...`, or `true` when there is nothing to test.
fn all_of(tests: Vec<Expr>, span: Span) -> Expr {
    tests
        .into_iter()
        .reduce(|acc, test| Expr::BoolAnd(Box::new(acc), Box::new(test), span, span))
        .unwrap_or(Expr::Bool(true))
}

/// The test and the body an arm of an equality `match` runs when one of its
/// alternatives is a tuple pattern. `scrutinee` names the variable holding
/// the matched value.
///
/// The test holds when any alternative does: a tuple pattern when every
/// element it writes a value for is equal to it, any other alternative when
/// the whole value is. The body declares each bound name first, read from the
/// alternative that matched.
#[must_use]
pub fn tuple_arm(
    scrutinee: &SmolStr,
    patterns: &[Expr],
    body: &[Expr],
    span: Span,
) -> (Expr, Box<[Expr]>) {
    let path = Expr::Var(scrutinee.clone(), span);
    let mut alternatives: Vec<(Expr, Vec<(SmolStr, Expr)>)> = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let mut tests = Vec::new();
        let mut binds = Vec::new();
        if let Some(elements) = tuple_elements(pattern) {
            lower_pattern(&path, &elements, span, &mut tests, &mut binds);
        } else {
            tests.push(Expr::Eq(
                Box::new(path.clone()),
                Box::new(pattern.clone()),
                span,
                span,
            ));
        }
        alternatives.push((all_of(tests, span), binds));
    }
    let test = alternatives
        .iter()
        .map(|(test, _)| test.clone())
        .reduce(|acc, test| Expr::BoolOr(Box::new(acc), Box::new(test), span, span))
        .unwrap_or(Expr::Bool(false));

    let mut code: Vec<Expr> = Vec::with_capacity(body.len() + alternatives[0].1.len());
    for (name, first_path) in &alternatives[0].1 {
        let value = if alternatives.len() == 1 {
            first_path.clone()
        } else {
            // Every alternative binds this name (`arm_bindings` checked), so
            // the value is read from whichever one matched, the last one
            // standing for itself.
            let read = |k: usize| -> Expr {
                alternatives[k]
                    .1
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or(Expr::Null, |(_, p)| p.clone())
            };
            let last = alternatives.len() - 1;
            let mut branches: Vec<Expr> = vec![read(0)];
            for (k, (test, _)) in alternatives.iter().enumerate().take(last).skip(1) {
                branches.push(Expr::ElseIfBlock(
                    Box::new(test.clone()),
                    Box::from([read(k)]),
                    span,
                ));
            }
            branches.push(Expr::ElseBlock(Box::from([read(last)])));
            Expr::InlineCondition(
                Box::new(alternatives[0].0.clone()),
                Box::from(branches),
                span,
                span,
            )
        };
        code.push(Expr::VarDeclare(name.clone(), Box::new(value), None));
    }
    code.extend(body.iter().cloned());
    (test, Box::from(code))
}

/// Whether an arm always matches: one of its alternatives is a tuple pattern
/// made of names and `_` only, all the way down. Such an arm, written last,
/// covers every case the way a `_` arm does.
#[must_use]
pub fn arm_is_irrefutable(patterns: &[Expr]) -> bool {
    fn irrefutable(elements: &[(&Expr, Span)]) -> bool {
        elements.iter().all(|(element, _)| match place(element) {
            Place::Bind(_) | Place::Ignore => true,
            Place::Nested(inner) => irrefutable(&inner),
            Place::Compare(_) => false,
        })
    }
    patterns
        .iter()
        .any(|p| tuple_elements(p).is_some_and(|elements| irrefutable(&elements)))
}
