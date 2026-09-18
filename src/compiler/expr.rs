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
    Int(i64),
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
    /// Condition(condition, code (contains else_if_blocks and potentially
    /// else_block), span, condition_span)
    ///
    /// The condition carries its own span so a condition whose type is not
    /// `bool` is reported against the condition rather than the whole `if`.
    Condition(Box<Self>, Box<[Self]>, Span, Span),
    /// InlineCondition - expression-form if/else, always produces a value, must have an else branch
    InlineCondition(Box<Self>, Box<[Self]>, Span, Span),
    /// ElseIfBlock(condition, code, condition_span)
    ElseIfBlock(Box<Self>, Box<[Self]>, Span),
    ElseBlock(Box<[Self]>),

    /// AnonymousFunction(args, code, span)
    AnonymousFunction(Box<[SmolStr]>, Box<[Self]>, Span),
    // AnonymousFunction(Box<[(SmolStr, SmolStr)]>, SmolStr, Box<[Self]>, Span),
    /// WhileBlock(condition, code, condition_span)
    WhileBlock(Box<Self>, Box<[Self]>, Span),
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
    /// CallValue(callee, args, callee_span, arg_markers)
    ///
    /// A call whose callee is an expression rather than a name: what another
    /// call handed back, an element of a list, a value read out of a map. The
    /// callee's static `Fn` type names the function the same way the type of a
    /// `let`-bound closure does.
    CallValue(Box<Self>, Box<[Self]>, Span, Box<[Span]>),
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
    BitAnd(Box<Self>, Box<Self>, Span, Span),
    BitOr(Box<Self>, Box<Self>, Span, Span),
    BitXor(Box<Self>, Box<Self>, Span, Span),
    Shl(Box<Self>, Box<Self>, Span, Span),
    Shr(Box<Self>, Box<Self>, Span, Span),
    BitNot(Box<Self>, Span, Span),
    Eq(Box<Self>, Box<Self>, Span, Span),
    NotEq(Box<Self>, Box<Self>, Span, Span),
    Sup(Box<Self>, Box<Self>, Span, Span),
    SupEq(Box<Self>, Box<Self>, Span, Span),
    Inf(Box<Self>, Box<Self>, Span, Span),
    InfEq(Box<Self>, Box<Self>, Span, Span),
    BoolAnd(Box<Self>, Box<Self>, Span, Span),
    BoolOr(Box<Self>, Box<Self>, Span, Span),
    BoolNeg(Box<Self>, Span, Span),
    Neg(Box<Self>, Span, Span),
}

impl Expr {
    /// Whether the only way to compile this form is as a value, so the
    /// register it lands in has to be read back.
    ///
    /// [`Expr::compile`] takes a `uses_id` flag saying whether the caller
    /// reads the value produced, and the two answers reach different arms. A
    /// literal, a variable, an operator and a field read have a value arm and
    /// nothing else; a declaration, a loop, an assignment and a control-flow
    /// form have a statement arm and nothing else; a call has both, and its
    /// statement arm is the one that skips the result register. A form in the
    /// first group still compiles as a value when it stands alone as a
    /// statement, and the register is freed straight away.
    ///
    /// The list is the arms of [`Expr::compile_with_code_context`] that open
    /// with `debug_assert!(uses_id)`, and the two are kept in step by hand: an
    /// arm that gains or loses that assertion belongs here or leaves here in
    /// the same change, or a statement of that form stops on the assertion it
    /// no longer matches.
    #[must_use]
    pub const fn is_value_only(&self) -> bool {
        matches!(
            self,
            Self::Float(_)
                | Self::Int(_)
                | Self::Bool(_)
                | Self::Null
                | Self::String(_)
                | Self::Var(_, _)
                | Self::Array(_, _)
                | Self::Map(_, _)
                | Self::Struct(..)
                | Self::NamespacedRef(_, _, _)
                | Self::GetStructField(_, _, _, _)
                | Self::InlineCondition(_, _, _, _)
                | Self::AnonymousFunction(_, _, _)
                | Self::ArrayGetIndex(_, _, _)
                | Self::ArrayGetSlice(..)
                | Self::Mul(..)
                | Self::Div(..)
                | Self::Add(..)
                | Self::Sub(..)
                | Self::Mod(..)
                | Self::Pow(..)
                | Self::BitAnd(..)
                | Self::BitOr(..)
                | Self::BitXor(..)
                | Self::Shl(..)
                | Self::Shr(..)
                | Self::BitNot(_, _, _)
                | Self::Eq(..)
                | Self::NotEq(..)
                | Self::Sup(..)
                | Self::SupEq(..)
                | Self::Inf(..)
                | Self::InfEq(..)
                | Self::BoolAnd(..)
                | Self::BoolOr(..)
                | Self::BoolNeg(_, _, _)
                | Self::Neg(_, _, _)
        )
    }
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
        Expr::BitAnd(_, _, _, _) => "&",
        Expr::BitOr(_, _, _, _) => "|",
        Expr::BitXor(_, _, _, _) => "^^",
        Expr::Shl(_, _, _, _) => "<<",
        Expr::Shr(_, _, _, _) => ">>",
        Expr::BitNot(_, _, _) => "~",
        Expr::Eq(..) => "==",
        Expr::NotEq(..) => "!=",
        Expr::Sup(_, _, _, _) => ">",
        Expr::SupEq(_, _, _, _) => ">=",
        Expr::Inf(_, _, _, _) => "<",
        Expr::InfEq(_, _, _, _) => "<=",
        Expr::BoolAnd(_, _, _, _) => "&&",
        Expr::BoolOr(_, _, _, _) => "||",
        _ => unsafe { unreachable_unchecked() },
    }
}

/// The method name unary minus takes.
///
/// `-` is the one operator with two forms, and both may be defined on one
/// type, so negation cannot share subtraction's symbol as a method name.
/// Subtraction keeps `-` and negation takes this, which no identifier and no
/// other operator spells.
pub const UNARY_MINUS_METHOD: &str = "neg-";

/// The operator symbols a method may be named after, as a diagnostic lists
/// them. Unary minus is written `-` like subtraction; the parameter count
/// tells the two apart.
pub const OPERATOR_SYMBOLS: &str = "+ - * / % ^ & | ^^ << >> == < <= ~";

/// How an operator reaches the method a type defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorForm {
    /// The method named by this operator, operands in the order they are
    /// written.
    Direct,
    /// `a != b` is `a == b` negated: the `==` method, then the bool flipped.
    Negated,
    /// `a > b` is `b < a`: the `<` method with the operands swapped, which is
    /// why the right operand is evaluated first.
    Swapped,
}

/// The method an operator symbol resolves to on a type that defines one, and
/// the form the call takes.
///
/// `!=`, `>` and `>=` have no method of their own: they are derived from `==`,
/// `<` and `<=`, so a type defines three comparisons and gets six operators.
/// `&&`, `||`, `!` and assignment are not overloadable and answer `None`.
#[must_use]
pub fn operator_method(symbol: &str) -> Option<(&'static str, OperatorForm)> {
    Some(match symbol {
        "+" => ("+", OperatorForm::Direct),
        "-" => ("-", OperatorForm::Direct),
        "*" => ("*", OperatorForm::Direct),
        "/" => ("/", OperatorForm::Direct),
        "%" => ("%", OperatorForm::Direct),
        "^" => ("^", OperatorForm::Direct),
        "&" => ("&", OperatorForm::Direct),
        "|" => ("|", OperatorForm::Direct),
        "^^" => ("^^", OperatorForm::Direct),
        "<<" => ("<<", OperatorForm::Direct),
        ">>" => (">>", OperatorForm::Direct),
        "==" => ("==", OperatorForm::Direct),
        "<" => ("<", OperatorForm::Direct),
        "<=" => ("<=", OperatorForm::Direct),
        "~" => ("~", OperatorForm::Direct),
        UNARY_MINUS_METHOD => (UNARY_MINUS_METHOD, OperatorForm::Direct),
        "!=" => ("==", OperatorForm::Negated),
        ">" => ("<", OperatorForm::Swapped),
        ">=" => ("<=", OperatorForm::Swapped),
        _ => return None,
    })
}

/// Whether a method name is one of the operators, so it is declared with an
/// operator symbol rather than an identifier.
#[must_use]
pub fn is_operator_method(name: &str) -> bool {
    matches!(operator_method(name), Some((_, OperatorForm::Direct)))
}

/// Whether an operator method takes one operand rather than two.
#[must_use]
pub fn is_unary_operator_method(name: &str) -> bool {
    name == UNARY_MINUS_METHOD || name == "~"
}

/// Whether an operator method has to hand back a `bool`.
///
/// `==`, `<` and `<=` answer a question about two values, and `!=`, `>` and
/// `>=` are derived from them, so a program reading one of the six as a
/// condition has to get a `bool` whatever the operands are.
#[must_use]
pub fn operator_method_answers_bool(name: &str) -> bool {
    matches!(name, "==" | "<" | "<=")
}

/// The operator method this node reaches on the body's own receiver, for a
/// node whose receiver is `self`.
///
/// A method that applies its own operator to `self` calls itself, and the
/// recursion check works off the names a body calls; without this a method
/// like `fn +(self, o)` returning `self + o` under a guard is inlined into
/// itself forever. The derived forms swap or negate, so `>` looks at the
/// right operand and `!=` at the left.
#[must_use]
pub fn self_operator_method(expr: &Expr) -> Option<&'static str> {
    fn is_self(expr: &Expr) -> bool {
        matches!(expr, Expr::Var(name, _) if name.as_str() == "self")
    }
    let (symbol, receiver) = match expr {
        Expr::Neg(x, _, _) => (UNARY_MINUS_METHOD, &**x),
        Expr::BitNot(x, _, _) => ("~", &**x),
        Expr::Mul(l, r, _, _)
        | Expr::Div(l, r, _, _)
        | Expr::Add(l, r, _, _)
        | Expr::Sub(l, r, _, _)
        | Expr::Mod(l, r, _, _)
        | Expr::Pow(l, r, _, _)
        | Expr::BitAnd(l, r, _, _)
        | Expr::BitOr(l, r, _, _)
        | Expr::BitXor(l, r, _, _)
        | Expr::Shl(l, r, _, _)
        | Expr::Shr(l, r, _, _)
        | Expr::Eq(l, r, _, _)
        | Expr::NotEq(l, r, _, _)
        | Expr::Inf(l, r, _, _)
        | Expr::InfEq(l, r, _, _)
        | Expr::Sup(l, r, _, _)
        | Expr::SupEq(l, r, _, _) => {
            let symbol = symbol_of_expr(expr);
            let (method, form) = operator_method(symbol)?;
            let receiver = if form == OperatorForm::Swapped { r } else { l };
            return is_self(receiver).then_some(method);
        }
        _ => return None,
    };
    let (method, _) = operator_method(symbol)?;
    is_self(receiver).then_some(method)
}

#[must_use]
pub fn code_modifies_variable(var_name: &SmolStr, code: &[Expr]) -> bool {
    code.iter().any(|expr| match expr {
        Expr::VarAssign(n, _, _) => n == var_name,
        Expr::Condition(_, body, _, _)
        | Expr::WhileBlock(_, body, _)
        | Expr::EvalBlock(body)
        | Expr::LoopBlock(body)
        | Expr::InlineCondition(_, body, _, _)
        | Expr::ElseIfBlock(_, body, _)
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

/// Records a name the code read or wrote without binding it first, when the
/// walk is inside an anonymous function.
fn use_free_name(name: &SmolStr, depth: u32, bound: &[SmolStr], out: &mut Vec<SmolStr>) {
    if depth > 0 && !bound.contains(name) && !out.contains(name) {
        out.push(name.clone());
    }
}

/// Walks a block in its own scope: the names it declares go out of scope with
/// it, so a closure written after it does not see them.
fn scan_block_free_names(
    code: &[Expr],
    depth: u32,
    bound: &mut Vec<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    let bound_len = bound.len();
    for expr in code {
        scan_free_names(expr, depth, bound, out);
    }
    bound.truncate(bound_len);
}

/// Collects, in the order they first appear, the names the anonymous
/// functions in `expr` read or write from the scope around them.
///
/// `bound` holds what is in scope at the point being walked, so a name it does
/// not hold comes from further out, which is what a closure captures. `depth`
/// counts the anonymous functions the walk is inside: a name used outside one
/// is resolved where it stands and is not recorded. A call name counts as a
/// use, since the callee may be a closure held in a variable; a name that
/// turns out to name a declared function is dropped by the caller, which keeps
/// only the names that resolve to variables. A nested function declaration has
/// a scope of its own and reads nothing from around it, so its body is
/// skipped.
fn scan_free_names(expr: &Expr, depth: u32, bound: &mut Vec<SmolStr>, out: &mut Vec<SmolStr>) {
    match expr {
        Expr::Var(name, _) => use_free_name(name, depth, bound, out),
        Expr::VarAssign(name, value, _) => {
            scan_free_names(value, depth, bound, out);
            use_free_name(name, depth, bound, out);
        }
        Expr::VarDeclare(name, value) => {
            scan_free_names(value, depth, bound, out);
            bound.push(name.clone());
        }
        Expr::AnonymousFunction(params, body, _) => {
            let bound_len = bound.len();
            bound.extend(params.iter().cloned());
            scan_block_free_names(body, depth + 1, bound, out);
            bound.truncate(bound_len);
        }
        Expr::FunctionCall(args, namespace, _, _, _) => {
            for arg in args {
                scan_free_names(arg, depth, bound, out);
            }
            if let [name] = &**namespace {
                use_free_name(name, depth, bound, out);
            }
        }
        Expr::ObjFunctionCall(obj, args, _, _, _, _, _) => {
            scan_free_names(obj, depth, bound, out);
            for arg in args {
                scan_free_names(arg, depth, bound, out);
            }
        }
        Expr::CallValue(callee, args, _, _) => {
            scan_free_names(callee, depth, bound, out);
            for arg in args {
                scan_free_names(arg, depth, bound, out);
            }
        }
        Expr::Array(items, _) => {
            for item in items {
                scan_free_names(item, depth, bound, out);
            }
        }
        Expr::Map(pairs, _) => {
            for (key, _, value, _) in pairs {
                scan_free_names(key, depth, bound, out);
                scan_free_names(value, depth, bound, out);
            }
        }
        Expr::Struct(_, fields, _, _) => {
            for (_, value, _, _) in fields {
                scan_free_names(value, depth, bound, out);
            }
        }
        Expr::Match(scrutinee, arms, wildcard, _) => {
            scan_free_names(scrutinee, depth, bound, out);
            for (pattern, body) in arms {
                let bound_len = bound.len();
                // An enum pattern binds its payload for the arm body; any
                // other pattern is a value the scrutinee is compared against.
                if let Expr::FunctionCall(binders, _, _, _, _) = pattern {
                    for binder in binders {
                        if let Expr::Var(name, _) = binder {
                            bound.push(name.clone());
                        }
                    }
                } else {
                    scan_free_names(pattern, depth, bound, out);
                }
                scan_block_free_names(body, depth, bound, out);
                bound.truncate(bound_len);
            }
            if let Some(body) = wildcard {
                scan_block_free_names(body, depth, bound, out);
            }
        }
        Expr::Condition(condition, body, ..)
        | Expr::InlineCondition(condition, body, ..)
        | Expr::ElseIfBlock(condition, body, ..)
        | Expr::WhileBlock(condition, body, ..) => {
            scan_free_names(condition, depth, bound, out);
            scan_block_free_names(body, depth, bound, out);
        }
        Expr::ElseBlock(body) | Expr::EvalBlock(body) | Expr::LoopBlock(body) => {
            scan_block_free_names(body, depth, bound, out);
        }
        Expr::ForLoop(var_name, iterated, body, _) => {
            scan_free_names(iterated, depth, bound, out);
            let bound_len = bound.len();
            bound.push(var_name.clone());
            scan_block_free_names(body, depth, bound, out);
            bound.truncate(bound_len);
        }
        Expr::IntForLoop(var_name, start, end, body, _, _) => {
            scan_free_names(start, depth, bound, out);
            scan_free_names(end, depth, bound, out);
            let bound_len = bound.len();
            bound.push(var_name.clone());
            scan_block_free_names(body, depth, bound, out);
            bound.truncate(bound_len);
        }
        Expr::TryCatchBlock(try_code, err_var, catch_code) => {
            scan_block_free_names(try_code, depth, bound, out);
            let bound_len = bound.len();
            bound.push(err_var.clone());
            scan_block_free_names(catch_code, depth, bound, out);
            bound.truncate(bound_len);
        }
        Expr::ReturnVal(value) => {
            if let Some(value) = value.as_ref() {
                scan_free_names(value, depth, bound, out);
            }
        }
        Expr::GetStructField(obj, _, _, _)
        | Expr::BoolNeg(obj, _, _)
        | Expr::Neg(obj, _, _)
        | Expr::BitNot(obj, _, _) => {
            scan_free_names(obj, depth, bound, out);
        }
        Expr::SetStructField(obj, _, value, _, _, _) => {
            scan_free_names(obj, depth, bound, out);
            scan_free_names(value, depth, bound, out);
        }
        Expr::ArrayGetIndex(base, index, _) => {
            scan_free_names(base, depth, bound, out);
            scan_free_names(index, depth, bound, out);
        }
        Expr::ArrayGetSlice(base, start, end, _) | Expr::ArrayModify(base, start, end, _, _) => {
            scan_free_names(base, depth, bound, out);
            scan_free_names(start, depth, bound, out);
            scan_free_names(end, depth, bound, out);
        }
        Expr::Mul(l, r, _, _)
        | Expr::Div(l, r, _, _)
        | Expr::Add(l, r, _, _)
        | Expr::Sub(l, r, _, _)
        | Expr::Mod(l, r, _, _)
        | Expr::Pow(l, r, _, _)
        | Expr::BitAnd(l, r, _, _)
        | Expr::BitOr(l, r, _, _)
        | Expr::BitXor(l, r, _, _)
        | Expr::Shl(l, r, _, _)
        | Expr::Shr(l, r, _, _)
        | Expr::Eq(l, r, _, _)
        | Expr::NotEq(l, r, _, _)
        | Expr::Sup(l, r, _, _)
        | Expr::SupEq(l, r, _, _)
        | Expr::Inf(l, r, _, _)
        | Expr::InfEq(l, r, _, _)
        | Expr::BoolAnd(l, r, _, _)
        | Expr::BoolOr(l, r, _, _) => {
            scan_free_names(l, depth, bound, out);
            scan_free_names(r, depth, bound, out);
        }
        // A literal, a type declaration, an import and a nested function
        // declaration read nothing from the scope around them.
        Expr::Float(_)
        | Expr::Int(_)
        | Expr::Bool(_)
        | Expr::Null
        | Expr::String(_)
        | Expr::NamespacedRef(_, _, _)
        | Expr::StructDeclare(_, _, _, _)
        | Expr::EnumDeclare(_, _, _, _)
        | Expr::FunctionDecl(_, _, _, _, _, _)
        | Expr::ImportDylib(_, _, _)
        | Expr::HostBlock(_, _, _)
        | Expr::ImportFile(_, _, _, _)
        | Expr::Break
        | Expr::Continue => {}
    }
}

/// The names an anonymous function reads or writes from the scope it is
/// written in, in the order they first appear.
///
/// The caller keeps the ones that name a variable there: those are the
/// closure's captures, and the rest name declared functions or nothing at all.
#[must_use]
pub fn closure_free_names(params: &[SmolStr], body: &[Expr]) -> Vec<SmolStr> {
    let mut bound: Vec<SmolStr> = params.to_vec();
    let mut out = Vec::new();
    scan_block_free_names(body, 1, &mut bound, &mut out);
    out
}

/// Whether an anonymous function written anywhere in `code` reads or writes
/// `name` from the scope around it.
///
/// This is what decides whether a variable lives in a cell: one no closure
/// names keeps its register and costs what it costs today.
#[must_use]
pub fn code_captures_variable(name: &SmolStr, code: &[Expr]) -> bool {
    let mut bound = Vec::new();
    let mut out = Vec::new();
    scan_block_free_names(code, 0, &mut bound, &mut out);
    out.contains(name)
}
