use super::expr::Expr;
use super::expr::Span;
use super::expr::mangle_method;
use super::expr::symbol_of_expr;
use crate::compiler::Namespace;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::FnSignature;
use crate::compiler::compiler_data::Function;
use crate::compiler::compiler_data::Source;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::error_invalid_type;
use crate::compiler::compiler_errors::error_op;
use crate::compiler::compiler_errors::error_struct_unknown_field;
use crate::compiler::compiler_errors::error_unknown_function;
use crate::compiler::compiler_errors::error_unknown_function_in_namespace;
use crate::compiler::compiler_errors::error_unknown_struct;
use crate::compiler::compiler_errors::error_unknown_type;
use crate::compiler::compiler_errors::error_unknown_type_with_namespace;
use crate::compiler::compiler_errors::error_unknown_variable;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::cell::RefCell;
use std::collections::HashSet;
use std::hint::cold_path;
use std::hint::unreachable_unchecked;
use std::rc::Rc;

pub use crate::rt::DataType;

/// Name prefix for the synthetic top-level function an anonymous function is
/// hoisted to. `<` is not a legal identifier character, so a hoisted name can
/// never collide with a user-written function.
const ANON_FN_PREFIX: &str = "<anon>";

// Tracks which user-defined functions are currently being analysed for their
// return type. Used to break mutual-recursion cycles in type inference
thread_local! {
    static RETURN_TYPE_INFERRING: RefCell<FxHashSet<usize>> =
        RefCell::new(FxHashSet::default());
}

/// Clears inference bookkeeping left behind when a previous compilation on
/// this thread was aborted by an error unwind (see `errors::collect_diagnostic`).
pub fn reset_inference_state() {
    RETURN_TYPE_INFERRING.with(|s| s.borrow_mut().clear());
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TypeExpr {
    Identifier(SmolStr, Span),
    NamespacedIdentifier(Box<[SmolStr]>, Span),
    Array(Box<Self>),
    Map(Box<Self>, Box<Self>),
    Union(Box<[Self]>),
}

impl TypeExpr {
    #[must_use]
    pub fn to_datatype(
        &self,
        file_idx: u16,
        namespace: &Namespace,
        sources: &[Source],
    ) -> DataType {
        match self {
            Self::Identifier(s, span) => match s.as_str() {
                "int" => DataType::Int,
                "float" => DataType::Float,
                "bool" => DataType::Bool,
                "string" => DataType::String,
                "null" => DataType::Null,
                // A dynamically-typed slot. Written `any`; modeled as `Unknown`,
                // which the type checker already treats permissively. Used for
                // enum payloads that hold a value of any type (option/result).
                "any" => DataType::Unknown,
                struct_name => {
                    if let Some(struct_id) =
                        namespace.find_struct(&[], struct_name, *span, file_idx, sources)
                    {
                        DataType::Struct(struct_id as u16)
                    } else if let Some(enum_id) = namespace.find_enum(&[], struct_name) {
                        DataType::Enum(enum_id as u16)
                    } else {
                        error_unknown_type(*span, file_idx, struct_name, sources, namespace);
                    }
                }
            },
            Self::NamespacedIdentifier(s, span) => {
                if let Some(struct_id) = namespace.find_struct(
                    &s[..s.len() - 1],
                    unsafe { s.last().unwrap_unchecked() },
                    *span,
                    file_idx,
                    sources,
                ) {
                    DataType::Struct(struct_id as u16)
                } else if let Some(enum_id) =
                    namespace.find_enum(&s[..s.len() - 1], unsafe { s.last().unwrap_unchecked() })
                {
                    DataType::Enum(enum_id as u16)
                } else {
                    cold_path();
                    error_unknown_type_with_namespace(
                        *span,
                        file_idx,
                        unsafe { s.last().unwrap_unchecked() },
                        sources,
                        namespace,
                        &s[..s.len() - 1],
                    )
                }
            }
            Self::Array(inner_t) => DataType::Array(Some(Box::new(
                inner_t.to_datatype(file_idx, namespace, sources),
            ))),
            Self::Map(k_t, v_t) => DataType::Map(Box::from((
                Some(k_t.to_datatype(file_idx, namespace, sources)),
                Some(v_t.to_datatype(file_idx, namespace, sources)),
            ))),
            Self::Union(poly) => DataType::Union(
                poly.iter()
                    .map(|t| t.to_datatype(file_idx, namespace, sources))
                    .collect(),
            )
            .check_poly(),
        }
    }
}

/// Renders a [`DataType`] with full struct/function detail (field names, arg
/// names) for diagnostics, resolving `Struct`/`Fn` ids against the compiler
/// `State`. The plain `Display` impl (in `candela-vm`) has no `State`, so it
/// renders those variants opaquely; this is the compiler-side detailed form.
#[must_use]
pub fn format_detailed(t: &DataType, state: &State<'_>) -> SmolStr {
    match t {
        DataType::Float => SmolStr::new_static("float"),
        DataType::Int => SmolStr::new_static("int"),
        DataType::Bool => SmolStr::new_static("bool"),
        DataType::String => SmolStr::new_static("string"),
        DataType::Array(array_type) => match array_type {
            Some(array_type) => {
                format_args!("{}[]", format_detailed(array_type, state)).to_smolstr()
            }
            None => SmolStr::new_static("Unknown[]"),
        },
        DataType::Null => SmolStr::new_static("null"),
        DataType::Unknown => SmolStr::new_static("Unknown"),
        DataType::Union(types) => format_args!(
            "{}",
            types
                .into_iter()
                .map(|x| format_detailed(x, state))
                .collect::<Vec<SmolStr>>()
                .join("|")
        )
        .to_smolstr(),
        DataType::Struct(s) => {
            let s = &state.structs[*s as usize];
            format_args!(
                "{} {{{}}}",
                s.name,
                s.fields
                    .iter()
                    .map(|(n, t, _)| {
                        format_args!("{n}: {}", format_detailed(t, state)).to_smolstr()
                    })
                    .collect::<Vec<SmolStr>>()
                    .join(", ")
            )
            .to_smolstr()
        }
        DataType::Enum(e) => state.enums[*e as usize].name.clone(),
        DataType::Map(m) => format_args!(
            "{{{}: {}}}",
            m.0.as_ref().unwrap_or(&DataType::Unknown),
            m.1.as_ref().unwrap_or(&DataType::Unknown)
        )
        .to_smolstr(),
        DataType::Fn(id) => {
            let f = &state.fns[*id as usize];
            format_args!(
                "fn ({})",
                f.args
                    .iter()
                    .map(|(a, _)| a.clone())
                    .collect::<Vec<SmolStr>>()
                    .join(", ")
            )
            .to_smolstr()
        }
    }
}

#[inline(always)]
#[must_use]
pub fn struct_field_type_matches(expected: &DataType, received: &DataType) -> bool {
    received == &DataType::Null || expected == received
}

/// Equality for monomorphization and return-type cache keys.
///
/// Identical to the loose type `==` except that function-typed arguments compare
/// by exact `Fn` id, so each distinct function passed to a higher-order function
/// keys its own specialization. Function references are always top-level
/// arguments (a function is passed directly, never nested inside an array or
/// map), so only the top-level `Fn` case needs the stricter rule.
#[must_use]
pub fn arg_types_specialize_equal(a: &[DataType], b: &[DataType]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| match (x, y) {
            (DataType::Fn(i), DataType::Fn(j)) => i == j,
            (DataType::Fn(_), _) | (_, DataType::Fn(_)) => false,
            _ => x == y,
        })
}

/// Collect all the function calls in the given code
pub fn collect_direct_fn_calls(content: &[Expr], calls: &mut Vec<SmolStr>) {
    let mut expr_stack: Vec<&Expr> = content.iter().collect();
    while let Some(expression) = expr_stack.pop() {
        match expression {
            Expr::FunctionCall(args, namespace, _, _) => {
                calls.push(namespace.last().unwrap().clone());
                expr_stack.extend(args.iter());
            }
            Expr::Condition(x, y, _)
            | Expr::InlineCondition(x, y, _)
            | Expr::ElseIfBlock(x, y)
            | Expr::WhileBlock(x, y)
            | Expr::ObjFunctionCall(x, y, _, _, _, _) => {
                expr_stack.push(x);
                expr_stack.extend(y.iter());
            }
            Expr::ElseBlock(x) | Expr::EvalBlock(x) | Expr::LoopBlock(x) => {
                expr_stack.extend(x.iter());
            }
            Expr::ReturnVal(code) => {
                if let Some(code) = code.as_ref() {
                    expr_stack.push(code);
                }
            }
            Expr::FunctionDecl(_, _, x, _) => expr_stack.extend(x.iter()),
            Expr::ArrayGetSlice(x, y, z, _) => {
                expr_stack.push(x);
                expr_stack.push(y);
                expr_stack.push(z);
            }
            Expr::VarDeclare(_, x)
            | Expr::VarAssign(_, x, _)
            | Expr::Neg(x, _, _)
            | Expr::BoolNeg(x, _, _) => expr_stack.push(x),
            Expr::ForLoop(_, _, code, _) => expr_stack.extend(code.iter()),
            Expr::IntForLoop(_, start, end, code, _, _) => {
                expr_stack.push(start);
                expr_stack.push(end);
                expr_stack.extend(code.iter());
            }
            Expr::ArrayModify(array, index, value, _, _) => {
                expr_stack.push(array);
                expr_stack.push(index);
                expr_stack.push(value);
            }
            Expr::Array(elems, _) => expr_stack.extend(elems.iter()),
            Expr::Struct(_, fields, _) => {
                expr_stack.extend(fields.iter().map(|(_, expr, _, _)| expr));
            }
            Expr::GetStructField(expr, _, _, _) => expr_stack.push(expr),
            Expr::SetStructField(expr, _, value, _, _, _) => {
                expr_stack.push(expr);
                expr_stack.push(value);
            }
            Expr::TryCatchBlock(try_code, _, catch_code) => {
                expr_stack.extend(try_code.iter());
                expr_stack.extend(catch_code.iter());
            }
            Expr::Match(scrutinee, arms, wildcard, _) => {
                expr_stack.push(scrutinee);
                for (pat, body) in arms {
                    expr_stack.push(pat);
                    expr_stack.extend(body.iter());
                }
                if let Some(w) = wildcard {
                    expr_stack.extend(w.iter());
                }
            }
            Expr::ArrayGetIndex(x, y, _)
            | Expr::Mul(x, y, _, _)
            | Expr::Div(x, y, _, _)
            | Expr::Add(x, y, _, _)
            | Expr::Sub(x, y, _, _)
            | Expr::Mod(x, y, _, _)
            | Expr::Pow(x, y, _, _)
            | Expr::Eq(x, y)
            | Expr::NotEq(x, y)
            | Expr::Sup(x, y, _, _)
            | Expr::SupEq(x, y, _, _)
            | Expr::Inf(x, y, _, _)
            | Expr::InfEq(x, y, _, _)
            | Expr::BoolAnd(x, y, _, _)
            | Expr::BoolOr(x, y, _, _) => {
                expr_stack.push(x);
                expr_stack.push(y);
            }
            _ => {}
        }
    }
}

/// Check if the function src_fn can call target_fn
pub fn can_reach(
    src_fn: &str,
    target_fn: &str,
    fns: &[Function],
    visited: &mut HashSet<SmolStr>,
) -> bool {
    if let Some(from_fn) = fns.iter().find(|f| f.name.as_str() == src_fn) {
        for callee in &from_fn.direct_calls {
            if callee == target_fn {
                return true;
            }
            if visited.insert(callee.clone()) && can_reach(callee, target_fn, fns, visited) {
                return true;
            }
        }
    }
    false
}

#[must_use]
pub fn check_if_returns_void(content: &[Expr]) -> bool {
    for content in content {
        match content {
            Expr::ElseIfBlock(_, code)
            | Expr::ElseBlock(code)
            | Expr::Condition(_, code, _)
            | Expr::InlineCondition(_, code, _)
            | Expr::WhileBlock(_, code)
            | Expr::ForLoop(_, _, code, _)
            | Expr::EvalBlock(code)
            | Expr::LoopBlock(code)
            | Expr::IntForLoop(_, _, _, code, _, _) => {
                if !check_if_returns_void(code) {
                    return false;
                }
            }
            Expr::Match(_, arms, wildcard, _) => {
                for (_, body) in arms {
                    if !check_if_returns_void(body) {
                        return false;
                    }
                }
                if let Some(w) = wildcard
                    && !check_if_returns_void(w)
                {
                    return false;
                }
            }
            Expr::ReturnVal(return_val) if return_val.is_some() => {
                return false;
            }
            _ => {}
        }
    }
    true
}

macro_rules! add_return_type {
    ($return_types: expr, $return_type: expr) => {
        if $return_type != DataType::Unknown && !($return_types).contains(&($return_type)) {
            ($return_types).push($return_type);
        }
    };
}

macro_rules! extend_return_types {
    ($return_types: expr, $new_types: expr) => {
        for return_type in $new_types {
            add_return_type!($return_types, return_type);
        }
    };
}

pub fn track_returns(
    content: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    fn_name: &str,
) -> Vec<DataType> {
    let mut flow = track_return_flow(content, v, ctx, state, fn_name);
    if !flow.always_returns && !flow.types.is_empty() {
        add_return_type!(&mut flow.types, DataType::Null);
    }
    flow.types
}

struct FnReturnFlow {
    types: Vec<DataType>,
    always_returns: bool,
}

fn track_scoped_returns(
    code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    fn_name: &str,
) -> FnReturnFlow {
    let v_len = v.len();
    let flow = track_return_flow(code, v, ctx, state, fn_name);
    v.truncate(v_len);
    flow
}

fn track_condition_returns(
    code: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    fn_name: &str,
) -> FnReturnFlow {
    let mut return_types = Vec::new();
    let first_branch_end = code
        .iter()
        .position(|expr| matches!(expr, Expr::ElseIfBlock(_, _) | Expr::ElseBlock(_)))
        .unwrap_or(code.len());

    let first_flow = track_scoped_returns(&code[..first_branch_end], v, ctx, state, fn_name);
    let mut all_branches_return = first_flow.always_returns;
    let mut has_else = false;
    extend_return_types!(&mut return_types, first_flow.types);

    for expr in &code[first_branch_end..] {
        match expr {
            Expr::ElseIfBlock(_, branch_code) => {
                let flow = track_scoped_returns(branch_code, v, ctx, state, fn_name);
                all_branches_return &= flow.always_returns;
                extend_return_types!(&mut return_types, flow.types);
            }
            Expr::ElseBlock(branch_code) => {
                has_else = true;
                let flow = track_scoped_returns(branch_code, v, ctx, state, fn_name);
                all_branches_return &= flow.always_returns;
                extend_return_types!(&mut return_types, flow.types);
            }
            _ => {}
        }
    }

    FnReturnFlow {
        types: return_types,
        always_returns: has_else && all_branches_return,
    }
}

fn track_return_flow(
    content: &[Expr],
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
    fn_name: &str,
) -> FnReturnFlow {
    let mut return_types: Vec<DataType> = Vec::new();
    for expr in content {
        match expr {
            Expr::Condition(_, code, _) | Expr::InlineCondition(_, code, _) => {
                let flow = track_condition_returns(code, v, ctx, state, fn_name);
                extend_return_types!(&mut return_types, flow.types);
                if flow.always_returns {
                    return FnReturnFlow {
                        types: return_types,
                        always_returns: true,
                    };
                }
            }
            Expr::ElseIfBlock(_, code)
            | Expr::ElseBlock(code)
            | Expr::EvalBlock(code)
            | Expr::LoopBlock(code) => {
                let flow = track_scoped_returns(code, v, ctx, state, fn_name);
                extend_return_types!(&mut return_types, flow.types);
                if flow.always_returns {
                    return FnReturnFlow {
                        types: return_types,
                        always_returns: true,
                    };
                }
            }
            Expr::VarDeclare(name, expr) => {
                let var_type = expr.infer_type(v, ctx, state);
                v.push(Variable {
                    name: name.clone(),
                    register_id: 0,
                    var_type,
                });
            }
            Expr::VarAssign(name, expr, _) => {
                let var_type = expr.infer_type(v, ctx, state);
                if let Some(var) = v.iter_mut().rfind(|var| &var.name == name) {
                    var.var_type = var_type;
                }
            }
            Expr::WhileBlock(_, code) => {
                let flow = track_scoped_returns(code, v, ctx, state, fn_name);
                extend_return_types!(&mut return_types, flow.types);
            }
            Expr::IntForLoop(var_name, _, _, code, _, _) => {
                let v_len = v.len();
                v.push(Variable {
                    name: var_name.clone(),
                    register_id: 0,
                    var_type: DataType::Int,
                });
                let flow = track_return_flow(code, v, ctx, state, fn_name);
                extend_return_types!(&mut return_types, flow.types);
                v.truncate(v_len);
            }
            Expr::ForLoop(var_name, array_expr, array_code, _) => {
                let inferred_collection_type = array_expr.infer_type(v, ctx, state);
                let elem_type = match inferred_collection_type {
                    DataType::Array(inner) => inner.map_or(DataType::Unknown, |t| *t),
                    DataType::String => DataType::String,
                    DataType::Unknown => DataType::Unknown,
                    // A map iterates its keys.
                    DataType::Map(m) => m.0.map_or(DataType::Unknown, |t| t),
                    _ => unsafe { unreachable_unchecked() },
                };
                let v_len = v.len();
                if var_name.as_str() != "_" {
                    v.push(Variable {
                        name: var_name.clone(),
                        register_id: 0,
                        var_type: elem_type,
                    });
                }
                let flow = track_return_flow(array_code, v, ctx, state, fn_name);
                extend_return_types!(&mut return_types, flow.types);
                v.truncate(v_len);
            }
            Expr::ObjFunctionCall(obj, args, namespace, _, _, _)
                if namespace.last().unwrap().as_str() == "push" =>
            {
                if let Expr::Var(var_name, _) = obj.as_ref()
                    && v.iter()
                        .rfind(|var| &var.name == var_name)
                        .is_some_and(|var| var.var_type == DataType::Array(None))
                {
                    let arg_type = args[0].infer_type(v, ctx, state);
                    if let Some(var) = v.iter_mut().rfind(|var| &var.name == var_name) {
                        var.var_type = DataType::Array(Some(Box::new(arg_type)));
                    }
                }
            }
            Expr::Match(scrutinee, arms, wildcard, span) => {
                let scrut_type = scrutinee.infer_type(v, ctx, state);
                let is_enum = matches!(scrut_type, DataType::Enum(_));
                let mut all_return = true;
                for (pat, body) in arms {
                    let v_len = v.len();
                    if let DataType::Enum(enum_id) = scrut_type {
                        let (vidx, binders) = crate::compiler::resolve_variant_pattern(
                            enum_id, pat, *span, ctx, state,
                        );
                        for (i, binder) in binders.iter().enumerate() {
                            if binder.as_str() != "_" {
                                let payload_type = state.enums[enum_id as usize].variants
                                    [vidx as usize]
                                    .payload[i]
                                    .clone();
                                v.push(Variable {
                                    name: binder.clone(),
                                    register_id: 0,
                                    var_type: payload_type,
                                });
                            }
                        }
                    }
                    let flow = track_return_flow(body, v, ctx, state, fn_name);
                    v.truncate(v_len);
                    all_return &= flow.always_returns;
                    extend_return_types!(&mut return_types, flow.types);
                }
                let exhaustive = if wildcard.is_some() {
                    if let Some(w) = wildcard {
                        let flow = track_scoped_returns(w, v, ctx, state, fn_name);
                        all_return &= flow.always_returns;
                        extend_return_types!(&mut return_types, flow.types);
                    }
                    true
                } else {
                    // An enum match with no wildcard is compile-time exhaustive.
                    is_enum
                };
                if exhaustive && all_return {
                    return FnReturnFlow {
                        types: return_types,
                        always_returns: true,
                    };
                }
            }
            Expr::ReturnVal(return_val) => {
                if let Some(val) = return_val.as_ref() {
                    let infered = val.infer_type(v, ctx, state);
                    add_return_type!(&mut return_types, infered);
                } else {
                    add_return_type!(&mut return_types, DataType::Null);
                }
                return FnReturnFlow {
                    types: return_types,
                    always_returns: true,
                };
            }
            _ => {}
        }
    }
    FnReturnFlow {
        types: return_types,
        always_returns: false,
    }
}

/// Infers the return type of a user function specialised for `infered_arg_types`,
/// caching the result on the function. Shared by direct `FunctionCall`s and by
/// `impl` method calls (which resolve to a mangled free function with the
/// receiver as argument 0). `function_name` is only used for diagnostics inside
/// `track_returns`.
fn infer_user_fn_return_type(
    fn_id: usize,
    infered_arg_types: Vec<DataType>,
    function_name: &str,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> DataType {
    let func = &state.fns[fn_id];
    // Check the return type cache
    if let Some((_, ret)) = func
        .return_type_cache
        .iter()
        .find(|(args, _)| arg_types_specialize_equal(args, &infered_arg_types))
    {
        return ret.clone();
    }

    let fn_args = func.args.clone();
    let fn_code = func.code.clone();
    let fn_src_file = func.src_file;
    let v_len_before_args = v.len();
    for (i, infered_type) in infered_arg_types.iter().cloned().enumerate() {
        // 0 => placeholder id, it's never used
        v.push(Variable {
            name: fn_args[i].0.clone(),
            register_id: 0,
            var_type: infered_type,
        });
    }

    // Mutual-recursion cycle guard -> if we are already in the middle of
    // inferring this function's return type, return Unknown to break the cycle
    let already_inferring = RETURN_TYPE_INFERRING.with(|s| s.borrow().contains(&fn_id));
    if already_inferring {
        v.truncate(v_len_before_args);
        return DataType::Unknown;
    }

    RETURN_TYPE_INFERRING.with(|s| s.borrow_mut().insert(fn_id));

    let fn_ctx = Ctx {
        file_idx: fn_src_file,
        ..ctx
    };
    let fn_type = track_returns(&fn_code, v, fn_ctx, state, function_name);

    RETURN_TYPE_INFERRING.with(|s| s.borrow_mut().remove(&fn_id));

    let to_return = if fn_type.is_empty() {
        // If function doesn't return anything, return nothing
        DataType::Null
    } else {
        // If function returns anything, check if it returns the same thing each time
        DataType::Union(Box::from(fn_type)).check_poly()
    };

    v.truncate(v_len_before_args);

    // Cache the result
    state.fns[fn_id]
        .return_type_cache
        .push((Box::from(infered_arg_types), to_return.clone()));

    to_return
}

impl Expr {
    pub fn infer_type(&self, v: &mut Vec<Variable>, ctx: Ctx, state: &mut State<'_>) -> DataType {
        match self {
            Self::Var(name, span) => {
                if let Some(var) = v.iter().rfind(|x| &x.name == name) {
                    var.var_type.clone()
                } else if let Some(fn_id) =
                    state
                        .namespace
                        .find_function(&[], name, *span, ctx.file_idx, state.sources)
                {
                    // A bare identifier that names a function is a function
                    // reference (a compile-time value passed to a higher-order
                    // function). Its static type is the callee's Fn id.
                    DataType::Fn(fn_id as u16)
                } else if let Some((enum_id, _)) =
                    crate::compiler::resolve_enum_variant(std::slice::from_ref(name), state)
                {
                    DataType::Enum(enum_id)
                } else {
                    error_unknown_variable(name, *span, v, ctx.file_idx, state.sources);
                }
            }
            Self::Float(_) => DataType::Float,
            Self::Int(_) => DataType::Int,
            Self::String(_) => DataType::String,
            Self::Bool(_) | Self::Eq(_, _) | Self::NotEq(_, _) => DataType::Bool,
            Self::Null => DataType::Null,
            Self::Array(x, _) => DataType::Array(if x.is_empty() {
                None
            } else {
                let elem_type = x
                    .iter()
                    .map(|elem| elem.infer_type(v, ctx, state))
                    .find(|elem_type| *elem_type != DataType::Unknown)
                    .unwrap_or(DataType::Unknown);
                Some(Box::from(elem_type))
            }),
            Self::Map(kv_pairs, _) => {
                if kv_pairs.is_empty() {
                    // An empty map literal has no key/value types yet, like an
                    // empty array (`Array(None)`); `insert` fills them in.
                    DataType::Map(Box::from((None, None)))
                } else {
                    let kv_type = kv_pairs
                        .iter()
                        .map(|(key, _, value, _)| {
                            (
                                key.infer_type(v, ctx, state),
                                value.infer_type(v, ctx, state),
                            )
                        })
                        .find(|(key_t, val_t)| {
                            key_t != &DataType::Unknown || val_t != &DataType::Unknown
                        })
                        .map_or(
                            (Some(DataType::Unknown), Some(DataType::Unknown)),
                            |(key_t, val_t)| (Some(key_t), Some(val_t)),
                        );
                    DataType::Map(Box::from(kv_type))
                }
            }
            Self::Add(x, y, span_l, span_r) => {
                match (x.infer_type(v, ctx, state), y.infer_type(v, ctx, state)) {
                    (DataType::Unknown, t) | (t, DataType::Unknown) => t,
                    (DataType::Float, DataType::Float) => DataType::Float,
                    (DataType::Int, DataType::Int) => DataType::Int,
                    (DataType::String, DataType::String) => DataType::String,
                    (DataType::Array(t1), DataType::Array(t2)) => DataType::Array(t1.or(t2)),
                    (l, r) => {
                        error_op(&l, &r, "+", *span_l, *span_r, ctx.file_idx, state.sources);
                    }
                }
            }
            Self::Mul(x, y, span_l, span_r)
            | Self::Div(x, y, span_l, span_r)
            | Self::Sub(x, y, span_l, span_r)
            | Self::Mod(x, y, span_l, span_r)
            | Self::Pow(x, y, span_l, span_r) => {
                match (x.infer_type(v, ctx, state), y.infer_type(v, ctx, state)) {
                    (DataType::Unknown, t) | (t, DataType::Unknown)
                        if matches!(t, DataType::Float | DataType::Int | DataType::Unknown) =>
                    {
                        t
                    }
                    (DataType::Float, DataType::Float) => DataType::Float,
                    (DataType::Int, DataType::Int) => DataType::Int,
                    (l, r) => {
                        error_op(
                            &l,
                            &r,
                            symbol_of_expr(self),
                            *span_l,
                            *span_r,
                            ctx.file_idx,
                            state.sources,
                        );
                    }
                }
            }
            Self::Sup(x, y, span_l, span_r)
            | Self::SupEq(x, y, span_l, span_r)
            | Self::Inf(x, y, span_l, span_r)
            | Self::InfEq(x, y, span_l, span_r) => {
                match (x.infer_type(v, ctx, state), y.infer_type(v, ctx, state)) {
                    (DataType::Unknown, DataType::Float | DataType::Int)
                    | (DataType::Float | DataType::Int, DataType::Unknown)
                    | (DataType::Float, DataType::Float)
                    | (DataType::Int, DataType::Int) => DataType::Bool,
                    (l, r) => error_op(
                        &l,
                        &r,
                        symbol_of_expr(self),
                        *span_l,
                        *span_r,
                        ctx.file_idx,
                        state.sources,
                    ),
                }
            }
            Self::BoolAnd(x, y, span_l, span_r) | Self::BoolOr(x, y, span_l, span_r) => {
                match (x.infer_type(v, ctx, state), y.infer_type(v, ctx, state)) {
                    (DataType::Unknown | DataType::Bool, DataType::Bool)
                    | (DataType::Bool, DataType::Unknown) => DataType::Bool,
                    (l, r) => {
                        error_op(&l, &r, "&&", *span_l, *span_r, ctx.file_idx, state.sources);
                    }
                }
            }
            Self::Neg(e, span_l, span_r) => match e.infer_type(v, ctx, state) {
                DataType::Float => DataType::Float,
                DataType::Int => DataType::Int,
                DataType::Unknown => DataType::Unknown,
                operand_type => error_op(
                    &DataType::Null,
                    &operand_type,
                    "-",
                    *span_l,
                    *span_r,
                    ctx.file_idx,
                    state.sources,
                ),
            },
            Self::BoolNeg(e, span_l, span_r) => match e.infer_type(v, ctx, state) {
                DataType::Bool => DataType::Bool,
                operand_type => error_op(
                    &DataType::Null,
                    &operand_type,
                    "!",
                    *span_l,
                    *span_r,
                    ctx.file_idx,
                    state.sources,
                ),
            },
            Self::ArrayGetIndex(array, _, _) => match array.infer_type(v, ctx, state) {
                DataType::Array(array_type) => array_type.map_or(DataType::Null, |t| *t),
                DataType::String => DataType::String,
                DataType::Unknown => DataType::Unknown,
                _ => unsafe { unreachable_unchecked() },
            },
            Self::GetStructField(s, field, struct_span, field_span) => {
                let s = s.infer_type(v, ctx, state);
                if let DataType::Struct(s_id) = s {
                    state.structs[s_id as usize]
                        .fields
                        .iter()
                        .find(|x| &x.0 == field)
                        .unwrap_or_else(|| {
                            let s = &state.structs[s_id as usize];
                            error_struct_unknown_field(
                                ctx.file_idx,
                                *field_span,
                                field,
                                &s.name,
                                &s.fields,
                                state.sources,
                            )
                        })
                        .1
                        .clone()
                } else {
                    error_invalid_type(
                        &DataType::Struct(0),
                        &s,
                        *struct_span,
                        None,
                        None,
                        ctx.file_idx,
                        state.sources,
                    );
                }
            }
            Self::ArrayGetSlice(array, _, _, _) => match array.infer_type(v, ctx, state) {
                DataType::Array(array_type) => DataType::Array(array_type),
                DataType::String => DataType::String,
                DataType::Unknown => DataType::Unknown,
                _ => unsafe { unreachable_unchecked() },
            },
            Self::FunctionCall(args, namespace, span, _) => {
                // A qualified enum-variant construction (`Color::Red(x)`) has an
                // enum type; intercept before the namespaced-function paths.
                if namespace.len() >= 2
                    && let Some((enum_id, _)) =
                        crate::compiler::resolve_enum_variant(namespace, state)
                {
                    return DataType::Enum(enum_id);
                }
                match namespace.last().unwrap().as_str() {
                    "print" | "write" | "append" | "delete" | "delete_dir" => DataType::Null,
                    "type" | "str" | "input" | "read" | "json_stringify" | "as_str" => {
                        DataType::String
                    }
                    "float" | "as_float" => DataType::Float,
                    "int" | "the_answer" | "as_int" => DataType::Int,
                    "bool" | "exists" | "as_bool" | "is_int" | "is_float" | "is_str"
                    | "is_bool" | "is_list" | "is_map" | "is_null" => DataType::Bool,
                    "range" => DataType::Array(Some(Box::from(DataType::Int))),
                    "argv" => DataType::Array(Some(Box::from(DataType::String))),
                    // A downcast to a collection yields an element/entry type of
                    // `any` (Unknown); json::parse yields a fully dynamic value.
                    "as_list" => DataType::Array(None),
                    "as_map" => DataType::Map(Box::from((None, None))),
                    "json_parse" => DataType::Unknown,
                    function_name => {
                        // A call to a function-typed parameter (a higher-order
                        // function calling the function it was handed): resolve
                        // the concrete callee from the parameter's static Fn type
                        // and infer that function's return type.
                        if namespace.len() == 1
                            && let Some(DataType::Fn(fn_id)) = v
                                .iter()
                                .rfind(|var| var.name.as_str() == function_name)
                                .map(|var| var.var_type.clone())
                        {
                            let infered_arg_types = args
                                .iter()
                                .map(|x| x.infer_type(v, ctx, state))
                                .collect::<Vec<DataType>>();
                            return infer_user_fn_return_type(
                                fn_id as usize,
                                infered_arg_types,
                                function_name,
                                v,
                                ctx,
                                state,
                            );
                        }
                        if let Some(lib) = state.dyn_libs.iter().find(|l| l.name == namespace[0])
                            && let Some(FnSignature {
                                return_type: fn_return_type,
                                ..
                            }) = lib.fns.iter().find(|x| x.name == function_name)
                        {
                            return fn_return_type.clone();
                        }
                        let infered_arg_types = args
                            .iter()
                            .map(|x| x.infer_type(v, ctx, state))
                            .collect::<Vec<DataType>>();

                        let Some(fn_id) = state
                            .fns
                            .iter()
                            .rposition(|func| func.name == function_name)
                        else {
                            // An unqualified call whose name is an enum variant
                            // (`Some(x)`) constructs that variant. User functions
                            // above keep priority.
                            if let Some((enum_id, _)) =
                                crate::compiler::resolve_enum_variant(namespace, state)
                            {
                                return DataType::Enum(enum_id);
                            }
                            if namespace.len() == 1 {
                                error_unknown_function(
                                    function_name,
                                    *span,
                                    state.namespace,
                                    ctx.file_idx,
                                    state.sources,
                                );
                            } else {
                                error_unknown_function_in_namespace(
                                    function_name,
                                    state.namespace,
                                    &namespace[..namespace.len() - 1],
                                    *span,
                                    ctx.file_idx,
                                    state.sources,
                                );
                            }
                        };

                        infer_user_fn_return_type(
                            fn_id,
                            infered_arg_types,
                            function_name,
                            v,
                            ctx,
                            state,
                        )
                    }
                }
            }
            Self::ObjFunctionCall(obj, args, namespace, _, fn_span, _) => {
                let method = namespace.last().unwrap().as_str();
                let obj_type = obj.infer_type(v, ctx, state);
                // A user-defined impl method resolves by the receiver's static
                // struct type to the mangled free function `Type#method`; its
                // return type is inferred exactly like any free function's. This
                // is checked BEFORE the builtin-method table so a struct method
                // that happens to share a name with a builtin (e.g. `len`) uses
                // its own return type rather than the builtin's.
                if let DataType::Struct(struct_id) = obj_type {
                    let struct_name = state.structs[struct_id as usize].name.clone();
                    let mangled = mangle_method(&struct_name, method);
                    if let Some(fn_id) = state.fns.iter().position(|f| f.name == mangled) {
                        let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len() + 1);
                        arg_types.push(DataType::Struct(struct_id));
                        for a in args {
                            arg_types.push(a.infer_type(v, ctx, state));
                        }
                        return infer_user_fn_return_type(
                            fn_id, arg_types, &mangled, v, ctx, state,
                        );
                    }
                    // No matching method: mirror the compile-time error path so
                    // inference does not hit the builtin arms with a struct type.
                    crate::compiler::compiler_errors::error_no_such_method(
                        method,
                        &struct_name,
                        *fn_span,
                        ctx.file_idx,
                        state.sources,
                    );
                }
                if let DataType::Enum(enum_id) = obj_type {
                    let enum_name = state.enums[enum_id as usize].name.clone();
                    let mangled = mangle_method(&enum_name, method);
                    if let Some(fn_id) = state.fns.iter().position(|f| f.name == mangled) {
                        let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len() + 1);
                        arg_types.push(DataType::Enum(enum_id));
                        for a in args {
                            arg_types.push(a.infer_type(v, ctx, state));
                        }
                        return infer_user_fn_return_type(
                            fn_id, arg_types, &mangled, v, ctx, state,
                        );
                    }
                    crate::compiler::compiler_errors::error_no_such_method(
                        method,
                        &enum_name,
                        *fn_span,
                        ctx.file_idx,
                        state.sources,
                    );
                }
                // An array collection method routed to a `std/list` helper
                // infers its return type from that helper, specialized for the
                // receiver and argument types.
                if let Some(fn_id) = crate::compiler::methods::routed_list_method(
                    method, &obj_type, args, v, ctx, state,
                ) {
                    let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len() + 1);
                    arg_types.push(obj_type.clone());
                    for a in args {
                        arg_types.push(a.infer_type(v, ctx, state));
                    }
                    return infer_user_fn_return_type(fn_id, arg_types, method, v, ctx, state);
                }
                match method {
                    "uppercase"
                    | "lowercase"
                    | "replace"
                    | "trim"
                    | "trim_sequence"
                    | "trim_left"
                    | "trim_right"
                    | "trim_sequence_left"
                    | "trim_sequence_right"
                    | "join" => DataType::String,
                    "starts_with" | "ends_with" | "contains" | "is_float" | "is_int" => {
                        DataType::Bool
                    }
                    "len" | "find" => DataType::Int,
                    "repeat" | "reverse" => {
                        let obj_type = obj.infer_type(v, ctx, state);
                        if obj_type == DataType::String {
                            DataType::String
                        } else if let DataType::Array(array_type) = obj_type {
                            DataType::Array(array_type)
                        } else {
                            unsafe { unreachable_unchecked() }
                        }
                    }
                    "push" | "sort" | "remove" | "insert" => DataType::Null,
                    "sqrt" | "round" | "floor" => DataType::Float,
                    "abs" => {
                        let obj_type = obj.infer_type(v, ctx, state);
                        if obj_type == DataType::Float {
                            DataType::Float
                        } else if obj_type == DataType::Int {
                            DataType::Int
                        } else {
                            unsafe { unreachable_unchecked() }
                        }
                    }
                    "split" => DataType::Array(Some(Box::from(DataType::String))),
                    "partition" => {
                        let obj_type = obj.infer_type(v, ctx, state);
                        if let DataType::Array(array_type) = obj_type {
                            DataType::Array(Some(Box::from(DataType::Array(array_type))))
                        } else {
                            unsafe { unreachable_unchecked() }
                        }
                    }
                    "get" => {
                        let obj_type = obj.infer_type(v, ctx, state);
                        if let DataType::Map(m) = obj_type {
                            m.1.unwrap_or(DataType::Unknown)
                        } else {
                            unsafe { unreachable_unchecked() }
                        }
                    }
                    "keys" => {
                        let obj_type = obj.infer_type(v, ctx, state);
                        if let DataType::Map(m) = obj_type {
                            DataType::Array(m.0.map(Box::new))
                        } else {
                            unsafe { unreachable_unchecked() }
                        }
                    }
                    "values" => {
                        let obj_type = obj.infer_type(v, ctx, state);
                        if let DataType::Map(m) = obj_type {
                            DataType::Array(m.1.map(Box::new))
                        } else {
                            unsafe { unreachable_unchecked() }
                        }
                    }
                    _ => unsafe { unreachable_unchecked() },
                }
            }
            Self::InlineCondition(_, code, _) => {
                let mut types: Vec<DataType> = Vec::with_capacity(code.len());
                types.push(code[0].infer_type(v, ctx, state));
                for t in &code[0..] {
                    if let Self::ElseIfBlock(_, code) = t {
                        let infered = code[0].infer_type(v, ctx, state);
                        if !types.contains(&infered) {
                            types.push(infered);
                        }
                    } else if let Self::ElseBlock(code) = t {
                        let infered = code[0].infer_type(v, ctx, state);
                        if !types.contains(&infered) {
                            types.push(infered);
                        }
                    }
                }
                DataType::Union(Box::from(types)).check_poly()
            }
            Self::NamespacedRef(path, span) => {
                if let Some((enum_id, _)) = crate::compiler::resolve_enum_variant(path, state) {
                    DataType::Enum(enum_id)
                } else {
                    crate::compiler::compiler_errors::error_enum(
                        "Unknown enum variant",
                        &format!("{} does not name an enum variant", path.join("::")),
                        *span,
                        ctx.file_idx,
                        state.sources,
                    );
                }
            }
            Self::Struct(namespace, _, span) => {
                let struct_name = &namespace[namespace.len() - 1];
                let namespace = &namespace[..(namespace.len() - 1)];
                DataType::Struct(
                    state
                        .namespace
                        .find_struct(namespace, struct_name, *span, ctx.file_idx, state.sources)
                        .unwrap_or_else(|| {
                            error_unknown_struct(struct_name, *span, state.sources, ctx.file_idx);
                        }) as u16,
                )
            }
            Self::AnonymousFunction(args, code, span) => {
                // An anonymous function is hoisted to a synthetic non-capturing
                // top-level function and referred to by its Fn id, exactly like a
                // named function reference. Inference runs many times, so the
                // hoist is keyed by source span and reused: the first encounter
                // registers the function, later ones resolve to the same id.
                let fn_name =
                    format_args!("{ANON_FN_PREFIX}{}:{}", span.start, span.end).to_smolstr();
                if let Some(id) = state.fns.iter().rposition(|f| f.name == fn_name) {
                    return DataType::Fn(id as u16);
                }
                let returns_null = check_if_returns_void(code);
                let mut callees = Vec::new();
                collect_direct_fn_calls(code, &mut callees);
                let id = state.fns.len() as u16;
                state.fns.push(Function {
                    name: fn_name,
                    args: args.iter().map(|a| (a.clone(), None)).collect(),
                    code: Rc::from(code.clone()),
                    impls: Vec::new(),
                    is_recursive: None,
                    returns_null,
                    src_file: ctx.file_idx,
                    return_type_cache: Vec::new(),
                    direct_calls: callees.into_boxed_slice(),
                    name_span: *span,
                });
                state.fn_registers.push(Vec::new());
                DataType::Fn(id)
            }
            _ => unsafe { unreachable_unchecked() },
        }
    }
}
