//! Patterns: one grammar for `match` arms, `is`, `let`, `for` and parameters.
//!
//! A pattern is resolved against the static type of the value it is held
//! against, then compiled to a run of tests that jump away when the value does
//! not fit and reads that put what it binds in registers. No instruction is
//! new: a variant test reads the tag, a field or an element is read out of
//! the struct holding it, and a literal, a constant or a range is compared the
//! way the same comparison written in an `if` is.

use super::Variable;
use super::compile_guard;
use super::compiler_data::Ctx;
use super::compiler_data::State;
use super::compiler_errors;
use super::expr::Expr;
use super::expr::Operand;
use super::expr::Span;
use super::expr::code_captures_variable;
use super::resolve_enum_variant;
use super::resolve_generic_variant;
use super::resolve_qualified_pattern_enum;
use super::set_jmp_size;
use super::type_system::DataType;
use super::type_system::TypeExpr;
use super::variant_shape;
use crate::compiler::expr::VariantShape;
use crate::instr::Instr;
use smol_strc::SmolStr;

/// A pattern as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `_`: matches anything and binds nothing.
    Wildcard(Span),
    /// A name that starts with a lowercase letter or `_`: matches anything and
    /// binds it.
    Bind(SmolStr, Span),
    /// A literal, compared by equality: `3`, `-1.5`, `"go"`, `true`, `null`.
    Value(Box<Expr>, Span),
    /// `low..high`, or `low..=high` when `inclusive`: a number in the range.
    Range(Box<Expr>, Box<Expr>, bool, Span),
    /// `(a, b)`: a tuple, element by element.
    Tuple(Box<[Self]>, Span),
    /// A path, with what it holds after it: a variant (`None`, `Some(x)`,
    /// `Click { x, y }`), a struct (`Point { x, y }`), or, with nothing after
    /// it, a constant the value is compared with.
    Named {
        path: Box<[SmolStr]>,
        type_args: Box<[TypeExpr]>,
        payload: Payload,
        span: Span,
    },
    /// `a | b`: either alternative. Every alternative binds the same names.
    Or(Box<[Self]>, Span),
}

/// What a [`Pattern::Named`] holds after its path.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// Nothing: `None`, `MAX`.
    None,
    /// In order, in parentheses: `Some(x)`.
    Positional(Box<[Pattern]>),
    /// By field name, in braces, `Point { x, y: py }`, and whether the list
    /// ends in `..`, which leaves the fields it does not name unchecked.
    Fields(Box<[(SmolStr, Pattern, Span)]>, bool),
}

/// Whether a name written alone in a pattern binds: it starts with a lowercase
/// letter or `_`. Any other name is a variant or a constant.
#[must_use]
pub fn is_binding_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
}

impl Pattern {
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Wildcard(span)
            | Self::Bind(_, span)
            | Self::Value(_, span)
            | Self::Range(_, _, _, span)
            | Self::Tuple(_, span)
            | Self::Named { span, .. }
            | Self::Or(_, span) => *span,
        }
    }

    /// The names the pattern binds, in the order they are written. Every
    /// alternative of an `|` binds the same ones, so the first says which.
    #[must_use]
    pub fn binders(&self) -> Vec<(SmolStr, Span)> {
        let mut out = Vec::new();
        self.collect_binders(&mut out);
        out
    }

    fn collect_binders(&self, out: &mut Vec<(SmolStr, Span)>) {
        match self {
            Self::Bind(name, span) => out.push((name.clone(), *span)),
            Self::Tuple(items, _)
            | Self::Named {
                payload: Payload::Positional(items),
                ..
            } => {
                for item in items {
                    item.collect_binders(out);
                }
            }
            Self::Named {
                payload: Payload::Fields(fields, _),
                ..
            } => {
                for (_, item, _) in fields {
                    item.collect_binders(out);
                }
            }
            Self::Or(alternatives, _) => alternatives[0].collect_binders(out),
            Self::Wildcard(_) | Self::Value(..) | Self::Range(..) | Self::Named { .. } => {}
        }
    }

    /// The expressions the pattern reads where it is written: the literals and
    /// bounds it compares with.
    #[must_use]
    pub fn reads(&self) -> Vec<&Expr> {
        let mut out = Vec::new();
        self.collect_reads(&mut out);
        out
    }

    fn collect_reads<'a>(&'a self, out: &mut Vec<&'a Expr>) {
        match self {
            Self::Value(value, _) => out.push(value),
            Self::Range(low, high, _, _) => {
                out.push(low);
                out.push(high);
            }
            Self::Tuple(items, _)
            | Self::Or(items, _)
            | Self::Named {
                payload: Payload::Positional(items),
                ..
            } => {
                for item in items {
                    item.collect_reads(out);
                }
            }
            Self::Named {
                payload: Payload::Fields(fields, _),
                ..
            } => {
                for (_, item, _) in fields {
                    item.collect_reads(out);
                }
            }
            Self::Wildcard(_) | Self::Bind(..) | Self::Named { .. } => {}
        }
    }

    /// Whether the pattern holds for any value of a type it fits, judged by
    /// its shape alone: names, `_`, and tuples of those. The compiler knows
    /// more (a struct pattern of names, a variant of a one-variant enum); see
    /// [`Resolved::always`].
    #[must_use]
    pub fn binds_only(&self) -> bool {
        match self {
            Self::Wildcard(_) | Self::Bind(..) => true,
            Self::Tuple(items, _) => items.iter().all(Self::binds_only),
            _ => false,
        }
    }
}

/// A pattern resolved against the type of the value it tests.
#[derive(Debug, Clone)]
pub enum Resolved {
    /// Matches anything.
    Any,
    /// Matches anything and binds it under this name, at this type.
    Bind(SmolStr, DataType),
    /// Equal to this value.
    Equal(Expr, Span),
    /// In this range; the flag says the high end is in it too.
    Range(Expr, Expr, bool, Span),
    /// A struct or a tuple: each field tested, by its index, holding a value
    /// of the type given.
    Fields(Vec<(u16, DataType, Self)>),
    /// One variant of an enum, then its payload, each place by its field
    /// index in the value.
    Variant(u16, u16, Vec<(u16, DataType, Self)>),
    /// Any of these.
    Or(Vec<Self>),
}

impl Resolved {
    /// The names the pattern binds with their types, sorted by name.
    pub fn bindings(&self) -> Vec<(SmolStr, DataType)> {
        let mut out = Vec::new();
        self.collect_bindings(&mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn collect_bindings(&self, out: &mut Vec<(SmolStr, DataType)>) {
        match self {
            Self::Bind(name, ty) => out.push((name.clone(), ty.clone())),
            Self::Fields(fields) | Self::Variant(_, _, fields) => {
                for (_, _, p) in fields {
                    p.collect_bindings(out);
                }
            }
            Self::Or(alternatives) => alternatives[0].collect_bindings(out),
            Self::Any | Self::Equal(..) | Self::Range(..) => {}
        }
    }
}

/// The base name of a type, `Cell` for the instantiation `Cell<int>`.
fn base_name(name: &str) -> &str {
    name.split('<').next().unwrap_or(name)
}

/// Resolves `pattern` against a value of type `ty`, reporting a pattern that
/// cannot fit it.
pub fn resolve(pattern: &Pattern, ty: &DataType, ctx: Ctx, state: &mut State<'_>) -> Resolved {
    match pattern {
        Pattern::Wildcard(_) => Resolved::Any,
        Pattern::Bind(name, _) => Resolved::Bind(name.clone(), ty.clone()),
        Pattern::Value(value, span) => Resolved::Equal((**value).clone(), *span),
        Pattern::Range(low, high, inclusive, span) => {
            Resolved::Range((**low).clone(), (**high).clone(), *inclusive, *span)
        }
        Pattern::Tuple(items, span) => {
            let types = match ty {
                DataType::Struct(id)
                    if state.structs[*id as usize].is_tuple()
                        && state.structs[*id as usize].fields.len() == items.len() =>
                {
                    state.structs[*id as usize]
                        .fields
                        .iter()
                        .map(|(_, t, _)| t.clone())
                        .collect::<Vec<_>>()
                }
                _ => compiler_errors::error_tuple_pattern_mismatch(
                    ctx.file_idx,
                    *span,
                    items.len(),
                    ty,
                    state.sources,
                    state.type_names(),
                ),
            };
            Resolved::Fields(
                items
                    .iter()
                    .zip(types)
                    .enumerate()
                    .map(|(i, (item, t))| {
                        let sub = resolve(item, &t, ctx, state);
                        (i as u16, t, sub)
                    })
                    .collect(),
            )
        }
        Pattern::Or(alternatives, _) => {
            let resolved: Vec<Resolved> = alternatives
                .iter()
                .map(|p| resolve(p, ty, ctx, state))
                .collect();
            let expected = resolved[0].bindings();
            for (k, alternative) in resolved.iter().enumerate().skip(1) {
                let found = alternative.bindings();
                if found != expected {
                    compiler_errors::error_pattern_bindings_differ(
                        &expected,
                        &found,
                        alternatives[k].span(),
                        ctx.file_idx,
                        state.sources,
                        state.type_names(),
                    );
                }
            }
            Resolved::Or(resolved)
        }
        Pattern::Named {
            path,
            type_args,
            payload,
            span,
        } => resolve_named(path, type_args, payload, *span, ty, ctx, state),
    }
}

/// The value a path written alone in a pattern compares with, a constant.
fn path_value(path: &[SmolStr], type_args: &[TypeExpr], span: Span) -> Expr {
    if path.len() == 1 {
        Expr::Var(path[0].clone(), span)
    } else {
        Expr::NamespacedRef(Box::from(path), span, Box::from(type_args))
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_named(
    path: &[SmolStr],
    type_args: &[TypeExpr],
    payload: &Payload,
    span: Span,
    ty: &DataType,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Resolved {
    match ty {
        DataType::Enum(enum_id) => {
            // A bare name that is no variant of the matched enum but names a
            // constant compares with it.
            if matches!(payload, Payload::None)
                && path.len() == 1
                && !state.enums[*enum_id as usize]
                    .variants
                    .iter()
                    .any(|variant| variant.name == path[0])
                && resolve_enum_variant(path, ctx.file_idx, state).is_none()
            {
                return Resolved::Equal(path_value(path, type_args, span), span);
            }
            let variant_idx = resolve_variant(*enum_id, path, type_args, payload, span, ctx, state);
            let payload_types = state.enums[*enum_id as usize].variants[variant_idx as usize]
                .payload
                .to_vec();
            let items: Vec<(u16, &Pattern)> = match payload {
                Payload::None => Vec::new(),
                Payload::Positional(items) => items
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i as u16, p))
                    .collect(),
                Payload::Fields(fields, rest) => {
                    let names = state
                        .generics
                        .variant_fields(*enum_id, variant_idx)
                        .map_or_else(Vec::new, <[SmolStr]>::to_vec);
                    order_fields(
                        &names,
                        fields,
                        *rest,
                        &path[path.len() - 1],
                        span,
                        "variant",
                        ctx,
                        state,
                    )
                }
            };
            if !matches!(payload, Payload::Fields(..)) && items.len() != payload_types.len() {
                compiler_errors::error_enum(
                    "Wrong variant payload arity",
                    &format!(
                        "Variant {} binds {} value(s) but the pattern has {}",
                        path[path.len() - 1],
                        payload_types.len(),
                        items.len()
                    ),
                    span,
                    ctx.file_idx,
                    state.sources,
                );
            }
            let fields = items
                .into_iter()
                .map(|(i, item)| {
                    let t = payload_types[i as usize].clone();
                    let sub = resolve(item, &t, ctx, state);
                    (i + 1, t, sub)
                })
                .collect();
            Resolved::Variant(*enum_id, variant_idx, fields)
        }
        DataType::Struct(struct_id) if matches!(payload, Payload::Fields(..)) => {
            let Payload::Fields(fields, rest) = payload else {
                unreachable!()
            };
            let s = &state.structs[*struct_id as usize];
            let struct_name = s.name.clone();
            if base_name(&struct_name) != path[path.len() - 1].as_str() {
                compiler_errors::error_type_test(
                    "Pattern of another type",
                    &format!(
                        "This pattern is a {}, but the value is a {struct_name}",
                        path.join("::")
                    ),
                    None,
                    "pattern_type_mismatch",
                    span,
                    ctx.file_idx,
                    state.sources,
                );
            }
            let names: Vec<SmolStr> = s.fields.iter().map(|(n, _, _)| n.clone()).collect();
            let types: Vec<DataType> = s.fields.iter().map(|(_, t, _)| t.clone()).collect();
            let items = order_fields(
                &names,
                fields,
                *rest,
                &struct_name,
                span,
                "struct",
                ctx,
                state,
            );
            Resolved::Fields(
                items
                    .into_iter()
                    .map(|(i, item)| {
                        let t = types[i as usize].clone();
                        let sub = resolve(item, &t, ctx, state);
                        (i, t, sub)
                    })
                    .collect(),
            )
        }
        _ => {
            // A variant pattern held against a value that is no enum.
            if let Some((enum_id, _)) = resolve_enum_variant(path, ctx.file_idx, state) {
                compiler_errors::error_match_not_enum(
                    &state.enums[enum_id as usize].name.clone(),
                    ty,
                    span,
                    ctx.file_idx,
                    state.sources,
                    state.type_names(),
                );
            }
            if !matches!(payload, Payload::None) {
                compiler_errors::error_type_test(
                    "Pattern of another type",
                    &format!(
                        "This pattern takes apart a {}, but the value is {}",
                        path.join("::"),
                        state.type_names().of(ty)
                    ),
                    None,
                    "pattern_type_mismatch",
                    span,
                    ctx.file_idx,
                    state.sources,
                );
            }
            Resolved::Equal(path_value(path, type_args, span), span)
        }
    }
}

/// The variant of enum `enum_id` a pattern's path names, checking that the
/// payload is written the way the variant takes it.
///
/// A qualified path resolves through its qualifier, the way a qualified
/// construction does, and has to name the matched enum; a bare name is looked
/// up among the matched enum's variants.
#[allow(clippy::too_many_arguments)]
fn resolve_variant(
    enum_id: u16,
    path: &[SmolStr],
    type_args: &[TypeExpr],
    payload: &Payload,
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> u16 {
    let variant_name = &path[path.len() - 1];
    match (variant_shape(path, ctx, state), payload) {
        (Some(VariantShape::Named(names)), Payload::Positional(_)) if !names.is_empty() => {
            compiler_errors::error_type_test(
                "Variant written in order",
                &format!("{variant_name} names its fields"),
                Some(&format!(
                    "Write the pattern with braces: {} {{ {} }}",
                    path.join("::"),
                    names.join(", ")
                )),
                "variant_payload_shape",
                span,
                ctx.file_idx,
                state.sources,
            );
        }
        (Some(VariantShape::Positional), Payload::Fields(..)) => {
            compiler_errors::error_type_test(
                "Variant written with braces",
                &format!("{variant_name} takes its payload in order, not by field name"),
                Some(&format!("Write it with parentheses: {variant_name}(...)")),
                "variant_payload_shape",
                span,
                ctx.file_idx,
                state.sources,
            );
        }
        _ => {}
    }
    if path.len() >= 2 {
        let (pattern_enum, variant_idx) = if type_args.is_empty() {
            resolve_qualified_pattern_enum(path, enum_id, span, ctx, state)
        } else {
            resolve_generic_variant(path, type_args, span, ctx, state)
        };
        if pattern_enum != enum_id {
            compiler_errors::error_pattern_enum_mismatch(
                &path.join("::"),
                &state.enums[pattern_enum as usize].name,
                &state.enums[enum_id as usize].name,
                span,
                ctx.file_idx,
                state.sources,
            );
        }
        return variant_idx;
    }
    let e = &state.enums[enum_id as usize];
    let Some(variant_idx) = e.variants.iter().position(|vt| &vt.name == variant_name) else {
        compiler_errors::error_enum(
            "Unknown enum variant",
            &format!("{} is not a variant of enum {}", variant_name, e.name),
            span,
            ctx.file_idx,
            state.sources,
        );
    };
    variant_idx as u16
}

/// The patterns a braced field list gives, each with the index of the field
/// it tests among `names`, the fields of the variant or struct `owner`. A list
/// that ends in `..` may leave fields out; any other names every field.
#[allow(clippy::too_many_arguments)]
fn order_fields<'p>(
    names: &[SmolStr],
    fields: &'p [(SmolStr, Pattern, Span)],
    rest: bool,
    owner: &str,
    span: Span,
    kind: &str,
    ctx: Ctx,
    state: &State<'_>,
) -> Vec<(u16, &'p Pattern)> {
    let mut out = Vec::with_capacity(fields.len());
    for (i, (field, pattern, field_span)) in fields.iter().enumerate() {
        let Some(index) = names.iter().position(|n| n == field) else {
            compiler_errors::error_type_test(
                if kind == "variant" {
                    "Unknown variant field"
                } else {
                    "Unknown struct field"
                },
                &format!("{owner} has no field {field}"),
                Some(&format!("Its fields are {}", names.join(", "))),
                if kind == "variant" {
                    "variant_field_unknown"
                } else {
                    "pattern_field_unknown"
                },
                *field_span,
                ctx.file_idx,
                state.sources,
            );
        };
        if fields[..i].iter().any(|(earlier, ..)| earlier == field) {
            compiler_errors::error_type_test(
                "Field written twice",
                &format!("{field} is already given"),
                None,
                if kind == "variant" {
                    "variant_field_twice"
                } else {
                    "pattern_field_twice"
                },
                *field_span,
                ctx.file_idx,
                state.sources,
            );
        }
        out.push((index as u16, pattern));
    }
    if !rest
        && let Some(missing) = names
            .iter()
            .find(|name| !fields.iter().any(|(f, ..)| f == *name))
    {
        compiler_errors::error_type_test(
            "Missing field in pattern",
            &format!("The pattern does not name {owner}'s field {missing}"),
            Some("Name every field, or end the pattern with .. to skip the rest"),
            if kind == "variant" {
                "variant_field_missing"
            } else {
                "pattern_field_missing"
            },
            span,
            ctx.file_idx,
            state.sources,
        );
    }
    out
}

/// What compiling a pattern left: the jumps taken when the value does not
/// match, which the caller aims at what runs then, and the variables it
/// binds, to bring into scope where it matched.
#[derive(Default)]
pub struct Compiled {
    pub fails: Vec<usize>,
    pub binds: Vec<Variable>,
}

/// Compiles the tests of `resolved` against the value in `reg` and the reads
/// of what it binds.
///
/// `owned` says `reg` is a register the pattern may hand to a binding as it
/// is; otherwise a binding takes a copy, so writing it leaves the matched
/// value alone. A name in `captured` is bound in a cell, since a closure in
/// the code that sees it takes it. `targets` are the registers an earlier
/// alternative of an `|` bound each name in, which this one fills too.
#[allow(clippy::too_many_arguments)]
pub fn compile(
    resolved: &Resolved,
    reg: u16,
    ty: &DataType,
    owned: bool,
    captured: &[SmolStr],
    targets: &[Variable],
    out: &mut Compiled,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    match resolved {
        Resolved::Any => {}
        Resolved::Bind(name, ty) => {
            if let Some(target) = targets.iter().find(|t| &t.name == name) {
                if target.cell {
                    output.push(Instr::NewCell(reg, target.register_id));
                } else if target.register_id != reg {
                    output.push(Instr::Mov(reg, target.register_id));
                }
                out.binds.push(target.clone());
                return;
            }
            let cell = captured.contains(name);
            let register_id = if cell {
                let cell_reg = state.alloc_reg();
                output.push(Instr::NewCell(reg, cell_reg));
                cell_reg
            } else if owned {
                reg
            } else {
                let copy = state.alloc_reg();
                output.push(Instr::Mov(reg, copy));
                copy
            };
            out.binds.push(Variable {
                declared: None,
                name: name.clone(),
                register_id,
                cell,
                var_type: ty.clone(),
            });
        }
        Resolved::Equal(value, span) => {
            let test = |held: Expr| {
                Expr::Eq(
                    Operand::new(held),
                    Operand::new(value.clone()),
                    *span,
                    *span,
                )
            };
            out.fails
                .extend(test_held(reg, ty, test, *span, v, ctx, state, output));
        }
        Resolved::Range(low, high, inclusive, span) => {
            let test = |held: Expr| {
                let above = Expr::SupEq(
                    Operand::new(held.clone()),
                    Operand::new(low.clone()),
                    *span,
                    *span,
                );
                let below = if *inclusive {
                    Expr::InfEq(Operand::new(held), Operand::new(high.clone()), *span, *span)
                } else {
                    Expr::Inf(Operand::new(held), Operand::new(high.clone()), *span, *span)
                };
                Expr::BoolAnd(Operand::new(above), Operand::new(below), *span, *span)
            };
            out.fails
                .extend(test_held(reg, ty, test, *span, v, ctx, state, output));
        }
        Resolved::Fields(fields) => {
            compile_fields(fields, reg, captured, targets, out, v, ctx, state, output);
        }
        Resolved::Variant(enum_id, variant_idx, fields) => {
            // A one-variant enum needs no test of its tag.
            if state.enums[*enum_id as usize].variants.len() > 1 {
                let tag = state.alloc_reg();
                output.push(Instr::GetFieldStruct(reg, 0, tag));
                let idx = state.alloc_reg();
                output.push(Instr::SetInt(idx, i32::from(*variant_idx)));
                out.fails.push(output.len());
                output.push(Instr::NotEqJmp(tag, idx, 0));
                state.free_reg(tag, v);
                state.free_reg(idx, v);
            }
            compile_fields(fields, reg, captured, targets, out, v, ctx, state, output);
        }
        Resolved::Or(alternatives) => {
            let mut matched: Vec<usize> = Vec::new();
            let mut shared: Vec<Variable> = targets.to_vec();
            let (last, others) = alternatives.split_last().unwrap();
            for alternative in others {
                let mut inner = Compiled::default();
                compile(
                    alternative,
                    reg,
                    ty,
                    false,
                    captured,
                    &shared,
                    &mut inner,
                    v,
                    ctx,
                    state,
                    output,
                );
                for bound in &inner.binds {
                    if !shared.iter().any(|s| s.name == bound.name) {
                        shared.push(bound.clone());
                    }
                }
                matched.push(output.len());
                output.push(Instr::Jmp(0));
                let next = output.len();
                for j in inner.fails {
                    set_jmp_size(&mut output[j], (next - j) as u16);
                }
            }
            let mut inner = Compiled::default();
            compile(
                last, reg, ty, false, captured, &shared, &mut inner, v, ctx, state, output,
            );
            for bound in inner.binds {
                if !shared.iter().any(|s| s.name == bound.name) {
                    shared.push(bound);
                }
            }
            out.fails.extend(inner.fails);
            let end = output.len();
            for j in matched {
                set_jmp_size(&mut output[j], (end - j) as u16);
            }
            for bound in shared {
                if !targets.iter().any(|t| t.name == bound.name) {
                    out.binds.push(bound);
                }
            }
        }
    }
}

/// Reads each field a struct, tuple or variant pattern tests out of the value
/// in `reg` and compiles its pattern against it.
#[allow(clippy::too_many_arguments)]
fn compile_fields(
    fields: &[(u16, DataType, Resolved)],
    reg: u16,
    captured: &[SmolStr],
    targets: &[Variable],
    out: &mut Compiled,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    for (field, field_type, sub) in fields {
        if matches!(sub, Resolved::Any) {
            continue;
        }
        let value = state.alloc_reg();
        output.push(Instr::GetFieldStruct(reg, *field, value));
        // The field's register stays the pattern's while something it binds
        // or tests reads it.
        let v_len = v.len();
        v.push(Variable {
            declared: None,
            name: SmolStr::new_static("[PATTERN FIELD]"),
            register_id: value,
            cell: false,
            var_type: field_type.clone(),
        });
        let binds_before = out.binds.len();
        compile(
            sub, value, field_type, true, captured, targets, out, v, ctx, state, output,
        );
        v.truncate(v_len);
        if !out.binds[binds_before..]
            .iter()
            .any(|bound| bound.register_id == value)
        {
            state.free_reg(value, v);
        }
    }
}

/// Compiles the test `make` builds around the value held in `reg`, typed by
/// what reads it, and returns the jumps it takes when the test is false.
#[allow(clippy::too_many_arguments)]
fn test_held(
    reg: u16,
    held_type: &DataType,
    make: impl Fn(Expr) -> Expr,
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> Vec<usize> {
    let name = SmolStr::new_static("[PATTERN VALUE]");
    let v_len = v.len();
    v.push(Variable {
        declared: None,
        name: name.clone(),
        register_id: reg,
        cell: false,
        var_type: held_type.clone(),
    });
    let test = make(Expr::Var(name, span));
    let fails = compile_guard(&test, span, &[], v, ctx, state, output);
    v.truncate(v_len);
    fails
}

/// The names in `pattern` a closure in `scope` takes, which are bound in
/// cells.
#[must_use]
pub fn captured_in(pattern: &Pattern, scope: &[&[Expr]]) -> Vec<SmolStr> {
    pattern
        .binders()
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| scope.iter().any(|code| code_captures_variable(name, code)))
        .collect()
}

/// Declares, for a walk that types code without compiling it, what `pattern`
/// binds when it holds against a value of type `ty`.
pub fn declare_bindings(
    pattern: &Pattern,
    ty: &DataType,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) {
    for (name, var_type) in resolve(pattern, ty, ctx, state).bindings() {
        v.push(Variable {
            declared: None,
            name,
            register_id: 0,
            cell: false,
            var_type,
        });
    }
}

/// Resolves the pattern of each arm of a `match` against the scrutinee's type.
pub fn resolve_arms(
    arms: &[super::expr::MatchArm],
    ty: &DataType,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<Resolved> {
    arms.iter()
        .map(|arm| resolve(&arm.pattern, ty, ctx, state))
        .collect()
}

/// Whether a `match` has an arm for every value of `ty`: a wildcard, or arms
/// with no guard whose patterns between them fit every value.
pub fn arms_cover(
    ty: &DataType,
    arms: &[super::expr::MatchArm],
    resolved: &[Resolved],
    has_wildcard: bool,
    state: &State<'_>,
) -> bool {
    has_wildcard
        || rows_cover(
            unguarded_rows(arms, resolved),
            std::slice::from_ref(ty),
            state,
        )
}

/// Whether `resolved` fits every value of type `ty`.
pub fn always_fits(resolved: &Resolved, ty: &DataType, state: &State<'_>) -> bool {
    rows_cover(vec![vec![Some(resolved)]], std::slice::from_ref(ty), state)
}

/// The variants of enum `enum_id` some value of which no arm with no guard
/// fits, by index.
pub fn missing_variants(
    enum_id: u16,
    arms: &[super::expr::MatchArm],
    resolved: &[Resolved],
    state: &State<'_>,
) -> Vec<u16> {
    let rows = expand_first(unguarded_rows(arms, resolved));
    (0..state.enums[enum_id as usize].variants.len() as u16)
        .filter(|&variant| {
            let (spec, types) = specialize_variant(&rows, enum_id, variant, &[], state);
            !rows_cover(spec, &types, state)
        })
        .collect()
}

/// One row per arm with no guard, holding its pattern.
fn unguarded_rows<'a>(
    arms: &[super::expr::MatchArm],
    resolved: &'a [Resolved],
) -> Vec<Vec<Option<&'a Resolved>>> {
    arms.iter()
        .zip(resolved)
        .filter(|(arm, _)| arm.guard.is_none())
        .map(|(_, resolved)| vec![Some(resolved)])
        .collect()
}

/// A row of patterns, one per value of a sequence; `None` fits anything.
type Row<'a> = Vec<Option<&'a Resolved>>;

/// Whether `rows`, each a pattern per value of a sequence typed `types`,
/// between them fit every such sequence. Each column is split by what its
/// type can be: every variant of an enum, the one shape of a struct or a
/// tuple, `true` and `false` for a `bool`; any other type has more values
/// than a pattern of literals and ranges can list, so only a pattern that
/// fits anything covers it.
fn rows_cover(rows: Vec<Row<'_>>, types: &[DataType], state: &State<'_>) -> bool {
    if rows.is_empty() {
        return false;
    }
    let Some((first, rest)) = types.split_first() else {
        return true;
    };
    let rows = expand_first(rows);
    let wild = |p: Option<&Resolved>| matches!(p, None | Some(Resolved::Any | Resolved::Bind(..)));
    if rows.iter().all(|row| wild(row[0])) {
        return rows_cover(
            rows.into_iter().map(|row| row[1..].to_vec()).collect(),
            rest,
            state,
        );
    }
    match first {
        DataType::Enum(enum_id) => {
            (0..state.enums[*enum_id as usize].variants.len() as u16).all(|variant| {
                let (spec, types) = specialize_variant(&rows, *enum_id, variant, rest, state);
                rows_cover(spec, &types, state)
            })
        }
        DataType::Struct(struct_id) => {
            let fields: Vec<DataType> = state.structs[*struct_id as usize]
                .fields
                .iter()
                .map(|(_, t, _)| t.clone())
                .collect();
            let spec = rows
                .iter()
                .filter_map(|row| {
                    let expanded = match row[0] {
                        None | Some(Resolved::Any | Resolved::Bind(..)) => vec![None; fields.len()],
                        Some(Resolved::Fields(given)) => spread(given, fields.len(), 0),
                        Some(_) => return None,
                    };
                    Some(
                        expanded
                            .into_iter()
                            .chain(row[1..].iter().copied())
                            .collect(),
                    )
                })
                .collect();
            let types: Vec<DataType> = fields.into_iter().chain(rest.iter().cloned()).collect();
            rows_cover(spec, &types, state)
        }
        DataType::Bool => [true, false].iter().all(|b| {
            let spec = rows
                .iter()
                .filter(|row| {
                    wild(row[0])
                        || matches!(row[0], Some(Resolved::Equal(Expr::Bool(v), _)) if v == b)
                })
                .map(|row| row[1..].to_vec())
                .collect();
            rows_cover(spec, rest, state)
        }),
        _ => rows_cover(
            rows.iter()
                .filter(|row| wild(row[0]))
                .map(|row| row[1..].to_vec())
                .collect(),
            rest,
            state,
        ),
    }
}

/// The rows with each `|` in their first place spread into one row per
/// alternative.
fn expand_first(rows: Vec<Row<'_>>) -> Vec<Row<'_>> {
    let mut out = Vec::with_capacity(rows.len());
    let mut pending = rows;
    while let Some(row) = pending.pop() {
        if let Some(Resolved::Or(alternatives)) = row[0] {
            for alternative in alternatives.iter().rev() {
                let mut split = row.clone();
                split[0] = Some(alternative);
                pending.push(split);
            }
        } else {
            out.push(row);
        }
    }
    out.reverse();
    out
}

/// The rows that fit variant `variant` of enum `enum_id`, each with its first
/// place replaced by the variant's payload, and the types of the new row.
fn specialize_variant<'a>(
    rows: &[Row<'a>],
    enum_id: u16,
    variant: u16,
    rest: &[DataType],
    state: &State<'_>,
) -> (Vec<Row<'a>>, Vec<DataType>) {
    let payload = &state.enums[enum_id as usize].variants[variant as usize].payload;
    let spec = rows
        .iter()
        .filter_map(|row| {
            let expanded = match row[0] {
                None | Some(Resolved::Any | Resolved::Bind(..)) => vec![None; payload.len()],
                Some(Resolved::Variant(_, v, given)) if *v == variant => {
                    spread(given, payload.len(), 1)
                }
                Some(_) => return None,
            };
            Some(
                expanded
                    .into_iter()
                    .chain(row[1..].iter().copied())
                    .collect(),
            )
        })
        .collect();
    let types = payload
        .iter()
        .cloned()
        .chain(rest.iter().cloned())
        .collect();
    (spec, types)
}

/// The patterns `given` places by field index, laid out over `width` places;
/// a place it does not name fits anything. A variant's fields count from 1,
/// past its tag, so `first` is the index of place 0.
fn spread(given: &[(u16, DataType, Resolved)], width: usize, first: u16) -> Row<'_> {
    let mut row = vec![None; width];
    for (field, _, pattern) in given {
        if let Some(place) = row.get_mut((field - first) as usize) {
            *place = Some(pattern);
        }
    }
    row
}
