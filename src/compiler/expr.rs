use super::type_system::ReturnAnnotation;
use super::type_system::TypeExpr;
use super::type_system::TypeParams;
pub use crate::rt::Span;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::{hint::unreachable_unchecked, rc::Rc};

/// Separator between a type name and a method name in a mangled method symbol.
///
/// `impl` methods lower to per-type-unique free functions named `Type#method`.
/// `#` is not a legal character in a candela identifier (which is
/// `[a-zA-Z_][a-zA-Z0-9_]*`) and is never produced by the lexer, so a mangled
/// name can never collide with a user-written free function or with a method of
/// another type: `Point#len` and `Str#len` are distinct symbols, and both are
/// distinct from a free `fn len`.
pub const METHOD_SEP: char = '#';

/// Builds the mangled free-function symbol an `impl Type { fn method ... }`
/// lowers to.
///
/// Used by the parser when lowering method declarations and by the
/// compiler/type-checker when resolving a `recv.method(...)` call site.
#[must_use]
pub fn mangle_method(type_name: &str, method_name: &str) -> SmolStr {
    format_args!("{type_name}{METHOD_SEP}{method_name}").to_smolstr()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Float(f64),
    Int(i32),
    Bool(bool),
    Null,
    String(SmolStr),
    Var(SmolStr, Span),
    /// Array(contents, [entire_array, elem_spans...])
    Array(Box<[Self]>, Box<[Span]>),
    /// Map(key-value pairs, span)
    Map(Box<[(Self, Span, Self, Span)]>, Span),
    /// Struct(name, fields, span, type_args)
    ///
    /// `type_args` holds the arguments of a generic struct literal
    /// (`Cell<int>{ value: 3 }`) and is empty otherwise.
    Struct(
        Box<[SmolStr]>,
        Box<[(SmolStr, Self, Span, Span)]>,
        Span,
        Box<[TypeExpr]>,
    ),
    /// StructDeclare(name, fields, span, type_params)
    StructDeclare(SmolStr, Box<[(SmolStr, TypeExpr, Span)]>, Span, TypeParams),
    /// EnumDeclare(name, variants: [(variant_name, payload_types, name_span)], span, type_params)
    EnumDeclare(
        SmolStr,
        Box<[(SmolStr, Box<[TypeExpr]>, Span)]>,
        Span,
        TypeParams,
    ),
    /// NamespacedRef(path, span, type_args)
    ///
    /// A bare namespaced identifier (`Color::Red`) with no call parentheses or
    /// struct braces. The only such form candela has is a nullary enum-variant
    /// construction; the compiler resolves it against the enum registry.
    /// `type_args` names the instantiation of a generic enum
    /// (`Slot<int>::Empty`) and is empty otherwise.
    NamespacedRef(Box<[SmolStr]>, Span, Box<[TypeExpr]>),
    /// Match(scrutinee, arms: [(pattern_expr, body)], wildcard_body, span)
    ///
    /// The arm patterns are parsed as ordinary expressions; the compiler picks
    /// the lowering by the scrutinee's static type. For an enum scrutinee each
    /// pattern is a variant pattern (`Circle(r)` binds the payload); otherwise
    /// each pattern is an equality test against the scrutinee.
    Match(
        Box<Self>,
        Box<[(Self, Box<[Self]>)]>,
        Option<Box<[Self]>>,
        Span,
    ),
    /// GetStructField(struct_expr, field, struct_span, field_span, value_span)
    GetStructField(Box<Self>, SmolStr, Span, Span),
    /// SetStructField(struct_expr, field, new_expr, struct_span, field_span, value_span)
    SetStructField(Box<Self>, SmolStr, Box<Self>, Span, Span, Span),
    /// VarDeclare(name, value),
    VarDeclare(SmolStr, Box<Self>),
    /// VarDeclare(name, value, start, end)
    VarAssign(SmolStr, Box<Self>, Span),
    /// Condition(condition, code (contains else_if_blocks and potentially else_block), start, end)
    Condition(Box<Self>, Box<[Self]>, Span),
    /// InlineCondition - expression-form if/else, always produces a value, must have an else branch
    InlineCondition(Box<Self>, Box<[Self]>, Span),
    ElseIfBlock(Box<Self>, Box<[Self]>),
    ElseBlock(Box<[Self]>),

    /// AnonymousFunction(args, code, span)
    AnonymousFunction(Box<[SmolStr]>, Box<[Self]>, Span),
    // AnonymousFunction(Box<[(SmolStr, SmolStr)]>, SmolStr, Box<[Self]>, Span),
    WhileBlock(Box<Self>, Box<[Self]>),
    /// FunctionCall(args, (optional namespace + name), span, (arg_start,arg_end), type_args)
    ///
    /// `type_args` holds the type arguments of a call written with them
    /// (`first<int>(nums)`) and is empty otherwise.
    FunctionCall(
        Box<[Self]>,
        Box<[SmolStr]>,
        Span,
        Box<[Span]>,
        Box<[TypeExpr]>,
    ),
    /// ObjFunctionCall(obj, args, namespace, obj_span, fn_span, arg_markers, type_args)
    ///
    /// `type_args` holds the type arguments a method call is written with
    /// (`b.tagged<string>("hi")`), which bind the method's own type parameters,
    /// and is empty otherwise.
    ObjFunctionCall(
        // Will be removed soon.
        Box<Self>,
        Box<[Self]>,
        Box<[SmolStr]>,
        // obj_span
        Span,
        // fn_span
        Span,
        Box<[Span]>,
        Box<[TypeExpr]>,
    ),
    /// FunctionDecl(name, args, code, name_span, return_type, type_params)
    ///
    /// `return_type` carries the optional `-> Type` annotation together with the
    /// span of the annotation itself, which is what a mismatch between the
    /// declared and the returned type is reported against. `type_params` names
    /// the type parameters of a generic function (`fn first<T>`) and is empty
    /// otherwise.
    FunctionDecl(
        SmolStr,
        Box<[(SmolStr, Option<TypeExpr>)]>,
        Rc<[Self]>,
        Span,
        ReturnAnnotation,
        TypeParams,
    ),

    ReturnVal(Box<Option<Self>>),

    ArrayGetIndex(Box<Self>, Box<Self>, Span),
    /// ArrayGetSlice(array, range_start, range_end, span)
    ArrayGetSlice(Box<Self>, Box<Self>, Box<Self>, Span),
    ArrayModify(Box<Self>, Box<Self>, Box<Self>, Span, Span),

    /// ForLoop(loop_var_name, loop_array+code, obj_markers)
    ForLoop(SmolStr, Box<Self>, Box<[Self]>, Span),
    /// IntForLoop(loop_var_name, first_elem, final_elem, code)
    IntForLoop(SmolStr, Box<Self>, Box<Self>, Box<[Self]>, Span, Span),
    /// ImportDylib(lib_path, [(fn_name, fn_args, fn_return_type, fn_name_span)], (start, end))
    ImportDylib(
        SmolStr,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        Span,
    ),

    /// HostBlock(namespace, [(fn_name, fn_args, fn_return_type, fn_name_span)], (start, end))
    ///
    /// Declares a host namespace whose functions are backed by Rust closures
    /// registered on the embedding [`crate::Engine`]. Mirrors [`Self::ImportDylib`]
    /// but dispatches to a registered closure instead of a C symbol.
    HostBlock(
        SmolStr,
        Box<[(SmolStr, Box<[TypeExpr]>, TypeExpr, Span)]>,
        Span,
    ),

    /// ImportFile(path, alias, is_logical, (start, end))
    ///
    /// `is_logical` marks a library import (`import "std/string";`, an
    /// extensionless path): the resolver maps it straight to the shipped
    /// library directory and never looks source-relative. A `.cdl` file import
    /// (`import "./local.cdl";`) has `is_logical` false and resolves
    /// source-relative first. `alias` is `Some` for `import ... as name;`,
    /// which binds the module under `name::`; a bare import (`None`) merges
    /// the module's symbols into the importing file's scope.
    ImportFile(SmolStr, Option<SmolStr>, bool, Span),

    Break,
    Continue,

    EvalBlock(Box<[Self]>),
    LoopBlock(Box<[Self]>),

    /// TryCatchBlock(try_code, err_var, catch_code)
    TryCatchBlock(Box<[Self]>, SmolStr, Box<[Self]>),

    Mul(Box<Self>, Box<Self>, Span, Span),
    Div(Box<Self>, Box<Self>, Span, Span),
    Add(Box<Self>, Box<Self>, Span, Span),
    Sub(Box<Self>, Box<Self>, Span, Span),
    Mod(Box<Self>, Box<Self>, Span, Span),
    Pow(Box<Self>, Box<Self>, Span, Span),
    Eq(Box<Self>, Box<Self>),
    NotEq(Box<Self>, Box<Self>),
    Sup(Box<Self>, Box<Self>, Span, Span),
    SupEq(Box<Self>, Box<Self>, Span, Span),
    Inf(Box<Self>, Box<Self>, Span, Span),
    InfEq(Box<Self>, Box<Self>, Span, Span),
    BoolAnd(Box<Self>, Box<Self>, Span, Span),
    BoolOr(Box<Self>, Box<Self>, Span, Span),
    BoolNeg(Box<Self>, Span, Span),
    Neg(Box<Self>, Span, Span),
}

#[cold]
#[inline(never)]
#[must_use]
pub const fn symbol_of_expr(expr: &Expr) -> &'static str {
    match expr {
        Expr::Mul(_, _, _, _) => "*",
        Expr::Div(_, _, _, _) => "/",
        Expr::Add(_, _, _, _) => "+",
        Expr::Sub(_, _, _, _) | Expr::Neg(_, _, _) => "-",
        Expr::Mod(_, _, _, _) => "%",
        Expr::Pow(_, _, _, _) => "^",
        Expr::Eq(_, _) => "==",
        Expr::NotEq(_, _) => "!=",
        Expr::Sup(_, _, _, _) => ">",
        Expr::SupEq(_, _, _, _) => ">=",
        Expr::Inf(_, _, _, _) => "<",
        Expr::InfEq(_, _, _, _) => "<=",
        Expr::BoolAnd(_, _, _, _) => "&&",
        Expr::BoolOr(_, _, _, _) => "||",
        _ => unsafe { unreachable_unchecked() },
    }
}

#[must_use]
pub fn code_modifies_variable(var_name: &SmolStr, code: &[Expr]) -> bool {
    code.iter().any(|expr| match expr {
        Expr::VarAssign(n, _, _) => n == var_name,
        Expr::Condition(_, body, _)
        | Expr::WhileBlock(_, body)
        | Expr::EvalBlock(body)
        | Expr::LoopBlock(body)
        | Expr::InlineCondition(_, body, _)
        | Expr::ElseIfBlock(_, body)
        | Expr::ElseBlock(body)
        | Expr::ForLoop(_, _, body, _)
        | Expr::IntForLoop(_, _, _, body, _, _) => code_modifies_variable(var_name, body),
        Expr::Match(_, arms, wildcard, _) => {
            arms.iter()
                .any(|(_, body)| code_modifies_variable(var_name, body))
                || wildcard
                    .as_ref()
                    .is_some_and(|body| code_modifies_variable(var_name, body))
        }
        _ => false,
    })
}

#[must_use]
pub fn var_assign(target: Expr, value: Expr, expr_span: Span, value_span: Span) -> Expr {
    if let Expr::Var(n, s) = target {
        Expr::VarAssign(n, Box::from(value), s)
    } else if let Expr::ArrayGetIndex(base, idx, _) = target {
        Expr::ArrayModify(base, idx, Box::from(value), expr_span, value_span)
    } else if let Expr::GetStructField(obj, field, obj_span, field_span) = target {
        Expr::SetStructField(
            obj,
            field,
            Box::from(value),
            obj_span,
            field_span,
            value_span,
        )
    } else {
        unsafe { unreachable_unchecked() }
    }
}
