//! Named arguments and parameter defaults.
//!
//! A call passes its arguments in order, then by name: `window(800, 600,
//! title: "x")`. Before anything compiles or types the call, the arguments are
//! put in the order the callee declares its parameters, and every parameter the
//! call leaves out takes the default it was declared with. What comes out is
//! the full argument list a call written in order would have, so the rest of
//! the compiler, and the program at run time, never sees a name.

use super::Source;
use super::compiler_data::Ctx;
use super::compiler_data::State;
use super::compiler_errors;
use super::expr::Expr;
use super::expr::METHOD_SEP;
use super::expr::Span;
use super::expr::TRAILING_ARG;
use smol_strc::SmolStr;

/// The parameters a call binds its arguments against.
pub struct Params<'a> {
    /// What the callee is called, for the reports.
    pub callee: &'a str,
    /// Every parameter's name, in declaration order.
    pub names: &'a [SmolStr],
    /// The default of each parameter that declares one, by the id
    /// [`crate::compiler::type_system::Generics::arg_default`] reads; empty
    /// when none does.
    pub defaults: &'a [Option<u32>],
}

/// The arguments of a call in the order `params` declares them, with every
/// argument the call leaves out filled from its default.
///
/// `None` when the call needs nothing done: it names no argument and passes
/// every parameter that has no default, or passes more arguments than there
/// are parameters, which the arity check reports.
pub fn bind(
    params: &Params<'_>,
    args: &[Expr],
    arg_spans: &[Span],
    span: Span,
    file_idx: u16,
    sources: &[Source],
) -> Option<(Box<[Expr]>, Box<[Span]>)> {
    let named = args.iter().any(|arg| matches!(arg, Expr::NamedArg(..)));
    if !named && args.len() >= params.names.len() {
        return None;
    }
    let mut slots: Vec<Option<(Expr, Span)>> = vec![None; params.names.len()];
    for (i, arg) in args.iter().enumerate() {
        let arg_span = arg_spans.get(i).copied().unwrap_or(span);
        if let Expr::NamedArg(name, value, closure_span) = arg
            && name == TRAILING_ARG
        {
            // The closure after the call binds the last parameter.
            let Some(idx) = slots
                .len()
                .checked_sub(1)
                .filter(|&idx| slots[idx].is_none())
            else {
                compiler_errors::error_type_test(
                    "Closure after a full call",
                    &match params.names.last() {
                        Some(last) => format!(
                            "{}'s last parameter, {last}, already has a value in this call",
                            params.callee
                        ),
                        None => format!("{} takes no arguments", params.callee),
                    },
                    Some(
                        "A closure after the call is the argument of the last parameter. Leave that one out of the parentheses",
                    ),
                    "trailing_closure_extra",
                    *closure_span,
                    file_idx,
                    sources,
                );
            };
            slots[idx] = Some(((**value).clone(), arg_span));
        } else if let Expr::NamedArg(name, value, name_span) = arg {
            let Some(idx) = params.names.iter().position(|p| p == name) else {
                compiler_errors::error_type_test(
                    "Unknown parameter name",
                    &format!("{} has no parameter named {name}", params.callee),
                    Some(&format!(
                        "Its parameters are {}",
                        if params.names.is_empty() {
                            String::from("none")
                        } else {
                            params.names.join(", ")
                        }
                    )),
                    "named_argument_unknown",
                    *name_span,
                    file_idx,
                    sources,
                );
            };
            if slots[idx].is_some() {
                compiler_errors::error_type_test(
                    "Argument given twice",
                    &format!("{name} already has a value in this call"),
                    None,
                    "named_argument_twice",
                    *name_span,
                    file_idx,
                    sources,
                );
            }
            slots[idx] = Some(((**value).clone(), arg_span));
        } else if i < slots.len() {
            slots[i] = Some((arg.clone(), arg_span));
        } else {
            // More arguments in order than there are parameters: the arity
            // check reports the count.
            return None;
        }
    }
    let mut bound = Vec::with_capacity(slots.len());
    let mut spans = Vec::with_capacity(slots.len());
    for (idx, slot) in slots.into_iter().enumerate() {
        if let Some((value, value_span)) = slot {
            bound.push(value);
            spans.push(value_span);
        } else if let Some(Some(id)) = params.defaults.get(idx) {
            bound.push(Expr::DefaultArg(*id, span));
            spans.push(span);
        } else if named {
            compiler_errors::error_type_test(
                "Missing argument",
                &format!("{} needs a value for {}", params.callee, params.names[idx]),
                Some(&format!(
                    "Pass it in order or by name: {}: ...",
                    params.names[idx]
                )),
                "argument_missing",
                span,
                file_idx,
                sources,
            );
        } else {
            // A call in order that is short of a parameter with no default:
            // the arity check reports it the way it always has.
            return None;
        }
    }
    Some((bound.into_boxed_slice(), spans.into_boxed_slice()))
}

/// Refuses an argument passed by name where the callee has no parameter names
/// to bind it to: a built-in, a function held in a value, a variant.
#[cold]
pub fn error_named_argument_unexpected(
    name: &str,
    span: Span,
    file_idx: u16,
    sources: &[Source],
) -> ! {
    compiler_errors::error_type_test(
        "Argument passed by name",
        &format!("{name}: names a parameter, and this call takes its arguments in order"),
        Some("Only a call to a declared function passes arguments by name. Pass this one in order"),
        "named_argument_unexpected",
        span,
        file_idx,
        sources,
    )
}

/// [`bind`] for a call of user function `fn_id`, whose first `skip`
/// parameters the call fills in itself (a method's receiver, which a dot call
/// passes from in front of the dot).
pub fn bind_user_args(
    fn_id: usize,
    skip: usize,
    args: &[Expr],
    arg_spans: &[Span],
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> Option<(Box<[Expr]>, Box<[Span]>)> {
    let func = &state.fns[fn_id];
    let defaults = u16::try_from(fn_id)
        .ok()
        .and_then(|id| state.generics.fn_defaults(id))
        .unwrap_or(&[]);
    let declared = func.args.len().saturating_sub(skip);
    let named = args.iter().any(|arg| matches!(arg, Expr::NamedArg(..)));
    if !named && (args.len() >= declared || defaults.is_empty()) {
        return None;
    }
    let names: Vec<SmolStr> = func.args[skip.min(func.args.len())..]
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let callee = func.name.replace(METHOD_SEP, "::");
    bind(
        &Params {
            callee: &callee,
            names: &names,
            defaults: defaults.get(skip..).unwrap_or(&[]),
        },
        args,
        arg_spans,
        span,
        ctx.file_idx,
        state.sources,
    )
}
