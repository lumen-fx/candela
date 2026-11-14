//! Implicit conversion between `int` and `float`.
//!
//! A value of one number type goes where the other is expected: a parameter,
//! a declared variable, a struct field, a return value, an element of a list or
//! a map whose type is declared. An `int` becomes a `float` exactly. A `float`
//! becomes an `int` the way `int(x)` makes it, truncated toward zero, and the
//! compiler warns about each place that happens, since it loses the fraction.
//!
//! The conversion is settled at compile time from the static types: the
//! checker accepts the pair, and the code generator writes one conversion
//! instruction at that site. A value whose type is already right, or one the
//! checker cannot pin down (`any`), passes through untouched, so code that
//! never mixes the two types runs exactly as before.

use super::UnwrapId;
use super::compiler_data::Ctx;
use super::compiler_data::State;
use super::compiler_data::Variable;
use super::expr::Expr;
use super::expr::Span;
use super::type_system::DataType;
use crate::Diagnostic;
use crate::instr::Instr;
use crate::instr::LibFunc;
use rustc_hash::FxHashSet;
use std::cell::RefCell;

thread_local! {
    /// The places a float-to-int warning was raised for during this compile, as
    /// `(file, start, end)`. A generic body compiles once per instantiation and
    /// a body can be compiled again when a call sees new argument types, so
    /// the same site is reached more than once; it is reported once.
    static WARNED: RefCell<FxHashSet<(u16, u32, u32)>> = RefCell::new(FxHashSet::default());
}

/// Forgets the sites warned about, at the start of a compile.
pub fn reset() {
    WARNED.with(|w| w.borrow_mut().clear());
}

/// Which way a number is converted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Numeric {
    /// An `int` where a `float` is expected. Exact.
    ToFloat,
    /// A `float` where an `int` is expected, truncated toward zero.
    ToInt,
}

impl Numeric {
    const fn lib_func(self) -> LibFunc {
        match self {
            Self::ToFloat => LibFunc::Float,
            Self::ToInt => LibFunc::Int,
        }
    }

    /// The type the value has once converted.
    #[must_use]
    pub const fn target(self) -> DataType {
        match self {
            Self::ToFloat => DataType::Float,
            Self::ToInt => DataType::Int,
        }
    }
}

/// The conversion that takes a value of type `found` into a slot declared
/// `expected`, if one does.
///
/// Only the two number types convert, and only as whole values: an `int[]` is
/// not a `float[]`, since the list is shared and converting it would mean
/// copying it. A slot that admits several types converts a number only when it
/// admits the other number type and not this one, so `float?` takes an `int`
/// and `int | float` takes either as it is.
#[must_use]
pub fn numeric(expected: &DataType, found: &DataType) -> Option<Numeric> {
    match (expected, found) {
        (DataType::Float, DataType::Int) => Some(Numeric::ToFloat),
        (DataType::Int, DataType::Float) => Some(Numeric::ToInt),
        (DataType::Union(members), DataType::Int)
            if members.contains(&DataType::Float) && !members.contains(&DataType::Int) =>
        {
            Some(Numeric::ToFloat)
        }
        (DataType::Union(members), DataType::Float)
            if members.contains(&DataType::Int) && !members.contains(&DataType::Float) =>
        {
            Some(Numeric::ToInt)
        }
        _ => None,
    }
}

/// The type an arithmetic or comparison operator works in for operands of
/// types `l` and `r` when they are one `int` and one `float`: `float`. `None`
/// for every other pair, which keeps the operator's own rules.
#[must_use]
pub const fn mixed_numbers(l: &DataType, r: &DataType) -> bool {
    matches!(
        (l, r),
        (DataType::Int, DataType::Float) | (DataType::Float, DataType::Int)
    )
}

/// The element type of a list literal whose elements are numbers of both
/// types: `float`, which every `int` among them converts to. `None` when the
/// elements are not such a mix.
#[must_use]
pub fn merged_number_type(types: &[DataType]) -> Option<DataType> {
    let mut float = false;
    let mut int = false;
    for t in types {
        match t {
            DataType::Float => float = true,
            DataType::Int => int = true,
            DataType::Unknown => {}
            _ => return None,
        }
    }
    (float && int).then_some(DataType::Float)
}

/// Whether the literal `value` reaches a slot declared `expected` once the
/// numbers inside it are converted: a scalar that [`numeric`] converts, a list
/// literal whose elements each fit the declared element type that way, or a
/// map literal whose values do. Literals nest, so `[[1, 2]]` fits `float[][]`.
///
/// Only a literal is looked into: nothing else holds it yet, so its elements
/// can be built at the declared type. A list held in a variable keeps its own.
pub fn literal_fits(
    expected: &DataType,
    value: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> bool {
    match (expected, value) {
        (DataType::Array(Some(element)), Expr::Array(items, _)) => items
            .iter()
            .all(|item| element_fits(element, item, v, ctx, state)),
        (DataType::Map(slots), Expr::Map(entries, _)) => {
            let (key, value) = &**slots;
            entries.iter().all(|(k, _, val, _)| {
                key.as_ref().is_none_or(|key| {
                    let found = k.infer_type(v, ctx, state);
                    super::type_system::param_type_matches(key, &found, state.generics)
                }) && value
                    .as_ref()
                    .is_none_or(|value| element_fits(value, val, v, ctx, state))
            })
        }
        (DataType::Enum(_), Expr::FunctionCall(..)) => {
            retargeted_variant(expected, value, v, ctx, state).is_some()
        }
        (DataType::Struct(_), Expr::Struct(..)) => {
            retargeted_struct(expected, value, v, ctx, state).is_some()
        }
        _ => {
            let found = value.infer_type(v, ctx, state);
            numeric(expected, &found).is_some()
        }
    }
}

/// Whether one element of a literal fits the element type it is declared with,
/// as it is or converted.
fn element_fits(
    expected: &DataType,
    item: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> bool {
    let found = item.infer_type(v, ctx, state);
    super::type_system::param_type_matches(expected, &found, state.generics)
        || literal_fits(expected, item, v, ctx, state)
}

/// Compiles `value` for a slot declared `expected` when getting it there takes
/// a conversion, and answers the register the converted value is in. `None`
/// when the value needs none; the caller compiles it as it always has.
///
/// `span` is where the value is written, which is where a float-to-int
/// conversion is reported.
#[allow(clippy::too_many_arguments)]
pub fn compile_converted(
    value: &Expr,
    expected: &DataType,
    span: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> Option<u16> {
    if let Some((enum_id, variant_idx)) = retargeted_variant(expected, value, v, ctx, state)
        && let Expr::FunctionCall(args, _, call_span, args_indexes, _) = value
    {
        return Some(super::compile_enum_construction(
            enum_id,
            variant_idx,
            args,
            *call_span,
            args_indexes,
            v,
            ctx,
            state,
            output,
        ));
    }
    if let Some(struct_id) = retargeted_struct(expected, value, v, ctx, state)
        && let Expr::Struct(_, fields, literal_span, _, base) = value
    {
        return Some(super::compile_struct_literal_at(
            struct_id as usize,
            fields,
            base.as_deref(),
            *literal_span,
            v,
            ctx,
            state,
            output,
        ));
    }
    if !matches!(
        expected,
        DataType::Int | DataType::Float | DataType::Union(_)
    ) {
        return None;
    }
    let found = value.infer_type(v, ctx, state);
    let conversion = numeric(expected, &found)?;
    Some(compile_conversion(
        value, conversion, span, tgt_id, v, ctx, state, output,
    ))
}

/// Compiles `value` and converts it as `conversion` says, into `tgt_id` when
/// one is given. A number literal converts while compiling, so `f(3)` passes
/// the constant `3.0` and costs nothing when the program runs.
#[allow(clippy::too_many_arguments)]
pub fn compile_conversion(
    value: &Expr,
    conversion: Numeric,
    span: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    if conversion == Numeric::ToInt {
        warn_float_to_int(span, ctx, state);
    }
    let constant = match (conversion, value) {
        #[allow(clippy::cast_precision_loss)]
        (Numeric::ToFloat, Expr::Int(n)) => Some(Expr::Float(*n as f64)),
        #[allow(clippy::cast_possible_truncation)]
        (Numeric::ToInt, Expr::Float(f)) => Some(Expr::Int(*f as i64)),
        _ => None,
    };
    if let Some(constant) = constant {
        return constant
            .compile(v, ctx, state, output, tgt_id, false, true)
            .unwrap_id();
    }
    let id = value
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(id, v);
    let output_id = state.alloc_reg_tgt(tgt_id);
    output.push(Instr::CallLibFunc(conversion.lib_func(), id, output_id));
    output_id
}

/// Reports, as a warning, that the float written at `span` becomes an int
/// there. Standard library code is not reported on: its conversions are the
/// library's own business, not the program's.
pub fn warn_float_to_int(span: Span, ctx: Ctx, state: &State<'_>) {
    if state.namespaces.is_std(ctx.file_idx) {
        return;
    }
    let fresh = WARNED.with(|w| w.borrow_mut().insert((ctx.file_idx, span.start, span.end)));
    if !fresh {
        return;
    }
    let Some(source) = state.sources.get(ctx.file_idx as usize) else {
        return;
    };
    crate::warnings::emit_warning(
        state.sources,
        "Float converted to int",
        Diagnostic {
            filename: source.filename.to_string(),
            span: (span.start as usize)..(span.end as usize),
            message: String::from(
                "This float becomes an int here (truncated toward zero). Write int(x) to make that explicit",
            ),
            code: String::from("implicit_float_to_int"),
        },
    );
}

/// Compiles the two operands of a number operator, `l` typed `t_l` and `r`
/// typed `t_r`, and answers their registers. When one is an `int` and the other
/// a `float`, the `int` is converted, so the operator works on two floats. The
/// left operand is worked out first either way.
#[allow(clippy::too_many_arguments)]
pub fn compile_number_operands(
    l: &Expr,
    r: &Expr,
    t_l: &DataType,
    t_r: &DataType,
    span_l: Span,
    span_r: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> (u16, u16) {
    let mixed = mixed_numbers(t_l, t_r);
    let mut operand =
        |e: &Expr, t: &DataType, span: Span, v: &mut Vec<Variable>, state: &mut State<'_>| {
            if mixed && *t == DataType::Int {
                compile_conversion(e, Numeric::ToFloat, span, None, v, ctx, state, output)
            } else {
                e.compile(v, ctx, state, output, None, false, true)
                    .unwrap_id()
            }
        };
    let id_l = operand(l, t_l, span_l, v, state);
    let id_r = operand(r, t_r, span_r, v, state);
    (id_l, id_r)
}

/// Whether `found` is `expected` but for `int` and `float` standing in for
/// each other somewhere inside it: `int` for `float`, `int[]` for `float[]`, a
/// union of the two for either. A function whose body returns such a type
/// may be declared with the other one; each `return` then converts what it
/// can, and reports the value it cannot (a list held elsewhere, say).
#[must_use]
pub fn differs_only_in_numbers(expected: &DataType, found: &DataType) -> bool {
    let number = |t: &DataType| matches!(t, DataType::Int | DataType::Float);
    match (expected, found) {
        (e, f) if number(e) && number(f) => true,
        (_, DataType::Union(members)) => members
            .iter()
            .all(|member| expected == member || differs_only_in_numbers(expected, member)),
        (DataType::Array(Some(e)), DataType::Array(Some(f))) => {
            e == f || differs_only_in_numbers(e, f)
        }
        (DataType::Map(e), DataType::Map(f)) => {
            let slot = |e: &Option<DataType>, f: &Option<DataType>| match (e, f) {
                (Some(e), Some(f)) => e == f || differs_only_in_numbers(e, f),
                _ => true,
            };
            slot(&e.0, &f.0) && slot(&e.1, &f.1)
        }
        _ => false,
    }
}

/// The span `expr` was written at, for the forms that carry one. A `return`
/// carries no span of its own, so its value's is what a warning about it
/// points at.
#[must_use]
pub fn span_of(expr: &Expr) -> Option<Span> {
    Some(match expr {
        Expr::Var(_, span)
        | Expr::Map(_, span)
        | Expr::Struct(_, _, span, _, _)
        | Expr::FunctionCall(_, _, span, _, _)
        | Expr::ArrayGetIndex(_, _, span)
        | Expr::Propagate(_, span)
        | Expr::ArrayGetSlice(_, _, _, span) => *span,
        Expr::Array(_, spans) => *spans.first()?,
        Expr::ObjFunctionCall(_, _, _, obj_span, fn_span, _, _) => obj_span.extend(*fn_span),
        Expr::GetStructField(_, _, l, r)
        | Expr::Cast(_, _, l, r)
        | Expr::Neg(_, l, r)
        | Expr::BitNot(_, l, r)
        | Expr::Mul(_, _, l, r)
        | Expr::Div(_, _, l, r)
        | Expr::Add(_, _, l, r)
        | Expr::Sub(_, _, l, r)
        | Expr::Mod(_, _, l, r)
        | Expr::Pow(_, _, l, r) => l.extend(*r),
        _ => return None,
    })
}

/// The variant to build when `value` constructs a generic enum at other type
/// arguments than the slot `expected` declares, and converting numbers makes
/// it fit: `Some(1)` where an `Option<float>` is expected is built as an
/// `Option<float>` holding `1.0`. Answers the declared enum and the variant's
/// index in it.
fn retargeted_variant(
    expected: &DataType,
    value: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<(u16, u16)> {
    let DataType::Enum(enum_id) = expected else {
        return None;
    };
    let Expr::FunctionCall(args, path, _, _, type_args) = value else {
        return None;
    };
    if !type_args.is_empty() {
        return None;
    }
    let (template, _) = state.generics.instantiation_of(expected)?;
    let found = value.infer_type(v, ctx, state);
    if found == *expected {
        return None;
    }
    let (found_template, _) = state.generics.instantiation_of(&found)?;
    if found_template != template {
        return None;
    }
    let variant = path.last()?;
    let variants = &state.enums[*enum_id as usize].variants;
    let variant_idx = variants.iter().position(|vt| &vt.name == variant)?;
    let payload = variants[variant_idx].payload.clone();
    if payload.len() != args.len() {
        return None;
    }
    args.iter()
        .zip(payload.iter())
        .all(|(arg, declared)| element_fits(declared, arg, v, ctx, state))
        .then_some((*enum_id, variant_idx as u16))
}

/// The struct to build when the literal `value` names a generic struct at
/// other type arguments than the slot `expected` declares, and converting
/// numbers makes it fit: `Cell { value: 3 }` where a `Cell<float>` is expected
/// is built as a `Cell<float>`. A literal that writes its type arguments keeps
/// them.
fn retargeted_struct(
    expected: &DataType,
    value: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<u16> {
    let DataType::Struct(struct_id) = expected else {
        return None;
    };
    let Expr::Struct(_, fields, _, type_args, _) = value else {
        return None;
    };
    if !type_args.is_empty() {
        return None;
    }
    let (template, _) = state.generics.instantiation_of(expected)?;
    let found = value.infer_type(v, ctx, state);
    if found == *expected {
        return None;
    }
    let (found_template, _) = state.generics.instantiation_of(&found)?;
    if found_template != template {
        return None;
    }
    let declared = state.structs[*struct_id as usize].fields.clone();
    fields
        .iter()
        .all(|(name, field_value, _, _)| {
            declared
                .iter()
                .find(|(declared_name, _, _)| declared_name == name)
                .is_some_and(|(_, field_type, _)| {
                    element_fits(field_type, field_value, v, ctx, state)
                })
        })
        .then_some(*struct_id)
}
