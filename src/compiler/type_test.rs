//! `is` and `as`: the type test and the checked downcast.
//!
//! Both settle what they can from the operand's static type. A test the type
//! already answers costs nothing while the program runs: `is` answers it as a
//! constant, and `as` hands the value on, or reports a downcast the type rules
//! out. What is left compiles to the instruction the value's type needs: one
//! tag test for a scalar, and a list of type codes for a struct, an enum, a
//! function, a union, or a collection whose contents are checked.
//!
//! Where an `is` is known to hold, it says more than a bool: a variable it
//! tested has the type it was tested for, and a variant test binds the
//! payload. The code that compiles a condition decides where that is (the
//! body of an `if` or a `while`, and the right of an `&&`), and asks this
//! module for those [`Facts`].

use super::UnwrapId;
use super::compiler_data::Ctx;
use super::compiler_data::State;
use super::compiler_data::Variable;
use super::compiler_errors;
use super::expr::Expr;
use super::expr::IsTarget;
use super::expr::Span;
use super::type_system::DataType;
use super::type_system::Generics;
use super::type_system::TypeExpr;
use super::type_system::find_template;
use super::type_system::instantiations_line_up;
use crate::data::Data;
use crate::instr::Instr;
use crate::instr::LibFunc;
use crate::instr::type_code;
use smol_strc::SmolStr;

/// What a value's static type says about a test for another type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// Every value of the static type passes.
    Always,
    /// No value of the static type passes.
    Never,
    /// Some do and some do not, so the value is looked at while the program
    /// runs.
    AtRunTime,
}

/// Whether every value of `have` is a `want`, none is, or the program has to
/// look.
///
/// `any` is what every value is, so a test for it always passes and a value
/// typed `any` is always looked at. A list or a map passes on what it holds,
/// position by position, and a union on its members. Two instantiations of one
/// generic type whose arguments line up count as the same type, the way a
/// parameter accepts one for the other. A function passes a test for any
/// function type: what it was compiled to accept is not something a value
/// carries.
#[must_use]
pub fn settle(have: &DataType, want: &DataType, generics: &Generics) -> Settled {
    if *want == DataType::Unknown {
        return Settled::Always;
    }
    match (have, want) {
        (DataType::Unknown, _) => Settled::AtRunTime,
        // A union compares equal to each of its members, so it is taken apart
        // before the equality test, which would pass it for any one of them.
        (DataType::Union(members), _) => {
            all_of(members.iter().map(|member| settle(member, want, generics)))
        }
        _ if have == want => Settled::Always,
        (_, DataType::Union(members)) => {
            let each: Vec<Settled> = members
                .iter()
                .map(|member| settle(have, member, generics))
                .collect();
            if each.contains(&Settled::Always) {
                Settled::Always
            } else if each.iter().all(|s| *s == Settled::Never) {
                Settled::Never
            } else {
                Settled::AtRunTime
            }
        }
        (DataType::Array(have), DataType::Array(want)) => {
            held(have.as_deref(), want.as_deref(), generics)
        }
        (DataType::Map(have), DataType::Map(want)) => both(
            held(have.0.as_ref(), want.0.as_ref(), generics),
            held(have.1.as_ref(), want.1.as_ref(), generics),
        ),
        (DataType::Fn(_) | DataType::FnValue(_), DataType::FnValue(_)) => Settled::Always,
        (DataType::Enum(_), DataType::Enum(_)) if instantiations_line_up(have, want, generics) => {
            Settled::Always
        }
        _ => Settled::Never,
    }
}

/// One position of a collection: what it holds against what is wanted there.
/// A position left open on the wanted side takes anything; one left open on
/// the static side (an empty literal nothing pinned) is looked at.
fn held(have: Option<&DataType>, want: Option<&DataType>, generics: &Generics) -> Settled {
    match (have, want) {
        (_, None | Some(DataType::Unknown)) => Settled::Always,
        (None, Some(_)) => Settled::AtRunTime,
        (Some(have), Some(want)) => settle(have, want, generics),
    }
}

/// Every member has to pass for the whole to.
fn all_of(each: impl Iterator<Item = Settled>) -> Settled {
    let each: Vec<Settled> = each.collect();
    if each.iter().all(|s| *s == Settled::Always) {
        Settled::Always
    } else if each.iter().all(|s| *s == Settled::Never) {
        Settled::Never
    } else {
        Settled::AtRunTime
    }
}

/// Two positions that both have to pass.
const fn both(a: Settled, b: Settled) -> Settled {
    match (a, b) {
        (Settled::Never, _) | (_, Settled::Never) => Settled::Never,
        (Settled::Always, Settled::Always) => Settled::Always,
        _ => Settled::AtRunTime,
    }
}

/// What the right side of an `is` names once resolved.
pub enum Test {
    /// A type test.
    Type(DataType),
    /// A variant test on a value of the enum `enum_id`, with one binder per
    /// payload position (`_` for one that binds nothing).
    Variant {
        enum_id: u16,
        variant_idx: u16,
        binders: Vec<SmolStr>,
    },
}

/// Resolves the right side of `operand is target`, `operand_type` being the
/// operand's static type.
///
/// A bare name is a type where one by that name is in scope. It is a variant
/// where the operand is an enum that has a variant by that name, and where no
/// type has it but some enum's variant does: `opt is None` reads the way the
/// `match` arm `None` does.
pub fn resolve_test(
    operand_type: &DataType,
    target: &IsTarget,
    operand_span: Span,
    target_span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Test {
    let pattern = match target {
        IsTarget::Variant(pattern) => pattern.clone(),
        IsTarget::Type(TypeExpr::Identifier(name, span))
            if names_variant(operand_type, std::slice::from_ref(name), *span, ctx, state) =>
        {
            Expr::Var(name.clone(), *span)
        }
        IsTarget::Type(TypeExpr::NamespacedIdentifier(path, span))
            if names_variant(operand_type, path, *span, ctx, state) =>
        {
            Expr::NamespacedRef(path.clone(), *span, Box::from([]))
        }
        IsTarget::Type(t) => return Test::Type(resolve_type(t, target_span, ctx, state)),
    };
    let DataType::Enum(enum_id) = *operand_type else {
        error_variant_test_not_enum(operand_type, operand_span, target_span, ctx, state);
    };
    let (variant_idx, binders) =
        super::resolve_variant_pattern(enum_id, &pattern, target_span, ctx, state);
    Test::Variant {
        enum_id,
        variant_idx,
        binders,
    }
}

/// Whether `path`, written where a type goes after `is`, names a variant
/// rather than a type.
fn names_variant(
    operand_type: &DataType,
    path: &[SmolStr],
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> bool {
    if super::resolve_enum_variant(path, ctx.file_idx, state).is_none() {
        return false;
    }
    // The operand's own enum has a variant by this name: that is what the
    // test asks about, whatever else the name could be.
    if let DataType::Enum(operand_enum) = operand_type
        && path.len() == 1
        && state.enums[*operand_enum as usize]
            .variants
            .iter()
            .any(|variant| variant.name == path[0])
    {
        return true;
    }
    let (module, name) = path.split_at(path.len() - 1);
    let name = name[0].as_str();
    let scope = state.scope(ctx.file_idx);
    let names_type = (module.is_empty()
        && (matches!(name, "int" | "float" | "bool" | "string" | "any")
            || state.generics.bound(name).is_some()))
        || scope.find_enum(module, name).is_some()
        || scope
            .find_struct(module, name, span, ctx.file_idx, state.sources)
            .is_some()
        || find_template(module, name, scope, state.generics).is_some();
    !names_type
}

/// The type written after `as`, or after an `is` that names a type.
///
/// A generic type named without its arguments would be `any`, which every
/// value is; that is never what a test or a downcast means, so it is reported
/// with the arguments it needs.
pub fn resolve_type(t: &TypeExpr, span: Span, ctx: Ctx, state: &mut State<'_>) -> DataType {
    if let TypeExpr::Identifier(name, _) = t
        && state.generics.bound(name).is_none()
        && find_template(&[], name, state.scope(ctx.file_idx), state.generics).is_some()
    {
        compiler_errors::error_type_test(
            "Missing type arguments",
            &format!("{name} is generic, and a test or a downcast needs the instantiation"),
            Some(&format!("Name its type arguments, as in {name}<int>")),
            "type_test_needs_arguments",
            span,
            ctx.file_idx,
            state.sources,
        );
    }
    t.to_datatype(&mut state.type_ctx(ctx.file_idx))
}

/// A variant test on a value that is not an enum.
#[cold]
fn error_variant_test_not_enum(
    operand_type: &DataType,
    operand_span: Span,
    target_span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> ! {
    let found = state.type_names().of(operand_type).to_string();
    let help = if *operand_type == DataType::Unknown {
        "An `any` value is tested for its enum first, as in `v is Option<int> && v is Some(x)`"
    } else {
        "A variant test reads an enum value"
    };
    compiler_errors::error_type_test(
        "Variant test on a value that is not an enum",
        &format!("This value is {found}, which has no variants"),
        Some(help),
        "variant_test_not_enum",
        operand_span.extend(target_span),
        ctx.file_idx,
        state.sources,
    )
}

/// A downcast the operand's static type rules out.
#[cold]
pub fn error_impossible_cast(
    have: &DataType,
    want: &DataType,
    span: Span,
    ctx: Ctx,
    state: &State<'_>,
) -> ! {
    let names = state.type_names();
    let (have, want) = (names.of(have).to_string(), names.of(want).to_string());
    compiler_errors::error_type_test(
        "Impossible downcast",
        &format!("A value of type {have} is never of type {want}"),
        Some(&format!(
            "A value turns into another type with a conversion, as in {want}(x), where one exists"
        )),
        "impossible_downcast",
        span,
        ctx.file_idx,
        state.sources,
    )
}

/// One entry of the list a type test checks a value against; see
/// [`type_code`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Code {
    Flat(i64),
    /// A list, and what each element may be. Empty for anything.
    List(Vec<Self>),
    /// A map, and what each key and each value may be. Empty for anything.
    Map(Vec<Self>, Vec<Self>),
}

/// The codes a value of type `t` may have, or `None` for `any`, which every
/// value is.
fn codes_of(t: &DataType, state: &State<'_>) -> Option<Vec<Code>> {
    let mut codes = Vec::new();
    push_codes(t, state, &mut codes).then_some(codes)
}

fn push_codes(t: &DataType, state: &State<'_>, codes: &mut Vec<Code>) -> bool {
    let code = match t {
        DataType::Int => Code::Flat(type_code::INT),
        DataType::Float => Code::Flat(type_code::FLOAT),
        DataType::String => Code::Flat(type_code::STRING),
        DataType::Bool => Code::Flat(type_code::BOOL),
        DataType::Null => Code::Flat(type_code::NULL),
        DataType::Array(element) => match element.as_deref().and_then(|e| codes_of(e, state)) {
            Some(elements) => Code::List(elements),
            None => Code::Flat(type_code::LIST),
        },
        DataType::Map(entry) => {
            let keys = entry.0.as_ref().and_then(|k| codes_of(k, state));
            let values = entry.1.as_ref().and_then(|v| codes_of(v, state));
            if keys.is_none() && values.is_none() {
                Code::Flat(type_code::MAP)
            } else {
                Code::Map(keys.unwrap_or_default(), values.unwrap_or_default())
            }
        }
        DataType::Fn(_) | DataType::FnValue(_) => Code::Flat(type_code::FUNCTION),
        DataType::Struct(id) => Code::Flat(
            type_code::STRUCT | (i64::from(state.structs[*id as usize].id) << type_code::KIND_BITS),
        ),
        DataType::Enum(id) => Code::Flat(
            type_code::ENUM | (i64::from(state.enums[*id as usize].id) << type_code::KIND_BITS),
        ),
        DataType::Union(members) => {
            return members
                .iter()
                .all(|member| push_codes(member, state, codes));
        }
        DataType::Unknown => return false,
    };
    if !codes.contains(&code) {
        codes.push(code);
    }
    true
}

/// The constant register holding `codes` in the form the VM reads.
fn codes_register(codes: &[Code], state: &mut State<'_>) -> u16 {
    let flat: Option<Vec<i64>> = codes
        .iter()
        .map(|code| match code {
            Code::Flat(code) => Some(*code),
            _ => None,
        })
        .collect();
    if let Some(flat) = flat {
        return state.type_codes_register(&flat);
    }
    let list = codes_list(codes, state);
    state.const_register(list)
}

fn codes_list(codes: &[Code], state: &mut State<'_>) -> Data {
    let items: Vec<Data> = codes
        .iter()
        .map(|code| match code {
            Code::Flat(code) => Data::int(*code),
            Code::List(elements) => {
                let elements = codes_list(elements, state);
                pool_list(vec![Data::int(type_code::LIST), elements], state)
            }
            Code::Map(keys, values) => {
                let keys = codes_list(keys, state);
                let values = codes_list(values, state);
                pool_list(vec![Data::int(type_code::MAP), keys, values], state)
            }
        })
        .collect();
    pool_list(items, state)
}

fn pool_list(items: Vec<Data>, state: &mut State<'_>) -> Data {
    state.pools.objs.push(items);
    Data::array((state.pools.objs.len() - 1) as u32)
}

/// The scalar test `is` compiles to for `t`, where one instruction answers it.
const fn scalar_test(t: &DataType) -> Option<LibFunc> {
    match t {
        DataType::Int => Some(LibFunc::IsIntVal),
        DataType::Float => Some(LibFunc::IsFloatVal),
        DataType::String => Some(LibFunc::IsStrVal),
        DataType::Bool => Some(LibFunc::IsBoolVal),
        _ => None,
    }
}

/// The scalar downcast `as` compiles to for `t`, where one instruction does it.
const fn scalar_downcast(t: &DataType) -> Option<LibFunc> {
    match t {
        DataType::Int => Some(LibFunc::AsIntVal),
        DataType::Float => Some(LibFunc::AsFloatVal),
        DataType::String => Some(LibFunc::AsStrVal),
        DataType::Bool => Some(LibFunc::AsBoolVal),
        _ => None,
    }
}

/// Emits the test or the downcast of the value in `src` against `t`, into
/// `dest`. `t` is never `any`: that one is settled without a look.
fn emit_check(
    downcast: bool,
    src: u16,
    t: &DataType,
    dest: u16,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) {
    let scalar = if downcast {
        scalar_downcast(t)
    } else {
        scalar_test(t)
    };
    if let Some(libfunc) = scalar {
        output.push(Instr::CallLibFunc(libfunc, src, dest));
        return;
    }
    let codes = codes_of(t, state).unwrap_or_default();
    let codes = codes_register(&codes, state);
    output.push(Instr::StoreFuncArg(codes));
    *state.allocated_arg_count += 1;
    let libfunc = if downcast {
        LibFunc::AsTypeVal
    } else {
        LibFunc::IsTypeVal
    };
    output.push(Instr::CallLibFunc(libfunc, src, dest));
}

/// Compiles `operand as t`, returning where the value is.
#[allow(clippy::too_many_arguments)]
pub fn compile_cast(
    operand: &Expr,
    target: &TypeExpr,
    operand_span: Span,
    target_span: Span,
    tgt_id: Option<u16>,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> u16 {
    let have = operand.infer_type(v, ctx, state);
    let want = resolve_type(target, target_span, ctx, state);
    let span = operand_span.extend(target_span);
    let settled = settle(&have, &want, state.generics);
    if settled == Settled::Never {
        error_impossible_cast(&have, &want, span, ctx, state);
    }
    // A function becomes a value only where it is built as one.
    if matches!(want, DataType::FnValue(_)) && settled == Settled::Always {
        return super::compile_fn_value(operand, v, ctx, state, output, tgt_id);
    }
    if settled == Settled::Always {
        return operand
            .compile(v, ctx, state, output, tgt_id, false, true)
            .unwrap_id();
    }
    let src = operand
        .compile(v, ctx, state, output, None, false, true)
        .unwrap_id();
    state.free_reg(src, v);
    let dest = state.alloc_reg_tgt(tgt_id);
    emit_check(true, src, &want, dest, state, output);
    state.add_to_src(ctx, output, span);
    dest
}

/// What an `is` that held makes true: instructions that read a variant's
/// payload, to run once the test passed, the names that come into scope, and
/// the registers to give back once the payload is read.
#[derive(Default)]
pub struct Facts {
    reads: Vec<Instr>,
    vars: Vec<Variable>,
    temps: Vec<u16>,
}

impl Facts {
    /// Emits the reads where the test is known to have held and brings the
    /// names into scope. The caller takes them out of scope by truncating `v`
    /// to where it stood before the condition.
    pub fn apply(self, v: &mut Vec<Variable>, state: &mut State<'_>, output: &mut Vec<Instr>) {
        output.extend(self.reads);
        v.extend(self.vars);
        for temp in self.temps {
            state.free_reg(temp, v);
        }
    }
}

/// Compiles `operand is target` to a bool, returning its register and, when
/// `want_facts` is set, what holds where the test passed.
#[allow(clippy::too_many_arguments)]
pub fn compile_is(
    operand: &Expr,
    target: &IsTarget,
    operand_span: Span,
    target_span: Span,
    tgt_id: Option<u16>,
    want_facts: bool,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    output: &mut Vec<Instr>,
) -> (u16, Facts) {
    let have = operand.infer_type(v, ctx, state);
    let mut facts = Facts::default();
    match resolve_test(&have, target, operand_span, target_span, ctx, state) {
        Test::Type(want) => {
            let settled = settle(&have, &want, state.generics);
            // A test the static type settles costs nothing to run. One it
            // rules out is no mistake: a function compiles once per type its
            // callers pass, and the test is how one body serves them all.
            if settled != Settled::AtRunTime {
                if !matches!(operand, Expr::Var(..)) {
                    let id = operand
                        .compile(v, ctx, state, output, None, false, true)
                        .unwrap_id();
                    state.free_reg(id, v);
                }
                if settled == Settled::Never {
                    push_narrowed(operand, want, want_facts, v, &mut facts);
                }
                let answer = Data::bool(settled == Settled::Always);
                return (state.const_register(answer), facts);
            }
            let src = operand
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            state.free_reg(src, v);
            let dest = state.alloc_reg_tgt(tgt_id);
            emit_check(false, src, &want, dest, state, output);
            push_narrowed(operand, want, want_facts, v, &mut facts);
            (dest, facts)
        }
        Test::Variant {
            enum_id,
            variant_idx,
            binders,
        } => {
            let scrut = operand
                .compile(v, ctx, state, output, None, false, true)
                .unwrap_id();
            let tag = state.alloc_reg();
            output.push(Instr::GetFieldStruct(scrut, 0, tag));
            let idx = state.alloc_reg();
            output.push(Instr::SetInt(idx, i32::from(variant_idx)));
            state.free_reg(tag, v);
            state.free_reg(idx, v);
            let dest = state.alloc_reg_tgt(tgt_id);
            output.push(Instr::Eq(tag, idx, dest));
            if !want_facts {
                state.free_reg(scrut, v);
                return (dest, facts);
            }
            let payload = state.enums[enum_id as usize].variants[variant_idx as usize]
                .payload
                .clone();
            for (i, (binder, var_type)) in binders.iter().zip(payload).enumerate() {
                if binder.as_str() == "_" {
                    continue;
                }
                let field = (i + 1) as u16;
                let value = state.alloc_reg();
                facts.reads.push(Instr::GetFieldStruct(scrut, field, value));
                let captured = state.captured_binders.contains(binder);
                let register_id = if captured {
                    let cell = state.alloc_reg();
                    facts.reads.push(Instr::NewCell(value, cell));
                    cell
                } else {
                    value
                };
                facts.vars.push(Variable {
                    declared: None,
                    name: binder.clone(),
                    register_id,
                    cell: captured,
                    var_type,
                });
            }
            facts.temps.push(scrut);
            (dest, facts)
        }
    }
}

/// Narrows a variable `operand` to `want` where the test held: the same
/// register, read as the type it was tested for.
fn push_narrowed(
    operand: &Expr,
    want: DataType,
    want_facts: bool,
    v: &[Variable],
    facts: &mut Facts,
) {
    if want_facts
        && let Expr::Var(name, _) = operand
        && let Some(var) = v.iter().rfind(|var| &var.name == name)
    {
        facts.vars.push(Variable {
            declared: var.declared.clone(),
            name: name.clone(),
            register_id: var.register_id,
            cell: var.cell,
            var_type: want,
        });
    }
}

/// The names and types an `is` that held brings into scope, for the walks
/// that type code without compiling it. Mirrors what [`compile_is`] binds.
pub fn is_fact_types(
    operand: &Expr,
    target: &IsTarget,
    operand_span: Span,
    target_span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<(SmolStr, DataType)> {
    let have = operand.infer_type(v, ctx, state);
    match resolve_test(&have, target, operand_span, target_span, ctx, state) {
        Test::Type(want) => match operand {
            Expr::Var(name, _) if settle(&have, &want, state.generics) != Settled::Always => {
                vec![(name.clone(), want)]
            }
            _ => Vec::new(),
        },
        Test::Variant {
            enum_id,
            variant_idx,
            binders,
        } => {
            let payload = &state.enums[enum_id as usize].variants[variant_idx as usize].payload;
            binders
                .iter()
                .zip(payload.iter())
                .filter(|(binder, _)| binder.as_str() != "_")
                .map(|(binder, t)| (binder.clone(), t.clone()))
                .collect()
        }
    }
}

/// Declares in `v`, for a walk that types code without compiling it, what
/// `condition` makes true where it held: what each `is` in its chain of `&&`
/// says. The caller truncates `v` back once the code the condition guards is
/// walked.
pub fn declare_condition_facts(
    condition: &Expr,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) {
    match condition {
        Expr::BoolAnd(l, r, _, _) => {
            declare_condition_facts(l, v, ctx, state);
            declare_condition_facts(r, v, ctx, state);
        }
        Expr::Is(operand, target, operand_span, target_span) => {
            for (name, var_type) in
                is_fact_types(operand, target, *operand_span, *target_span, v, ctx, state)
            {
                // The operand narrowed keeps what its `let` declared; a
                // variant's binders are new variables.
                let declared = match &**operand {
                    Expr::Var(operand_name, _) if *operand_name == name => v
                        .iter()
                        .rfind(|var| var.name == name)
                        .and_then(|var| var.declared.clone()),
                    _ => None,
                };
                v.push(Variable {
                    declared,
                    name,
                    register_id: 0,
                    cell: false,
                    var_type,
                });
            }
        }
        _ => {}
    }
}

/// The names the variant tests in `condition` bind that a closure in `scope`
/// (the code the condition guards) or in another operand of its `&&` chain
/// captures. Those are bound in cells.
#[must_use]
pub fn captured_binders(condition: &Expr, scope: &[Expr]) -> Vec<SmolStr> {
    let mut conjuncts: Vec<&Expr> = Vec::new();
    flatten_and(condition, &mut conjuncts);
    let mut captured = Vec::new();
    for conjunct in &conjuncts {
        let Expr::Is(_, target, _, _) = conjunct else {
            continue;
        };
        let IsTarget::Variant(Expr::FunctionCall(binders, _, _, _, _)) = &**target else {
            continue;
        };
        for binder in binders {
            let Expr::Var(name, _) = binder else {
                continue;
            };
            let in_scope = super::expr::code_captures_variable(name, scope)
                || conjuncts.iter().any(|other| {
                    !std::ptr::eq(*other, *conjunct)
                        && super::expr::code_captures_variable(name, std::slice::from_ref(*other))
                });
            if in_scope && !captured.contains(name) {
                captured.push(name.clone());
            }
        }
    }
    captured
}

fn flatten_and<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::BoolAnd(l, r, _, _) = expr {
        flatten_and(l, out);
        flatten_and(r, out);
    } else {
        out.push(expr);
    }
}
