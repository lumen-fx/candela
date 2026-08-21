use super::expr::Expr;
use super::expr::Span;
use super::expr::mangle_method;
use super::expr::symbol_of_expr;
use crate::compiler::Namespace;
use crate::compiler::SymbolKind;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::EnumType;
use crate::compiler::compiler_data::EnumVariant;
use crate::compiler::compiler_data::FnGenerics;
use crate::compiler::compiler_data::FnSignature;
use crate::compiler::compiler_data::Function;
use crate::compiler::compiler_data::Source;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Struct;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::error_instantiation_depth;
use crate::compiler::compiler_errors::error_invalid_type;
use crate::compiler::compiler_errors::error_op;
use crate::compiler::compiler_errors::error_struct_unknown_field;
use crate::compiler::compiler_errors::error_type_arg_count;
use crate::compiler::compiler_errors::error_type_args_on_plain_function;
use crate::compiler::compiler_errors::error_type_args_on_plain_type;
use crate::compiler::compiler_errors::error_unknown_function;
use crate::compiler::compiler_errors::error_unknown_function_in_namespace;
use crate::compiler::compiler_errors::error_unknown_struct;
use crate::compiler::compiler_errors::error_unknown_type;
use crate::compiler::compiler_errors::error_unknown_type_param;
use crate::compiler::compiler_errors::error_unknown_type_with_namespace;
use crate::compiler::compiler_errors::error_unknown_variable;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::cell::RefCell;
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

/// A declared `-> Type` return annotation with the span it was written at.
///
/// `None` leaves the return type inferred. Boxed because a declaration is
/// carried inside [`Expr`], where the annotation is the rarest field.
pub type ReturnAnnotation = Option<Box<(TypeExpr, Span)>>;

/// The type parameters a declaration introduces (`struct Cell<T>`), in the
/// order they were written. Empty for a declaration that takes none.
pub type TypeParams = Box<[SmolStr]>;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TypeExpr {
    Identifier(SmolStr, Span),
    NamespacedIdentifier(Box<[SmolStr]>, Span),
    /// A generic type applied to its arguments, `Cell<int>`.
    Generic(Box<GenericType>),
    Array(Box<Self>),
    Map(Box<Self>, Box<Self>),
    Union(Box<[Self]>),
}

/// A generic type and the arguments it is applied to. Boxed inside
/// [`TypeExpr`], which every declaration carries by value.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct GenericType {
    pub name: SmolStr,
    pub args: Box<[TypeExpr]>,
    pub span: Span,
}

impl TypeExpr {
    /// Whether this type mentions any of `params`.
    ///
    /// An annotation that does is left un-pinned when the call site supplies no
    /// type arguments: candela infers such a parameter from the argument, the
    /// same as an un-annotated one.
    #[must_use]
    pub fn mentions_any(&self, params: &[SmolStr]) -> bool {
        match self {
            Self::Identifier(name, _) => params.contains(name),
            Self::NamespacedIdentifier(_, _) => false,
            Self::Generic(generic) => {
                params.contains(&generic.name)
                    || generic.args.iter().any(|a| a.mentions_any(params))
            }
            Self::Array(inner) => inner.mentions_any(params),
            Self::Map(k, val) => k.mentions_any(params) || val.mentions_any(params),
            Self::Union(poly) => poly.iter().any(|t| t.mentions_any(params)),
        }
    }

    #[must_use]
    pub fn to_datatype(&self, ctx: &mut TypeCtx<'_>) -> DataType {
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
                    if let Some(bound) = ctx.generics.bound(struct_name) {
                        bound
                    } else if let Some(struct_id) = ctx.namespace.find_struct(
                        &[],
                        struct_name,
                        *span,
                        ctx.file_idx,
                        ctx.sources,
                    ) {
                        DataType::Struct(struct_id as u16)
                    } else if let Some(enum_id) = ctx.namespace.find_enum(&[], struct_name) {
                        DataType::Enum(enum_id as u16)
                    } else if ctx.generics.is_template(struct_name) {
                        // A generic type named without its arguments is the
                        // dynamic slot: candela never makes a missing type
                        // argument an error.
                        DataType::Unknown
                    } else if ctx.generics.in_generic_body() {
                        error_unknown_type_param(
                            *span,
                            ctx.file_idx,
                            struct_name,
                            &ctx.generics.bound_names(),
                            ctx.sources,
                        );
                    } else {
                        error_unknown_type(
                            *span,
                            ctx.file_idx,
                            struct_name,
                            ctx.sources,
                            ctx.namespace,
                        );
                    }
                }
            },
            Self::NamespacedIdentifier(s, span) => {
                if let Some(struct_id) = ctx.namespace.find_struct(
                    &s[..s.len() - 1],
                    unsafe { s.last().unwrap_unchecked() },
                    *span,
                    ctx.file_idx,
                    ctx.sources,
                ) {
                    DataType::Struct(struct_id as u16)
                } else if let Some(enum_id) = ctx
                    .namespace
                    .find_enum(&s[..s.len() - 1], unsafe { s.last().unwrap_unchecked() })
                {
                    DataType::Enum(enum_id as u16)
                } else {
                    cold_path();
                    error_unknown_type_with_namespace(
                        *span,
                        ctx.file_idx,
                        unsafe { s.last().unwrap_unchecked() },
                        ctx.sources,
                        ctx.namespace,
                        &s[..s.len() - 1],
                    )
                }
            }
            Self::Generic(generic) => {
                let args: Vec<DataType> = generic.args.iter().map(|a| a.to_datatype(ctx)).collect();
                instantiate(&generic.name, &args, generic.span, ctx)
            }
            Self::Array(inner_t) => DataType::Array(Some(Box::new(inner_t.to_datatype(ctx)))),
            Self::Map(k_t, v_t) => DataType::Map(Box::from((
                Some(k_t.to_datatype(ctx)),
                Some(v_t.to_datatype(ctx)),
            ))),
            Self::Union(poly) => {
                DataType::Union(poly.iter().map(|t| t.to_datatype(ctx)).collect()).check_poly()
            }
        }
    }
}

/// How deep one generic type may be instantiated inside another before the
/// compiler stops. A type whose own fields name a deeper instantiation of
/// itself (`struct L<T> { next: L<L<T>> }`) has no finite set of
/// instantiations, and this is where that is reported instead of hanging.
const MAX_INSTANTIATION_DEPTH: u32 = 32;

/// What a declaration of a generic type keeps: its parameters and the field or
/// variant types they appear in, unresolved. Substituting the parameters and
/// resolving the result is what [`instantiate`] does.
#[derive(Debug)]
struct TypeTemplate {
    name: SmolStr,
    params: TypeParams,
    /// File the declaration was written in; its field types resolve against
    /// that file's namespace whatever file the instantiation is written in.
    file_idx: u16,
    body: TemplateBody,
}

#[derive(Debug, Clone)]
enum TemplateBody {
    Struct(Box<[(SmolStr, TypeExpr, Span)]>),
    Enum(Box<[(SmolStr, Box<[TypeExpr]>, Span)]>),
}

/// An `impl` block written against a generic type, kept until the type is
/// instantiated.
///
/// `args` is the header as written: `impl Cell<T>` applies to every
/// instantiation and binds `T`, `impl Cell<int>` applies to that one.
#[derive(Debug)]
pub struct ImplTemplate {
    pub type_name: SmolStr,
    pub args: Box<[TypeExpr]>,
    /// The methods, as [`Expr::FunctionDecl`] carrying the plain method name.
    /// The name is mangled per instantiated type when the block is lowered.
    pub methods: Box<[Expr]>,
    pub file_idx: u16,
    pub span: Span,
}

/// The generic declarations of a program, the instantiations made from them,
/// and the type parameters bound while a body is being compiled.
#[derive(Debug, Default)]
pub struct Generics {
    templates: Vec<TypeTemplate>,
    impls: Vec<ImplTemplate>,
    /// Instantiations by rendered name, so `Cell<int>` written twice is one
    /// struct. `<` and `>` cannot occur in an identifier, so a rendered name
    /// never collides with a user-written one.
    instantiations: Vec<(SmolStr, DataType)>,
    /// Type parameters bound for the body being compiled. Only the top frame is
    /// in scope: the parameters of a function never reach the body of a
    /// function it calls.
    bindings: Vec<Box<[(SmolStr, DataType)]>>,
    /// Each file's namespace, so a template resolves its own types where it was
    /// declared.
    file_namespaces: FxHashMap<u16, Namespace>,
    depth: u32,
}

impl Generics {
    /// The type this name is currently bound to, if it names a type parameter
    /// of the body being compiled.
    #[must_use]
    pub fn bound(&self, name: &str) -> Option<DataType> {
        self.bindings
            .last()?
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, t)| t.clone())
    }

    /// Whether any type parameter is in scope, which is what makes an unknown
    /// type name worth reporting as a type parameter rather than a type.
    #[must_use]
    fn in_generic_body(&self) -> bool {
        self.bindings.last().is_some_and(|frame| !frame.is_empty())
    }

    fn bound_names(&self) -> Vec<SmolStr> {
        self.bindings
            .last()
            .map(|frame| frame.iter().map(|(p, _)| p.clone()).collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn is_template(&self, name: &str) -> bool {
        self.templates.iter().any(|t| t.name == name)
    }

    #[must_use]
    pub fn params_of(&self, name: &str) -> Option<&[SmolStr]> {
        self.templates
            .iter()
            .rfind(|t| t.name == name)
            .map(|t| &*t.params)
    }

    /// For each type parameter of a struct template, the field declared with
    /// that parameter as its type. A literal written without type arguments
    /// takes each parameter from the value in that field.
    #[must_use]
    pub fn param_fields(&self, name: &str) -> Vec<Option<SmolStr>> {
        let Some(template) = self.templates.iter().rfind(|t| t.name == name) else {
            return Vec::new();
        };
        let TemplateBody::Struct(fields) = &template.body else {
            return vec![None; template.params.len()];
        };
        template
            .params
            .iter()
            .map(|param| {
                fields
                    .iter()
                    .find(|(_, field_type, _)| {
                        matches!(field_type, TypeExpr::Identifier(t, _) if t == param)
                    })
                    .map(|(field_name, _, _)| field_name.clone())
            })
            .collect()
    }

    pub fn add_struct_template(
        &mut self,
        name: SmolStr,
        params: TypeParams,
        file_idx: u16,
        fields: Box<[(SmolStr, TypeExpr, Span)]>,
    ) {
        self.templates.push(TypeTemplate {
            name,
            params,
            file_idx,
            body: TemplateBody::Struct(fields),
        });
    }

    pub fn add_enum_template(
        &mut self,
        name: SmolStr,
        params: TypeParams,
        file_idx: u16,
        variants: Box<[(SmolStr, Box<[TypeExpr]>, Span)]>,
    ) {
        self.templates.push(TypeTemplate {
            name,
            params,
            file_idx,
            body: TemplateBody::Enum(variants),
        });
    }

    /// Records the `impl` blocks a file declared against a generic type,
    /// stamping each with the file it came from.
    pub fn add_impls(&mut self, impls: Vec<ImplTemplate>, file_idx: u16) {
        self.impls.extend(impls.into_iter().map(|mut block| {
            block.file_idx = file_idx;
            block
        }));
    }

    pub fn set_file_namespaces(&mut self, file_namespaces: &FxHashMap<u16, Namespace>) {
        self.file_namespaces.clone_from(file_namespaces);
    }

    /// The scope a file had once it was parsed, for resolving a type written in
    /// that file from somewhere else.
    #[must_use]
    pub fn file_namespace(&self, file_idx: u16) -> Namespace {
        self.file_namespaces
            .get(&file_idx)
            .cloned()
            .unwrap_or_default()
    }

    /// Binds `frame` for the body about to be compiled. Every body pushes a
    /// frame, an empty one when it has no type parameters, so the caller's
    /// parameters do not resolve inside it.
    pub fn push_bindings(&mut self, frame: Box<[(SmolStr, DataType)]>) {
        self.bindings.push(frame);
    }

    pub fn pop_bindings(&mut self) {
        self.bindings.pop();
    }
}

/// What resolving a [`TypeExpr`] needs: the scope the type is written in, and
/// the registries an instantiation adds to.
pub struct TypeCtx<'a> {
    pub file_idx: u16,
    pub namespace: &'a Namespace,
    pub sources: &'a [Source],
    pub structs: &'a mut Vec<Struct>,
    pub enums: &'a mut Vec<EnumType>,
    pub fns: &'a mut Vec<Function>,
    pub fn_registers: &'a mut Vec<Vec<u16>>,
    pub generics: &'a mut Generics,
}

impl TypeCtx<'_> {
    /// The same registries, resolving names in another file's scope. A
    /// declaration resolves its own types where it was written, whatever file
    /// the use is written in.
    pub const fn reborrow<'b>(
        &'b mut self,
        file_idx: u16,
        namespace: &'b Namespace,
    ) -> TypeCtx<'b> {
        TypeCtx {
            file_idx,
            namespace,
            sources: self.sources,
            structs: self.structs,
            enums: self.enums,
            fns: self.fns,
            fn_registers: self.fn_registers,
            generics: self.generics,
        }
    }
}

/// Renders `t` as the name an instantiation is registered under.
///
/// The name comes from the resolved type, never from the source text, so two
/// spellings of one type give one instantiation and two types can never give
/// the same name.
#[must_use]
fn render_type(t: &DataType, structs: &[Struct], enums: &[EnumType]) -> SmolStr {
    match t {
        DataType::Int => SmolStr::new_static("int"),
        DataType::Float => SmolStr::new_static("float"),
        DataType::Bool => SmolStr::new_static("bool"),
        DataType::String => SmolStr::new_static("string"),
        DataType::Null => SmolStr::new_static("null"),
        DataType::Unknown => SmolStr::new_static("any"),
        DataType::Array(inner) => match inner {
            Some(inner) => format_args!("{}[]", render_type(inner, structs, enums)).to_smolstr(),
            None => SmolStr::new_static("any[]"),
        },
        DataType::Map(m) => format_args!(
            "{{{}: {}}}",
            m.0.as_ref().map_or_else(
                || SmolStr::new_static("any"),
                |k| render_type(k, structs, enums)
            ),
            m.1.as_ref().map_or_else(
                || SmolStr::new_static("any"),
                |val| render_type(val, structs, enums)
            )
        )
        .to_smolstr(),
        DataType::Union(poly) => poly
            .iter()
            .map(|t| render_type(t, structs, enums))
            .collect::<Vec<SmolStr>>()
            .join("|")
            .into(),
        DataType::Struct(id) => structs[*id as usize].name.clone(),
        DataType::Enum(id) => enums[*id as usize].name.clone(),
        DataType::Fn(id) => format_args!("fn:{id}").to_smolstr(),
    }
}

/// The name a generic type is registered under once its arguments are known.
#[must_use]
fn render_instantiation(
    base: &str,
    args: &[DataType],
    structs: &[Struct],
    enums: &[EnumType],
) -> SmolStr {
    let rendered = args
        .iter()
        .map(|a| render_type(a, structs, enums))
        .collect::<Vec<SmolStr>>()
        .join(", ");
    format_args!("{base}<{rendered}>").to_smolstr()
}

/// Resolves `base<args>` to an ordinary struct or enum, registering it the
/// first time it is asked for.
///
/// Past this point nothing generic is left: the instantiation is a concrete
/// type with concrete field types, and every later stage (field access, method
/// dispatch, the artifact codec, the VM) treats it like any other.
pub fn instantiate(base: &str, args: &[DataType], span: Span, ctx: &mut TypeCtx<'_>) -> DataType {
    let name = render_instantiation(base, args, ctx.structs, ctx.enums);
    if let Some((_, t)) = ctx.generics.instantiations.iter().find(|(n, _)| n == &name) {
        return t.clone();
    }
    let Some(template_idx) = ctx.generics.templates.iter().rposition(|t| t.name == base) else {
        if ctx
            .namespace
            .find_struct(&[], base, span, ctx.file_idx, ctx.sources)
            .is_some()
            || ctx.namespace.find_enum(&[], base).is_some()
        {
            error_type_args_on_plain_type(span, ctx.file_idx, base, ctx.sources);
        }
        error_unknown_type(span, ctx.file_idx, base, ctx.sources, ctx.namespace);
    };
    if ctx.generics.templates[template_idx].params.len() != args.len() {
        error_type_arg_count(
            span,
            ctx.file_idx,
            base,
            ctx.generics.templates[template_idx].params.len(),
            args.len(),
            ctx.sources,
        );
    }
    if ctx.generics.depth >= MAX_INSTANTIATION_DEPTH {
        error_instantiation_depth(span, ctx.file_idx, base, ctx.sources);
    }

    let template_file = ctx.generics.templates[template_idx].file_idx;
    let template_namespace = ctx
        .generics
        .file_namespaces
        .get(&template_file)
        .cloned()
        .unwrap_or_default();
    let frame: Box<[(SmolStr, DataType)]> = ctx.generics.templates[template_idx]
        .params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect();

    // The instantiation is registered and cached before its own field types are
    // resolved, so a type whose fields name it resolves to the type being built
    // instead of instantiating it again.
    let is_struct = matches!(
        ctx.generics.templates[template_idx].body,
        TemplateBody::Struct(_)
    );
    let instantiated = if is_struct {
        let id = ctx.structs.len() as u16;
        ctx.structs.push(Struct {
            name: name.clone(),
            fields: Box::from([]),
            id,
            name_span: span,
        });
        DataType::Struct(id)
    } else {
        let id = ctx.enums.len() as u16;
        ctx.enums.push(EnumType {
            name: name.clone(),
            variants: Box::from([]),
            id,
            name_span: span,
        });
        DataType::Enum(id)
    };
    ctx.generics
        .instantiations
        .push((name.clone(), instantiated.clone()));

    let body = ctx.generics.templates[template_idx].body.clone();
    ctx.generics.depth += 1;
    ctx.generics.push_bindings(frame.clone());
    {
        let mut inner = ctx.reborrow(template_file, &template_namespace);
        match body {
            TemplateBody::Struct(fields) => {
                let resolved = fields
                    .iter()
                    .map(|(field_name, field_type, field_span)| {
                        (
                            field_name.clone(),
                            field_type.to_datatype(&mut inner),
                            *field_span,
                        )
                    })
                    .collect();
                if let DataType::Struct(id) = instantiated {
                    inner.structs[id as usize].fields = resolved;
                }
            }
            TemplateBody::Enum(variants) => {
                let resolved = variants
                    .iter()
                    .map(|(variant_name, payload, name_span)| EnumVariant {
                        name: variant_name.clone(),
                        payload: payload.iter().map(|t| t.to_datatype(&mut inner)).collect(),
                        name_span: *name_span,
                    })
                    .collect();
                if let DataType::Enum(id) = instantiated {
                    inner.enums[id as usize].variants = resolved;
                }
            }
        }
    }
    ctx.generics.pop_bindings();

    lower_impls(base, args, &name, &frame, ctx);
    ctx.generics.depth -= 1;

    instantiated
}

/// Lowers every `impl` block that applies to a freshly instantiated type.
///
/// A generic block (`impl Cell<T>`) binds its parameters from the
/// instantiation's arguments; a concrete one (`impl Cell<int>`) applies only
/// when the arguments match. Each method becomes an ordinary free function
/// named `Cell<int>#get`, exactly as a method on a plain type does.
fn lower_impls(
    base: &str,
    args: &[DataType],
    type_name: &SmolStr,
    type_frame: &[(SmolStr, DataType)],
    ctx: &mut TypeCtx<'_>,
) {
    let applicable: Vec<usize> = ctx
        .generics
        .impls
        .iter()
        .enumerate()
        .filter(|(_, block)| block.type_name == base && block.args.len() == args.len())
        .map(|(i, _)| i)
        .collect();
    for idx in applicable {
        let impl_file = ctx.generics.impls[idx].file_idx;
        let impl_namespace = ctx
            .generics
            .file_namespaces
            .get(&impl_file)
            .cloned()
            .unwrap_or_default();
        let header = ctx.generics.impls[idx].args.clone();
        let mut frame: Vec<(SmolStr, DataType)> = Vec::with_capacity(header.len());
        let mut applies = true;
        for (header_arg, arg) in header.iter().zip(args) {
            match header_arg {
                // A bare name that is not a type of its own is a parameter the
                // header introduces, bound to whatever this instantiation
                // passes. Anything else is a concrete type the header pins, and
                // the block applies only when the argument is that type.
                TypeExpr::Identifier(pname, _)
                    if is_type_parameter(pname, &impl_namespace, ctx) =>
                {
                    frame.push((pname.clone(), arg.clone()));
                }
                other => {
                    let mut inner = ctx.reborrow(impl_file, &impl_namespace);
                    inner.generics.push_bindings(Box::from(type_frame));
                    let pinned = other.to_datatype(&mut inner);
                    ctx.generics.pop_bindings();
                    if &pinned != arg {
                        applies = false;
                        break;
                    }
                }
            }
        }
        if !applies {
            continue;
        }
        let methods = ctx.generics.impls[idx].methods.clone();
        let bindings: Box<[(SmolStr, DataType)]> = Box::from(frame);
        for method in methods {
            lower_method(
                &method,
                type_name,
                &bindings,
                impl_file,
                &impl_namespace,
                ctx,
            );
        }
    }
}

/// Whether a name written as a type argument in an `impl` header introduces a
/// type parameter rather than naming a type.
fn is_type_parameter(name: &SmolStr, namespace: &Namespace, ctx: &TypeCtx<'_>) -> bool {
    if matches!(
        name.as_str(),
        "int" | "float" | "bool" | "string" | "null" | "any"
    ) {
        return false;
    }
    !ctx.generics.is_template(name)
        && !namespace.symbols.iter().any(|(symbol, kind)| {
            symbol == name && matches!(kind, SymbolKind::Struct(_) | SymbolKind::Enum(_))
        })
}

/// Registers one method of an instantiated `impl` block as the mangled free
/// function its call sites resolve to.
fn lower_method(
    method: &Expr,
    type_name: &SmolStr,
    bindings: &[(SmolStr, DataType)],
    file_idx: u16,
    namespace: &Namespace,
    ctx: &mut TypeCtx<'_>,
) {
    let Expr::FunctionDecl(method_name, args, code, name_span, return_type, params) = method else {
        return;
    };
    let mangled = mangle_method(type_name, method_name);
    if let Some(existing) = ctx.fns.iter().find(|f| f.name == mangled) {
        crate::compiler::compiler_errors::error_function_already_defined(
            existing,
            *name_span,
            file_idx,
            ctx.sources,
        );
    }
    let mut inner = ctx.reborrow(file_idx, namespace);
    inner.generics.push_bindings(Box::from(bindings));
    let resolved_args: Box<[(SmolStr, Option<DataType>)]> = args
        .iter()
        .map(|(arg_name, arg_type)| {
            (
                arg_name.clone(),
                arg_type
                    .as_ref()
                    .filter(|t| !t.mentions_any(params))
                    .map(|t| t.to_datatype(&mut inner)),
            )
        })
        .collect();
    let resolved_return = return_type
        .as_deref()
        .filter(|(t, _)| !t.mentions_any(params))
        .map(|(t, t_span)| (t.to_datatype(&mut inner), *t_span));
    ctx.generics.pop_bindings();

    let mut callees = Vec::new();
    collect_direct_fn_calls(code, &mut callees);
    ctx.fns.push(Function {
        name: mangled,
        args: resolved_args,
        code: Rc::clone(code),
        impls: Vec::new(),
        is_recursive: None,
        returns_null: check_if_returns_void(code),
        src_file: file_idx,
        return_type_cache: Vec::new(),
        direct_calls: callees.into_boxed_slice(),
        name_span: *name_span,
        return_type: resolved_return,
        generics: Some(Box::new(FnGenerics {
            params: params.clone(),
            arg_types: args.iter().map(|(_, t)| t.clone()).collect(),
            return_type: return_type.clone(),
            bindings: Box::from(bindings),
            file_idx,
        })),
    });
    ctx.fn_registers.push(Vec::new());
}

/// Resolves the struct a literal names, instantiating the generic type when the
/// literal carries type arguments.
///
/// A literal of a generic type written without them takes each type parameter
/// from the value in the field declared with that parameter as its type, so
/// `Cell{ value: 3 }` is a `Cell<int>`. A parameter no field pins is `any`.
pub fn struct_literal_id(
    namespace: &[SmolStr],
    fields: &[(SmolStr, Expr, Span, Span)],
    type_args: &[TypeExpr],
    span: Span,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> u16 {
    let name = namespace[namespace.len() - 1].clone();
    let path = &namespace[..namespace.len() - 1];
    if !type_args.is_empty() {
        let args = resolve_type_args(type_args, ctx, state);
        return instantiated_struct_id(&name, &args, span, ctx, state);
    }
    if path.is_empty() && state.generics.is_template(&name) {
        let param_fields = state.generics.param_fields(&name);
        let mut args: Vec<DataType> = Vec::with_capacity(param_fields.len());
        for field_name in param_fields {
            args.push(
                field_name
                    .and_then(|f| fields.iter().find(|(n, _, _, _)| *n == f))
                    .map_or(DataType::Unknown, |(_, value, _, _)| {
                        value.infer_type(v, ctx, state)
                    }),
            );
        }
        return instantiated_struct_id(&name, &args, span, ctx, state);
    }
    state
        .namespace
        .find_struct(path, &name, span, ctx.file_idx, state.sources)
        .unwrap_or_else(|| {
            error_unknown_struct(&name, span, state.sources, ctx.file_idx);
        }) as u16
}

fn instantiated_struct_id(
    name: &SmolStr,
    args: &[DataType],
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> u16 {
    match instantiate(name, args, span, &mut state.type_ctx(ctx.file_idx)) {
        DataType::Struct(id) => id,
        _ => error_unknown_struct(name, span, state.sources, ctx.file_idx),
    }
}

/// Resolves each type argument of a call, struct literal or variant path in the
/// scope it was written in.
#[must_use]
pub fn resolve_type_args(type_args: &[TypeExpr], ctx: Ctx, state: &mut State<'_>) -> Vec<DataType> {
    type_args
        .iter()
        .map(|t| t.to_datatype(&mut state.type_ctx(ctx.file_idx)))
        .collect()
}

/// Resolves a variant of a generic enum named with its arguments
/// (`Slot<int>::Empty`) to the instantiated enum and the variant's index.
pub fn resolve_generic_variant(
    path: &[SmolStr],
    type_args: &[TypeExpr],
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> (u16, u16) {
    let variant = path[path.len() - 1].clone();
    let base = path[path.len() - 2].clone();
    let args = resolve_type_args(type_args, ctx, state);
    let DataType::Enum(enum_id) =
        instantiate(&base, &args, span, &mut state.type_ctx(ctx.file_idx))
    else {
        crate::compiler::compiler_errors::error_enum(
            "Unknown enum",
            &format!("{base} does not name an enum"),
            span,
            ctx.file_idx,
            state.sources,
        );
    };
    let Some(variant_idx) = state.enums[enum_id as usize]
        .variants
        .iter()
        .position(|vt| vt.name == variant)
    else {
        crate::compiler::compiler_errors::error_enum(
            "Unknown enum variant",
            &format!("{} does not name an enum variant", path.join("::")),
            span,
            ctx.file_idx,
            state.sources,
        );
    };
    (enum_id, variant_idx as u16)
}

/// Resolves a call written with type arguments (`first<int>(nums)`) to the
/// function it names and the arguments bound to its type parameters.
pub fn resolve_generic_call(
    fn_name: &SmolStr,
    type_args: &[TypeExpr],
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> (usize, Vec<DataType>) {
    let Some(fn_id) =
        state
            .namespace
            .find_function(&[], fn_name, span, ctx.file_idx, state.sources)
    else {
        error_unknown_function(fn_name, span, state.namespace, ctx.file_idx, state.sources);
    };
    let args = resolve_call_type_args(fn_id, fn_name, type_args, span, ctx, state);
    (fn_id, args)
}

/// Checks the type arguments a call is written with against the parameters the
/// function it resolves to declares, and resolves them to types.
///
/// `name` is the spelling to report against, which for a method is the method
/// name rather than the mangled symbol the call reaches.
pub fn resolve_call_type_args(
    fn_id: usize,
    name: &str,
    type_args: &[TypeExpr],
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<DataType> {
    let params_len = state.fns[fn_id]
        .generics
        .as_ref()
        .map_or(0, |g| g.params.len());
    if params_len == 0 {
        error_type_args_on_plain_function(span, ctx.file_idx, name, state.sources);
    }
    if params_len != type_args.len() {
        error_type_arg_count(
            span,
            ctx.file_idx,
            name,
            params_len,
            type_args.len(),
            state.sources,
        );
    }
    resolve_type_args(type_args, ctx, state)
}

/// The return type a `host` or `dylib` block declares for a namespaced call,
/// or `None` when the path names no declared function.
///
/// The path is resolved whole, the way the call is compiled: the leading
/// element selects the block and the last element the function in it. A `host`
/// declaration wins over a `dylib` one of the same name, matching the order the
/// two are bound in.
fn declared_return_type(namespace: &[SmolStr], state: &State<'_>) -> Option<DataType> {
    let fn_name = namespace.last()?;
    let block = &namespace[0];
    let declared = |is_host: bool| {
        state
            .dyn_libs
            .iter()
            .find(|lib| lib.is_host == is_host && lib.name == *block)
            .and_then(|lib| lib.fns.iter().find(|f| f.name == *fn_name))
            .map(|f| f.return_type.clone())
    };
    declared(true).or_else(|| declared(false))
}

/// Renders a [`DataType`] with full struct/function detail for diagnostics.
///
/// Field and argument names are resolved against the compiler `State` by
/// `Struct`/`Fn` id. The plain `Display` impl (in `candela-vm`) has no
/// `State`, so it renders those variants opaquely; this is the compiler-side
/// detailed form.
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

/// Whether an argument of type `received` satisfies a parameter declared as
/// `expected`.
///
/// `Unknown` is the `any` slot and the type of a value the checker cannot pin
/// down (a `json::parse` result, for instance). It stands in for every type on
/// either side, so annotating a parameter `any` keeps the parameter dynamic and
/// passing a dynamic value to a typed parameter is still allowed. Every other
/// pair uses the ordinary type equality.
#[inline(always)]
#[must_use]
pub fn param_type_matches(expected: &DataType, received: &DataType) -> bool {
    *expected == DataType::Unknown || *received == DataType::Unknown || expected == received
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
///
/// # Panics
///
/// Panics when a `FunctionCall` node carries an empty namespace path, which
/// the parser never produces.
pub fn collect_direct_fn_calls(content: &[Expr], calls: &mut Vec<SmolStr>) {
    let mut expr_stack: Vec<&Expr> = content.iter().collect();
    while let Some(expression) = expr_stack.pop() {
        match expression {
            Expr::FunctionCall(args, namespace, _, _, _) => {
                calls.push(namespace.last().unwrap().clone());
                expr_stack.extend(args.iter());
            }
            Expr::Condition(x, y, _)
            | Expr::InlineCondition(x, y, _)
            | Expr::ElseIfBlock(x, y)
            | Expr::WhileBlock(x, y)
            | Expr::ObjFunctionCall(x, y, _, _, _, _, _) => {
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
            Expr::FunctionDecl(_, _, x, _, _, _) => expr_stack.extend(x.iter()),
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
            Expr::Struct(_, fields, _, _) => {
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
pub fn can_reach<S: std::hash::BuildHasher>(
    src_fn: &str,
    target_fn: &str,
    fns: &[Function],
    visited: &mut std::collections::HashSet<SmolStr, S>,
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
                    DataType::Map(m) => m.0.unwrap_or(DataType::Unknown),
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
            Expr::ObjFunctionCall(obj, args, namespace, _, _, _, _)
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
///
/// `type_args` are the arguments a generic call named. They are part of the
/// specialisation key because a type parameter no argument mentions still
/// changes what the body builds.
fn infer_user_fn_return_type(
    fn_id: usize,
    infered_arg_types: &[DataType],
    type_args: &[DataType],
    function_name: &str,
    v: &mut Vec<Variable>,
    ctx: Ctx,
    state: &mut State<'_>,
) -> DataType {
    let key = specialization_key(type_args, infered_arg_types);
    let func = &state.fns[fn_id];
    // Check the return type cache
    if let Some((_, ret)) = func
        .return_type_cache
        .iter()
        .find(|(args, _)| arg_types_specialize_equal(args, &key))
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
    state
        .generics
        .push_bindings(fn_bindings(fn_id, type_args, state));
    let fn_type = track_returns(&fn_code, v, fn_ctx, state, function_name);
    state.generics.pop_bindings();

    RETURN_TYPE_INFERRING.with(|s| s.borrow_mut().remove(&fn_id));

    let to_return = if fn_type.is_empty() {
        // No tracked type means either no value is returned at all, or every
        // returned value was itself dynamic (return-type tracking records no
        // type for `Unknown`). A function handing back an `any` payload is
        // dynamic, not null.
        if check_if_returns_void(&fn_code) {
            DataType::Null
        } else {
            DataType::Unknown
        }
    } else {
        // If function returns anything, check if it returns the same thing each time
        DataType::Union(Box::from(fn_type)).check_poly()
    };

    v.truncate(v_len_before_args);

    // Cache the result
    state.fns[fn_id]
        .return_type_cache
        .push((key, to_return.clone()));

    to_return
}

/// The key one specialisation of a function is found by: the type arguments the
/// call named, then the argument types it passed.
#[must_use]
pub fn specialization_key(type_args: &[DataType], arg_types: &[DataType]) -> Box<[DataType]> {
    let mut key: Vec<DataType> = Vec::with_capacity(type_args.len() + arg_types.len());
    key.extend(type_args.iter().cloned());
    key.extend(arg_types.iter().cloned());
    key.into_boxed_slice()
}

/// The type parameters bound while a function's body is compiled: what its
/// enclosing `impl` block fixed, plus what the call site named.
#[must_use]
pub fn fn_bindings(
    fn_id: usize,
    type_args: &[DataType],
    state: &State<'_>,
) -> Box<[(SmolStr, DataType)]> {
    let Some(generics) = state.fns[fn_id].generics.as_ref() else {
        return Box::from([]);
    };
    let mut frame: Vec<(SmolStr, DataType)> = generics.bindings.to_vec();
    for (param, arg) in generics.params.iter().zip(type_args) {
        frame.push((param.clone(), arg.clone()));
    }
    frame.into_boxed_slice()
}

impl Expr {
    /// Infers this expression's static [`DataType`] without emitting code.
    ///
    /// # Panics
    ///
    /// Panics when a call node carries an empty namespace path, which the
    /// parser never produces.
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
            Self::FunctionCall(args, namespace, span, _, type_args) => {
                // A call written with type arguments names either a variant of a
                // generic enum (`Slot<int>::Filled(x)`) or a generic function.
                if !type_args.is_empty() {
                    if namespace.len() >= 2 {
                        let (enum_id, _) =
                            resolve_generic_variant(namespace, type_args, *span, ctx, state);
                        return DataType::Enum(enum_id);
                    }
                    let fn_name = namespace.last().unwrap().clone();
                    let (fn_id, call_type_args) =
                        resolve_generic_call(&fn_name, type_args, *span, ctx, state);
                    let infered_arg_types = args
                        .iter()
                        .map(|x| x.infer_type(v, ctx, state))
                        .collect::<Vec<DataType>>();
                    return infer_user_fn_return_type(
                        fn_id,
                        &infered_arg_types,
                        &call_type_args,
                        &fn_name,
                        v,
                        ctx,
                        state,
                    );
                }
                // A qualified enum-variant construction (`Color::Red(x)`) has an
                // enum type; intercept before the namespaced-function paths.
                if namespace.len() >= 2
                    && let Some((enum_id, _)) =
                        crate::compiler::resolve_enum_variant(namespace, state)
                {
                    return DataType::Enum(enum_id);
                }
                // A call into a declared namespace takes its type from the
                // declaration, before the built-in table below gets to read the
                // bare name. `gpio::read` is whatever its `host` block says it
                // is, not the `read` that returns a string.
                if namespace.len() >= 2
                    && let Some(declared) = declared_return_type(namespace, state)
                {
                    return declared;
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
                                &infered_arg_types,
                                &[],
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
                            &infered_arg_types,
                            &[],
                            function_name,
                            v,
                            ctx,
                            state,
                        )
                    }
                }
            }
            Self::ObjFunctionCall(obj, args, namespace, _, fn_span, _, type_args) => {
                let method = namespace.last().unwrap().as_str();
                let obj_type = obj.infer_type(v, ctx, state);
                // A user-defined impl method resolves by the receiver's static
                // struct type to the mangled free function `Type#method`; its
                // return type is inferred exactly like any free function's. This
                // is checked before the builtin-method table so a struct method
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
                        let call_type_args = if type_args.is_empty() {
                            Vec::new()
                        } else {
                            resolve_call_type_args(fn_id, method, type_args, *fn_span, ctx, state)
                        };
                        return infer_user_fn_return_type(
                            fn_id,
                            &arg_types,
                            &call_type_args,
                            &mangled,
                            v,
                            ctx,
                            state,
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
                        let call_type_args = if type_args.is_empty() {
                            Vec::new()
                        } else {
                            resolve_call_type_args(fn_id, method, type_args, *fn_span, ctx, state)
                        };
                        return infer_user_fn_return_type(
                            fn_id,
                            &arg_types,
                            &call_type_args,
                            &mangled,
                            v,
                            ctx,
                            state,
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
                // A builtin-typed receiver resolving to an `impl` method
                // (`impl list { fn sum(self) ... }` -> `list#sum`) infers its
                // return type from that method, specialized for the receiver
                // and argument types.
                if let Some(fn_id) = crate::compiler::methods::impl_method_on_builtin(
                    method, &obj_type, args, v, ctx, state,
                ) {
                    let mut arg_types: Vec<DataType> = Vec::with_capacity(args.len() + 1);
                    arg_types.push(obj_type.clone());
                    for a in args {
                        arg_types.push(a.infer_type(v, ctx, state));
                    }
                    let call_type_args = if type_args.is_empty() {
                        Vec::new()
                    } else {
                        resolve_call_type_args(fn_id, method, type_args, *fn_span, ctx, state)
                    };
                    return infer_user_fn_return_type(
                        fn_id,
                        &arg_types,
                        &call_type_args,
                        method,
                        v,
                        ctx,
                        state,
                    );
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
            Self::NamespacedRef(path, span, type_args) => {
                if !type_args.is_empty() {
                    let (enum_id, _) = resolve_generic_variant(path, type_args, *span, ctx, state);
                    return DataType::Enum(enum_id);
                }
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
            Self::Struct(namespace, fields, span, type_args) => DataType::Struct(
                struct_literal_id(namespace, fields, type_args, *span, v, ctx, state),
            ),
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
                    // An anonymous function takes no return annotation.
                    return_type: None,
                    generics: None,
                });
                state.fn_registers.push(Vec::new());
                DataType::Fn(id)
            }
            _ => unsafe { unreachable_unchecked() },
        }
    }
}
