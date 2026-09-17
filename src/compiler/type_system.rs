use super::expr::Expr;
use super::expr::METHOD_SEP;
use super::expr::Span;
use super::expr::mangle_method;
use super::expr::symbol_of_expr;
use crate::compiler::FileNamespaces;
use crate::compiler::Namespace;
use crate::compiler::SymbolKind;
use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::EnumType;
use crate::compiler::compiler_data::EnumVariant;
use crate::compiler::compiler_data::FnGenerics;
use crate::compiler::compiler_data::Function;
use crate::compiler::compiler_data::Source;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Struct;
use crate::compiler::compiler_data::TypeNames;
use crate::compiler::compiler_data::Variable;
use crate::compiler::compiler_errors::error_instantiation_depth;
use crate::compiler::compiler_errors::error_invalid_obj_type;
use crate::compiler::compiler_errors::error_invalid_type;
use crate::compiler::compiler_errors::error_op;
use crate::compiler::compiler_errors::error_struct_unknown_field;
use crate::compiler::compiler_errors::error_type_arg_count;
use crate::compiler::compiler_errors::error_type_args_on_plain_function;
use crate::compiler::compiler_errors::error_type_args_on_plain_type;
use crate::compiler::compiler_errors::error_type_not_indexable;
use crate::compiler::compiler_errors::error_unknown_function;
use crate::compiler::compiler_errors::error_unknown_function_in_namespace;
use crate::compiler::compiler_errors::error_unknown_namespace;
use crate::compiler::compiler_errors::error_unknown_struct;
use crate::compiler::compiler_errors::error_unknown_type;
use crate::compiler::compiler_errors::error_unknown_type_param;
use crate::compiler::compiler_errors::error_unknown_type_with_namespace;
use crate::compiler::compiler_errors::error_unknown_variable;
use crate::compiler::functions::callee_fn_id;
use crate::compiler::methods::dyn_lib_receiver;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::cell::RefCell;
use std::hint::cold_path;
use std::hint::unreachable_unchecked;
use std::rc::Rc;
use std::slice;

pub use crate::rt::DataType;

/// Name prefix for the synthetic top-level function an anonymous function is
/// hoisted to. `<` is not a legal identifier character, so a hoisted name can
/// never collide with a user-written function.
///
/// Public because a frontend reading the function table has to tell the
/// entries a person wrote from the ones the compiler made: an outline lists
/// the first and leaves out the second.
pub const ANON_FN_PREFIX: &str = "<anon>";

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
    /// A generic type applied to its arguments, `Cell<int>`, with or without
    /// a module path in front of the name (`g::Slot<int>`).
    Generic(Box<GenericType>),
    Array(Box<Self>),
    Map(Box<Self>, Box<Self>),
    Union(Box<[Self]>),
}

/// A generic type and the arguments it is applied to. Boxed inside
/// [`TypeExpr`], which every declaration carries by value.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct GenericType {
    /// The module path the name was written behind, empty for a name written
    /// on its own. A module bound with `as` puts its alias here
    /// (`g::Slot<int>`).
    pub namespace: Box<[SmolStr]>,
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
                // A name written behind a module path names that module's
                // generic type, never a type parameter of the body being
                // compiled.
                (generic.namespace.is_empty() && params.contains(&generic.name))
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
                    } else if let Some(struct_id) =
                        ctx.scope()
                            .find_struct(&[], struct_name, *span, ctx.file_idx, ctx.sources)
                    {
                        DataType::Struct(struct_id as u16)
                    } else if let Some(enum_id) = ctx.scope().find_enum(&[], struct_name) {
                        DataType::Enum(enum_id as u16)
                    } else if find_template(&[], struct_name, ctx.scope(), ctx.generics).is_some() {
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
                            ctx.scope(),
                        );
                    }
                }
            },
            Self::NamespacedIdentifier(s, span) => {
                if let Some(struct_id) = ctx.scope().find_struct(
                    &s[..s.len() - 1],
                    unsafe { s.last().unwrap_unchecked() },
                    *span,
                    ctx.file_idx,
                    ctx.sources,
                ) {
                    DataType::Struct(struct_id as u16)
                } else if let Some(enum_id) = ctx
                    .scope()
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
                        ctx.scope(),
                        &s[..s.len() - 1],
                    )
                }
            }
            Self::Generic(generic) => {
                // The path in front of the name says which module the type
                // comes from, so a path that names no module is reported here
                // rather than resolving to whatever declared the name.
                if !generic.namespace.is_empty()
                    && ctx.scope().resolve(&generic.namespace).is_none()
                {
                    cold_path();
                    error_unknown_namespace(
                        &generic.namespace,
                        generic.span,
                        ctx.file_idx,
                        ctx.sources,
                    );
                }
                let args: Vec<DataType> = generic.args.iter().map(|a| a.to_datatype(ctx)).collect();
                instantiate(&generic.namespace, &generic.name, &args, generic.span, ctx)
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

/// The type parameters in scope while one body is compiled.
///
/// `named` are the ones the call site's type arguments and the enclosing `impl`
/// block fixed. `unnamed` are the parameters neither of them mentions: they
/// stand for `any`, so a body that writes such a parameter in a type position
/// compiles instead of reporting a type that has no value yet. The two are kept
/// apart because an annotation mentioning an unnamed parameter stays un-pinned:
/// what the body returns is inferred rather than checked against `any`.
#[derive(Debug, Default)]
pub struct BindingFrame {
    named: Box<[(SmolStr, DataType)]>,
    unnamed: Box<[SmolStr]>,
}

impl BindingFrame {
    /// A frame in which every parameter is fixed to a type, which is what an
    /// `impl` block and a generic instantiation produce.
    #[must_use]
    pub fn all_named(named: Box<[(SmolStr, DataType)]>) -> Self {
        Self {
            named,
            unnamed: Box::from([]),
        }
    }

    fn is_empty(&self) -> bool {
        self.named.is_empty() && self.unnamed.is_empty()
    }
}

/// The generic declarations of a program, the instantiations made from them,
/// and the type parameters bound while a body is being compiled.
#[derive(Debug, Default)]
pub struct Generics {
    templates: Vec<TypeTemplate>,
    impls: Vec<ImplTemplate>,
    /// Instantiations by the declaration they came from and their rendered
    /// name, so `Cell<int>` written twice is one struct and two modules each
    /// declaring `Cell<T>` keep their own. `<` and `>` cannot occur in an
    /// identifier, so a rendered name never collides with a user-written one.
    instantiations: Vec<(usize, SmolStr, DataType)>,
    /// Type parameters bound for the body being compiled. Only the top frame is
    /// in scope: the parameters of a function never reach the body of a
    /// function it calls.
    bindings: Vec<BindingFrame>,
    depth: u32,
}

impl Generics {
    /// The type this name is currently bound to, if it names a type parameter
    /// of the body being compiled. A parameter nothing named is `any`.
    #[must_use]
    pub fn bound(&self, name: &str) -> Option<DataType> {
        let frame = self.bindings.last()?;
        frame
            .named
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, t)| t.clone())
            .or_else(|| {
                frame
                    .unnamed
                    .iter()
                    .any(|param| param == name)
                    .then_some(DataType::Unknown)
            })
    }

    /// Whether the call site or an enclosing `impl` block fixed this type
    /// parameter to a type, as opposed to leaving it standing for `any`. An
    /// annotation that mentions a parameter nothing named stays un-pinned.
    #[must_use]
    pub fn names(&self, name: &str) -> bool {
        self.bindings
            .last()
            .is_some_and(|frame| frame.named.iter().any(|(param, _)| param == name))
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
            .map(|frame| {
                frame
                    .named
                    .iter()
                    .map(|(p, _)| p.clone())
                    .chain(frame.unnamed.iter().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The parameters of the generic declaration `name` names, for a caller
    /// with no scope to resolve it in: a mangled method name
    /// (`Cell<int>#get`) carries the type name and no module path. Takes the
    /// declaration registered last when two modules declare the name.
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
    pub fn param_fields(&self, template_idx: usize) -> Vec<Option<SmolStr>> {
        let Some(template) = self.templates.get(template_idx) else {
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

    /// The generic declaration the last-registered template under `name` is,
    /// which is what an unqualified name falls back to when the scope it was
    /// written in registers no template by that name.
    fn last_template_named(&self, name: &str) -> Option<usize> {
        self.templates.iter().rposition(|t| t.name == name)
    }

    /// Records a generic `struct` declaration and returns the index the
    /// declaring scope registers it under, which is what a name written
    /// behind a module path resolves to.
    pub fn add_struct_template(
        &mut self,
        name: SmolStr,
        params: TypeParams,
        file_idx: u16,
        fields: Box<[(SmolStr, TypeExpr, Span)]>,
    ) -> u32 {
        self.templates.push(TypeTemplate {
            name,
            params,
            file_idx,
            body: TemplateBody::Struct(fields),
        });
        (self.templates.len() - 1) as u32
    }

    /// Records a generic `enum` declaration. Returns its index, as
    /// [`Generics::add_struct_template`] does.
    pub fn add_enum_template(
        &mut self,
        name: SmolStr,
        params: TypeParams,
        file_idx: u16,
        variants: Box<[(SmolStr, Box<[TypeExpr]>, Span)]>,
    ) -> u32 {
        self.templates.push(TypeTemplate {
            name,
            params,
            file_idx,
            body: TemplateBody::Enum(variants),
        });
        (self.templates.len() - 1) as u32
    }

    /// Records the `impl` blocks a file declared against a generic type,
    /// stamping each with the file it came from.
    pub fn add_impls(&mut self, impls: Vec<ImplTemplate>, file_idx: u16) {
        self.impls.extend(impls.into_iter().map(|mut block| {
            block.file_idx = file_idx;
            block
        }));
    }

    /// Binds `frame` for the body about to be compiled. Every body pushes a
    /// frame, an empty one when it has no type parameters, so the caller's
    /// parameters do not resolve inside it.
    pub fn push_bindings(&mut self, frame: Box<[(SmolStr, DataType)]>) {
        self.bindings.push(BindingFrame::all_named(frame));
    }

    /// Binds a frame that also carries the parameters nothing named, which a
    /// function specialisation has and an `impl` block does not.
    pub fn push_frame(&mut self, frame: BindingFrame) {
        self.bindings.push(frame);
    }

    pub fn pop_bindings(&mut self) {
        self.bindings.pop();
    }

    /// Snapshots the state a single generic instantiation (or a function
    /// specialization's own type-parameter frame) can grow or unbalance, so an
    /// aborted compile attempt can undo it with [`Generics::rollback_to`].
    ///
    /// `instantiations` and the structs/enums it names are a cache: an
    /// `instantiate` call registers the pair before the instantiation's own
    /// fields are resolved, so a type whose fields name itself resolves to the
    /// type being built instead of recursing. A diagnostic raised while
    /// resolving those fields (or while lowering an `impl` block against the
    /// instantiation, which pushes its lowered methods the same way a closure
    /// literal pushes an anonymous function) leaves the cache entry pointing at
    /// a struct or enum whose fields were never filled in. `bindings` and
    /// `depth` are pushed and incremented, respectively, before that same
    /// resolution and are only popped or decremented after it returns; a
    /// `push_bindings` without a matching `pop_bindings` (function
    /// specialization has its own such pair too, see
    /// `functions/user_functions.rs`) leaks a frame, and an unmatched `depth`
    /// increment eventually trips the instantiation-depth cap for programs
    /// nowhere near it.
    #[must_use]
    pub const fn checkpoint(&self) -> GenericsCheckpoint {
        GenericsCheckpoint {
            instantiations: self.instantiations.len(),
            bindings: self.bindings.len(),
            depth: self.depth,
        }
    }

    /// Undoes everything a failed compile attempt did to the instantiation
    /// cache and the binding/depth bookkeeping, back to `checkpoint`. The
    /// structs and enums an aborted `instantiate` call registered are not
    /// `Generics`' to remove; the caller truncates those (see
    /// `Program::rollback_to`) using the same pre-attempt lengths.
    pub fn rollback_to(&mut self, checkpoint: &GenericsCheckpoint) {
        self.instantiations.truncate(checkpoint.instantiations);
        self.bindings.truncate(checkpoint.bindings);
        self.depth = checkpoint.depth;
    }
}

/// A checkpoint of the [`Generics`] state taken by [`Generics::checkpoint`] and
/// undone by [`Generics::rollback_to`].
pub struct GenericsCheckpoint {
    instantiations: usize,
    bindings: usize,
    depth: u32,
}

/// What resolving a [`TypeExpr`] needs: the scope the type is written in, and
/// the registries an instantiation adds to.
pub struct TypeCtx<'a> {
    pub file_idx: u16,
    pub namespaces: &'a FileNamespaces,
    pub sources: &'a [Source],
    pub structs: &'a mut Vec<Struct>,
    pub enums: &'a mut Vec<EnumType>,
    pub fns: &'a mut Vec<Function>,
    pub generics: &'a mut Generics,
}

impl<'a> TypeCtx<'a> {
    /// The scope the type being resolved is written in.
    #[must_use]
    pub fn scope(&self) -> &'a Namespace {
        self.namespaces.get(self.file_idx)
    }
    /// The same registries, resolving names in another file's scope. A
    /// declaration resolves its own types where it was written, whatever file
    /// the use is written in.
    pub const fn reborrow(&mut self, file_idx: u16) -> TypeCtx<'_> {
        TypeCtx {
            file_idx,
            namespaces: self.namespaces,
            sources: self.sources,
            structs: self.structs,
            enums: self.enums,
            fns: self.fns,
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
///
/// `qualifier` is the module the declaration was written in, present only when
/// another module declares a generic type by the same name. Two such types are
/// separate types, and the name is what a diagnostic prints and what a method
/// on the type is mangled under, so both have to say which module they came
/// from.
#[must_use]
fn render_instantiation(
    qualifier: Option<&str>,
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
    match qualifier {
        Some(module) => format_args!("{module}::{base}<{rendered}>").to_smolstr(),
        None => format_args!("{base}<{rendered}>").to_smolstr(),
    }
}

/// The path segments a file is known by: its path with the `.cdl` taken off,
/// split on the directory separator, which is what an `import` writes.
#[must_use]
fn module_segments(file_idx: u16, sources: &[Source]) -> Vec<&str> {
    let Some(source) = sources.get(file_idx as usize) else {
        return Vec::new();
    };
    let path: &str = source
        .filename
        .strip_suffix(".cdl")
        .unwrap_or(&source.filename);
    path.split(['/', '\\']).filter(|s| !s.is_empty()).collect()
}

/// The module prefix a type declared in `file_idx` takes when the modules in
/// `declarers` declare that name too: the fewest trailing path segments that
/// tell this file apart from the others. One `plain.cdl` gives `plain`; two of
/// them, one per package, give `strutil::plain` and `sqlite::plain`, so a
/// qualified name is one type and no more.
#[must_use]
fn module_qualifier(file_idx: u16, declarers: &[u16], sources: &[Source]) -> SmolStr {
    let mine = module_segments(file_idx, sources);
    let others: Vec<Vec<&str>> = declarers
        .iter()
        .filter(|other| **other != file_idx)
        .map(|other| module_segments(*other, sources))
        .collect();
    for take in 1..=mine.len() {
        let tail = &mine[mine.len() - take..];
        if others
            .iter()
            .all(|other| other.len() < take || &other[other.len() - take..] != tail)
        {
            return tail.join("::").into();
        }
    }
    mine.join("::").into()
}

/// Puts the declaring module in front of every plain `struct` or `enum` name
/// that more than one module declares, so a package's types never collide with
/// a consumer's types of the same name.
///
/// Plain types share one program-wide table keyed by name, and a method lowers
/// to a free function named after its type (`Plain#get`), so two modules each
/// declaring `Plain` leave one `Plain#get` answering calls on both. The name a
/// duplicate is registered under becomes `right::Plain`, which is what a
/// diagnostic prints and what a method on it is mangled under; a generic type
/// two modules declare is qualified the same way, see `render_instantiation`.
/// A name only one module declares is left as it was written.
///
/// Every `impl` method is re-mangled against the name its receiver ended up
/// with, resolved in the scope the `impl` block was written in, and so are the
/// calls a method makes on its own receiver, which the recursion check reads.
pub fn qualify_duplicate_type_names(
    structs: &mut [Struct],
    enums: &mut [EnumType],
    fns: &mut [Function],
    struct_files: &[(u16, u16)],
    enum_files: &[(u16, u16)],
    namespaces: &FileNamespaces,
    sources: &[Source],
) {
    // The modules that declare each name. A struct and an enum of one name are
    // as much two types as two structs are: both mangle their methods against
    // that one name.
    let mut declarers: FxHashMap<SmolStr, Vec<u16>> = FxHashMap::default();
    for (name, file_idx) in struct_files
        .iter()
        .filter_map(|(id, file_idx)| Some((&structs.get(*id as usize)?.name, *file_idx)))
        .chain(
            enum_files
                .iter()
                .filter_map(|(id, file_idx)| Some((&enums.get(*id as usize)?.name, *file_idx))),
        )
    {
        declarers.entry(name.clone()).or_default().push(file_idx);
    }
    if declarers.values().all(|files| files.len() <= 1) {
        return;
    }

    for &(struct_id, file_idx) in struct_files {
        let Some(declared) = structs.get_mut(struct_id as usize) else {
            continue;
        };
        if let Some(files) = declarers.get(&declared.name)
            && files.len() > 1
        {
            declared.name = qualified_type_name(file_idx, files, &declared.name, sources);
        }
    }
    for &(enum_id, file_idx) in enum_files {
        let Some(declared) = enums.get_mut(enum_id as usize) else {
            continue;
        };
        if let Some(files) = declarers.get(&declared.name)
            && files.len() > 1
        {
            declared.name = qualified_type_name(file_idx, files, &declared.name, sources);
        }
    }

    for func in fns.iter_mut() {
        let scope = namespaces.get(func.src_file);
        if let Some(mangled) = requalify_method(&func.name, scope, structs, enums) {
            func.name = mangled;
        }
        for callee in &mut func.direct_calls {
            if let Some(mangled) = requalify_method(callee, scope, structs, enums) {
                *callee = mangled;
            }
        }
    }
}

/// The name a type declared in `file_idx` is registered under once the modules
/// in `declarers` declare that name too.
#[must_use]
fn qualified_type_name(
    file_idx: u16,
    declarers: &[u16],
    name: &str,
    sources: &[Source],
) -> SmolStr {
    format_args!("{}::{name}", module_qualifier(file_idx, declarers, sources)).to_smolstr()
}

/// The mangled symbol a method ends up with, when the type its `impl` block
/// named was qualified. `None` when `name` is not a method, when the type it
/// names is not a plain type in `scope` (a builtin receiver, or a generic
/// instantiation, which carries its own qualifier already), or when that type
/// kept the name it was written with.
#[must_use]
fn requalify_method(
    name: &str,
    scope: &Namespace,
    structs: &[Struct],
    enums: &[EnumType],
) -> Option<SmolStr> {
    let (type_name, method) = name.split_once(METHOD_SEP)?;
    let declared = scope.symbols.iter().find_map(|(symbol, kind)| {
        if symbol.as_str() != type_name {
            return None;
        }
        match kind {
            SymbolKind::Struct(id) => structs.get(*id as usize).map(|s| &s.name),
            SymbolKind::Enum(id) => enums.get(*id as usize).map(|e| &e.name),
            SymbolKind::Fn(_) | SymbolKind::Template(_) => None,
        }
    })?;
    (declared != type_name).then(|| mangle_method(declared, method))
}

/// The generic declaration `namespace::name` names, as an index into
/// `Generics::templates`.
///
/// A name written behind a module path resolves only to the declaration that
/// module makes, so two modules each declaring `Slot<T>` stay apart. A name
/// written on its own resolves in the scope it was written in, which holds the
/// file's own declarations and the ones its bare imports merged in; when that
/// scope has none it falls back to the last declaration of the name anywhere
/// in the program, which is where a name reaches a template registered while a
/// body was being compiled.
#[must_use]
pub fn find_template(
    namespace: &[SmolStr],
    name: &str,
    scope: &Namespace,
    generics: &Generics,
) -> Option<usize> {
    scope.find_template(namespace, name).or_else(|| {
        namespace
            .is_empty()
            .then(|| generics.last_template_named(name))
            .flatten()
    })
}

/// Resolves `base<args>` to an ordinary struct or enum, registering it the
/// first time it is asked for.
///
/// Past this point nothing generic is left: the instantiation is a concrete
/// type with concrete field types, and every later stage (field access, method
/// dispatch, the artifact codec, the VM) treats it like any other.
pub fn instantiate(
    namespace: &[SmolStr],
    base: &str,
    args: &[DataType],
    span: Span,
    ctx: &mut TypeCtx<'_>,
) -> DataType {
    let Some(template_idx) = find_template(namespace, base, ctx.scope(), ctx.generics) else {
        if ctx
            .scope()
            .find_struct(namespace, base, span, ctx.file_idx, ctx.sources)
            .is_some()
            || ctx.scope().find_enum(namespace, base).is_some()
        {
            error_type_args_on_plain_type(span, ctx.file_idx, base, ctx.sources);
        }
        if !namespace.is_empty() {
            error_unknown_type_with_namespace(
                span,
                ctx.file_idx,
                base,
                ctx.sources,
                ctx.scope(),
                namespace,
            );
        }
        error_unknown_type(span, ctx.file_idx, base, ctx.sources, ctx.scope());
    };
    // Another module declaring the same name makes the two separate types, and
    // the name has to say so; see `render_instantiation`.
    let declarers: Vec<u16> = ctx
        .generics
        .templates
        .iter()
        .filter(|t| t.name == base)
        .map(|t| t.file_idx)
        .collect();
    let qualifier = (declarers.len() > 1).then(|| {
        module_qualifier(
            ctx.generics.templates[template_idx].file_idx,
            &declarers,
            ctx.sources,
        )
    });
    let name = render_instantiation(qualifier.as_deref(), base, args, ctx.structs, ctx.enums);
    if let Some((_, _, t)) = ctx
        .generics
        .instantiations
        .iter()
        .find(|(idx, n, _)| *idx == template_idx && n == &name)
    {
        return t.clone();
    }
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
        .push((template_idx, name.clone(), instantiated.clone()));

    let body = ctx.generics.templates[template_idx].body.clone();
    ctx.generics.depth += 1;
    ctx.generics.push_bindings(frame.clone());
    {
        let mut inner = ctx.reborrow(template_file);
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

    lower_impls(template_idx, base, args, &name, &frame, ctx);
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
    template_idx: usize,
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
        // The header names a type in the scope of the file the block was
        // written in, so a block in one module never attaches to another
        // module's declaration of the same name.
        if find_template(&[], base, ctx.namespaces.get(impl_file), ctx.generics)
            != Some(template_idx)
        {
            continue;
        }
        let header = ctx.generics.impls[idx].args.clone();
        let mut frame: Vec<(SmolStr, DataType)> = Vec::with_capacity(header.len());
        let mut applies = true;
        for (header_arg, arg) in header.iter().zip(args) {
            match header_arg {
                // A bare name that is not a type of its own is a parameter the
                // header introduces, bound to whatever this instantiation
                // passes. Anything else is a concrete type the header pins, and
                // the block applies only when the argument is that type.
                TypeExpr::Identifier(pname, _) if is_type_parameter(pname, impl_file, ctx) => {
                    frame.push((pname.clone(), arg.clone()));
                }
                other => {
                    let mut inner = ctx.reborrow(impl_file);
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
            lower_method(&method, type_name, &bindings, impl_file, ctx);
        }
    }
}

/// Whether a name written as a type argument in an `impl` header introduces a
/// type parameter rather than naming a type.
fn is_type_parameter(name: &SmolStr, file_idx: u16, ctx: &TypeCtx<'_>) -> bool {
    if matches!(
        name.as_str(),
        "int" | "float" | "bool" | "string" | "null" | "any"
    ) {
        return false;
    }
    find_template(&[], name, ctx.namespaces.get(file_idx), ctx.generics).is_none()
        && !ctx
            .namespaces
            .get(file_idx)
            .symbols
            .iter()
            .any(|(symbol, kind)| {
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
    let mut inner = ctx.reborrow(file_idx);
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
    collect_direct_fn_calls(code, Some(type_name.as_str()), &mut callees);
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
        return instantiated_struct_id(path, &name, &args, span, ctx, state);
    }
    if let Some(template_idx) =
        find_template(path, &name, state.scope(ctx.file_idx), state.generics)
    {
        let param_fields = state.generics.param_fields(template_idx);
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
        return instantiated_struct_id(path, &name, &args, span, ctx, state);
    }
    state
        .scope(ctx.file_idx)
        .find_struct(path, &name, span, ctx.file_idx, state.sources)
        .unwrap_or_else(|| {
            error_unknown_struct(&name, span, state.sources, ctx.file_idx);
        }) as u16
}

fn instantiated_struct_id(
    namespace: &[SmolStr],
    name: &SmolStr,
    args: &[DataType],
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> u16 {
    match instantiate(
        namespace,
        name,
        args,
        span,
        &mut state.type_ctx(ctx.file_idx),
    ) {
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

/// Whether a call path written with type arguments names an enum variant
/// rather than a generic function.
///
/// The list is written on the segment before the variant
/// (`Slot<int>::Filled`, `g::Slot<int>::Filled`) and on the last segment for a
/// function (`first<int>`, `m::first<int>`), so whether that earlier segment
/// names a type is what tells the two apart. A non-generic enum counts too, so
/// `Color::Red<int>(1)` still reports type arguments on a plain type instead of
/// looking for a function in a namespace called `Color`.
#[must_use]
pub fn type_args_name_a_variant(path: &[SmolStr], ctx: Ctx, state: &State<'_>) -> bool {
    let Some(base_idx) = path.len().checked_sub(2) else {
        return false;
    };
    let base = &path[base_idx];
    find_template(
        &path[..base_idx],
        base,
        state.scope(ctx.file_idx),
        state.generics,
    )
    .is_some()
        || state
            .scope(ctx.file_idx)
            .find_enum(&path[..base_idx], base)
            .is_some()
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
    let namespace = &path[..path.len() - 2];
    let args = resolve_type_args(type_args, ctx, state);
    let DataType::Enum(enum_id) = instantiate(
        namespace,
        &base,
        &args,
        span,
        &mut state.type_ctx(ctx.file_idx),
    ) else {
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

/// Resolves a call written with type arguments to the function it names and
/// the arguments bound to its type parameters.
///
/// `namespace` is the path in front of the name, empty for a call the file
/// writes unqualified (`first<int>(nums)`) and the module alias for one it
/// reaches through an import (`m::first<int>(nums)`).
pub fn resolve_generic_call(
    namespace: &[SmolStr],
    fn_name: &SmolStr,
    type_args: &[TypeExpr],
    span: Span,
    ctx: Ctx,
    state: &mut State<'_>,
) -> (usize, Vec<DataType>) {
    let Some(fn_id) = state.scope(ctx.file_idx).find_function(namespace, fn_name) else {
        if !namespace.is_empty() {
            error_unknown_function_in_namespace(fn_name, namespace, span, ctx.file_idx, state);
        }
        error_unknown_function(
            fn_name,
            span,
            state.scope(ctx.file_idx),
            ctx.file_idx,
            state.sources,
        );
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

/// The type an unqualified built-in call yields, or `None` when no built-in
/// takes that name.
///
/// Keep in step with the match in `builtin_functions`, which lowers these
/// calls: a name that lowers there and is missing here is reported as an
/// unknown function, and a name here that does not lower there types a call
/// that never reaches a built-in.
fn builtin_fn_return_type(name: &str) -> Option<DataType> {
    Some(match name {
        "print" | "exit" | "throw" => DataType::Null,
        "type" | "str" | "input" | "json_stringify" | "as_str" => DataType::String,
        "float" | "as_float" => DataType::Float,
        "int" | "the_answer" | "as_int" => DataType::Int,
        "bool" | "as_bool" | "is_int" | "is_float" | "is_str" | "is_bool" | "is_list"
        | "is_map" | "is_null" => DataType::Bool,
        "range" => DataType::Array(Some(Box::from(DataType::Int))),
        "argv" => DataType::Array(Some(Box::from(DataType::String))),
        // A downcast to a collection yields an element/entry type of `any`
        // (Unknown). That is a known type, not a gap: the entries stay dynamic
        // instead of taking their type from the first `push`/`insert` the way an
        // empty literal does. json::parse yields a fully dynamic value.
        "as_list" => DataType::Array(Some(Box::from(DataType::Unknown))),
        "as_map" => DataType::Map(Box::from((
            Some(DataType::Unknown),
            Some(DataType::Unknown),
        ))),
        "json_parse" => DataType::Unknown,
        _ => return None,
    })
}

/// The type an `fs::` call yields, or `None` when the file library takes no
/// function of that name. Keep in step with `fs_lib_functions`, which lowers
/// these calls.
///
/// These names are reachable only through the `fs` path. Answering for them off
/// the bare last segment is what made a program's own `read` or `exists` infer
/// as a file operation.
fn fs_fn_return_type(name: &str) -> Option<DataType> {
    Some(match name {
        "read" => DataType::String,
        "exists" => DataType::Bool,
        "write" | "append" | "delete" | "delete_dir" => DataType::Null,
        _ => return None,
    })
}

/// Renders a [`DataType`] with full struct/function detail. This is what the
/// `type` builtin hands back, so its output is a string the program can read.
///
/// Field and argument names are resolved against the compiler `State` by
/// `Struct`/`Fn` id. The plain `Display` impl (in `candela-vm`) has no
/// `State`, so it renders those variants opaquely; this is the compiler-side
/// detailed form. A diagnostic wants the shorter name a type was declared
/// under, which is [`TypeNames`] instead.
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
        // Both halves are rendered through this same function, so a map of a
        // user type names that type the way a list of it does. Reaching for
        // `Display` here is what made `type({"a": Value::Num(1)})` answer
        // `{string: enum}`.
        DataType::Map(m) => format_args!(
            "{{{}: {}}}",
            m.0.as_ref().map_or_else(
                || SmolStr::new_static("Unknown"),
                |k| format_detailed(k, state)
            ),
            m.1.as_ref().map_or_else(
                || SmolStr::new_static("Unknown"),
                |val| format_detailed(val, state)
            )
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

/// Fills in element types a value left open, taking them from the type that was
/// declared for it.
///
/// An empty array literal has no element type: it is `Array(None)`, so indexing
/// it yields `null` and a body specialised on it cannot match on its elements.
/// An empty map literal is `Map(None, None)`, so reading a key out of it yields
/// nothing the body can match on either. Where the declaration says what the
/// collection holds, the declaration wins and the specialisation compiles at the
/// declared types. Only an open position is filled, so a collection that does
/// name what it holds still has to match what was declared. The walk descends
/// through arrays and through both halves of a map, so an empty literal nested
/// inside one is pinned too.
fn pin_open_element_types(inferred: &mut DataType, declared: &DataType) {
    match (inferred, declared) {
        (DataType::Array(element), DataType::Array(Some(declared_element))) => match element {
            None => *element = Some(declared_element.clone()),
            Some(element) => pin_open_element_types(element, declared_element),
        },
        (DataType::Map(entry), DataType::Map(declared_entry)) => {
            pin_open_slot(&mut entry.0, declared_entry.0.as_ref());
            pin_open_slot(&mut entry.1, declared_entry.1.as_ref());
        }
        _ => {}
    }
}

/// Pins a caller's binding that still holds an empty literal to the type the
/// parameter it is passed to declares.
///
/// `let` takes no annotation, so a binding that starts out as `[]` or `{}`
/// holds nothing until something says what it holds. The first `push` or
/// `insert` on it is one such statement; handing it to a parameter that
/// declares a collection type is another, and it is often the first one the
/// caller writes, because the callee is what fills the collection. Pinning it
/// here gives the caller's later reads the element or value type the callee
/// put in.
///
/// Only a binding that is still the empty-literal placeholder is pinned. One
/// that already names what it holds keeps its type, so a parameter declaring
/// something else stays the argument mismatch the call site reports.
pub(crate) fn pin_empty_literal_bindings(
    args: &[Expr],
    declared_arg_types: &[Option<DataType>],
    v: &mut [Variable],
) {
    for (arg, declared) in args.iter().zip(declared_arg_types) {
        let (Expr::Var(name, _), Some(declared)) = (arg, declared) else {
            continue;
        };
        let Some(var) = v.iter_mut().rfind(|var| &var.name == name) else {
            continue;
        };
        if is_empty_literal_type(&var.var_type) {
            pin_open_element_types(&mut var.var_type, declared);
        }
    }
}

/// Whether a type is what an empty literal leaves behind: an array with no
/// element type, or a map with neither a key nor a value type. A downcast
/// (`as_map`) hands back entries of `any`, which names a type and is left
/// alone.
#[must_use]
fn is_empty_literal_type(ty: &DataType) -> bool {
    match ty {
        DataType::Array(element) => element.is_none(),
        DataType::Map(entry) => entry.0.is_none() && entry.1.is_none(),
        _ => false,
    }
}

/// One half of a map's type: its keys or its values. An open slot takes the
/// declared type, a slot that names a type is descended into so a nested empty
/// literal is pinned as well, and a declaration that is itself open leaves the
/// slot alone.
fn pin_open_slot(inferred: &mut Option<DataType>, declared: Option<&DataType>) {
    let Some(declared) = declared else {
        return;
    };
    match inferred {
        None => *inferred = Some(declared.clone()),
        Some(inferred) => pin_open_element_types(inferred, declared),
    }
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

/// Collects the functions the given code calls by name.
///
/// `self_type` is the type a method's body is compiled against, so a call the
/// body makes on its own receiver (`self.down(n - 1)`) is recorded under the
/// mangled name that method resolves to (`Box#down`). It is `None` for an
/// ordinary function, whose `self` names no receiver. A call on any other
/// receiver is left out: which function it reaches depends on the receiver's
/// type, which only the call site knows.
///
/// What this list is for is recursion: a function that can reach itself
/// through it is compiled as a call rather than inlined, so a method that
/// calls itself is compiled the way a recursive function is.
///
/// # Panics
///
/// Panics when a `FunctionCall` node carries an empty namespace path, which
/// the parser never produces.
pub fn collect_direct_fn_calls(
    content: &[Expr],
    self_type: Option<&str>,
    calls: &mut Vec<SmolStr>,
) {
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
            | Expr::WhileBlock(x, y) => {
                expr_stack.push(x);
                expr_stack.extend(y.iter());
            }
            Expr::ObjFunctionCall(obj, args, namespace, _, _, _, _) => {
                if let Some(type_name) = self_type
                    && matches!(&**obj, Expr::Var(name, _) if name == "self")
                    && let Some(method) = namespace.last()
                {
                    calls.push(mangle_method(type_name, method));
                }
                expr_stack.push(obj);
                expr_stack.extend(args.iter());
            }
            Expr::CallValue(callee, args, _, _) => {
                expr_stack.push(callee);
                expr_stack.extend(args.iter());
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
            Expr::ForLoop(var_name, array_expr, array_code, span) => {
                let inferred_collection_type = array_expr.infer_type(v, ctx, state);
                let elem_type = match inferred_collection_type {
                    DataType::Array(inner) => inner.map_or(DataType::Unknown, |t| *t),
                    DataType::String => DataType::String,
                    DataType::Unknown => DataType::Unknown,
                    // A map iterates its keys.
                    DataType::Map(m) => m.0.unwrap_or(DataType::Unknown),
                    // A function whose return type is inferred has its body
                    // walked before the loop is compiled, so a loop over a
                    // value nothing can iterate arrives here first and reports
                    // what the compile stage would have.
                    t => error_type_not_indexable(
                        &t,
                        *span,
                        true,
                        ctx.file_idx,
                        state.sources,
                        state.type_names(),
                    ),
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
                // Only an array with no element type at all upgrades here; an
                // array of `any` compares equal to `Array(None)` and keeps its
                // dynamic element type.
                if let Expr::Var(var_name, _) = obj.as_ref()
                    && v.iter()
                        .rfind(|var| &var.name == var_name)
                        .is_some_and(|var| matches!(var.var_type, DataType::Array(None)))
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
                if !is_enum {
                    crate::compiler::check_match_scrutinee_is_enum(
                        &scrut_type,
                        arms,
                        *span,
                        v,
                        ctx,
                        state,
                    );
                }
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
    // The body is compiled at the types `handle_user_function` specialises on,
    // which pin an argument's open element type to what the parameter declares.
    // Infer against the same types, or the caller reads a return type from a
    // body that was never compiled that way. Reading the declarations costs a
    // clone, so the arguments are asked first whether any left one open.
    let pinned = if infered_arg_types.iter().any(has_open_element_type) {
        let declared_arg_types = specialized_arg_types(fn_id, type_args, ctx, state);
        pinned_arg_types(infered_arg_types, &declared_arg_types)
    } else {
        None
    };
    let infered_arg_types = pinned.as_deref().unwrap_or(infered_arg_types);

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
        .push_frame(fn_bindings(fn_id, type_args, state));
    let fn_type = track_returns(&fn_code, v, fn_ctx, state, function_name);
    // Read while the bindings are still pushed, so a generic `-> T[]` resolves
    // to the element type this call named.
    let declared_return = specialized_return_type(fn_id, fn_ctx, state);
    state.generics.pop_bindings();

    RETURN_TYPE_INFERRING.with(|s| s.borrow_mut().remove(&fn_id));

    let mut to_return = if fn_type.is_empty() {
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

    // `return []` carries no element type, so a declared `-> T[]` is what the
    // caller gets: the annotation says what the empty array holds.
    if let Some((declared, _)) = &declared_return {
        pin_open_element_types(&mut to_return, declared);
    }

    v.truncate(v_len_before_args);

    // Cache the result
    state.fns[fn_id]
        .return_type_cache
        .push((key, to_return.clone()));

    to_return
}

/// The declared return type of `function` in the `host` or `dylib` block named
/// `block`, when such a block declares such a function. Both spellings of the
/// call, `app::rows(id)` and `app.rows(id)`, read the signature through here.
fn dyn_lib_return_type(block: &str, function: &str, state: &State<'_>) -> Option<DataType> {
    state
        .dyn_libs
        .iter()
        .find(|lib| lib.name == block)
        .and_then(|lib| lib.fns.iter().find(|sig| sig.name == function))
        .map(|sig| sig.return_type.clone())
}

/// The `-> Type` annotation as it reads for the specialisation being compiled,
/// with the type parameters currently bound.
///
/// An annotation naming a parameter the call left unbound stays un-pinned, so
/// what the body returns is inferred rather than checked against a type that
/// has no value yet.
pub(crate) fn specialized_return_type(
    fn_id: usize,
    ctx: Ctx,
    state: &mut State<'_>,
) -> Option<(DataType, Span)> {
    let Some(generics) = state.fns[fn_id].generics.as_ref() else {
        return state.fns[fn_id].return_type.clone();
    };
    let unbound: Vec<SmolStr> = generics
        .params
        .iter()
        .filter(|param| !state.generics.names(param))
        .cloned()
        .collect();
    let generics = state.fns[fn_id].generics.as_ref()?;
    let annotation = generics.return_type.as_deref()?;
    if annotation.0.mentions_any(&unbound) {
        return state.fns[fn_id].return_type.clone();
    }
    let (return_type, return_span) = annotation.clone();
    let file_idx = generics.file_idx;
    let mut base = state.type_ctx(ctx.file_idx);
    let mut type_ctx = base.reborrow(file_idx);
    Some((return_type.to_datatype(&mut type_ctx), return_span))
}

/// The parameter types this call specialises on.
///
/// Without type arguments these are the function's own declared types, where an
/// annotation naming a type parameter was left un-pinned. A call that names its
/// type arguments resolves the annotations again with them bound, which is what
/// makes `first<int>(xs)` reject a `float[]`.
pub(crate) fn specialized_arg_types(
    fn_id: usize,
    type_args: &[DataType],
    ctx: Ctx,
    state: &mut State<'_>,
) -> Vec<Option<DataType>> {
    let declared = state.fns[fn_id]
        .args
        .iter()
        .map(|(_, t)| t.clone())
        .collect::<Vec<Option<DataType>>>();
    if type_args.is_empty() {
        return declared;
    }
    let Some(generics) = state.fns[fn_id].generics.as_ref() else {
        return declared;
    };
    let arg_types = generics.arg_types.clone();
    let file_idx = generics.file_idx;
    let frame = fn_bindings(fn_id, type_args, state);
    let mut base = state.type_ctx(ctx.file_idx);
    let mut type_ctx = base.reborrow(file_idx);
    type_ctx.generics.push_frame(frame);
    let resolved = arg_types
        .iter()
        .map(|t| t.as_ref().map(|t| t.to_datatype(&mut type_ctx)))
        .collect();
    type_ctx.generics.pop_bindings();
    resolved
}

/// The argument types a call specialises on: what its arguments infer to, with
/// any element type they left open filled in from `declared_arg_types`, the
/// parameter declarations as [`specialized_arg_types`] reads them.
///
/// Answers `None` when no argument left an element type open, which is every
/// call that passes no empty collection literal.
pub(crate) fn pinned_arg_types(
    infered_arg_types: &[DataType],
    declared_arg_types: &[Option<DataType>],
) -> Option<Vec<DataType>> {
    if !infered_arg_types.iter().any(has_open_element_type) {
        return None;
    }
    let mut pinned = infered_arg_types.to_vec();
    for (inferred, declared) in pinned.iter_mut().zip(declared_arg_types) {
        if let Some(declared) = declared {
            pin_open_element_types(inferred, declared);
        }
    }
    Some(pinned)
}

/// Whether this type leaves an element type open, which is what an empty array
/// or map literal produces.
#[must_use]
fn has_open_element_type(ty: &DataType) -> bool {
    match ty {
        DataType::Array(None) => true,
        DataType::Array(Some(element)) => has_open_element_type(element),
        DataType::Map(entry) => has_open_slot(entry.0.as_ref()) || has_open_slot(entry.1.as_ref()),
        _ => false,
    }
}

/// Whether one half of a map's type is open: either the map names no type there
/// at all, or what it names holds an open element type itself.
#[must_use]
fn has_open_slot(slot: Option<&DataType>) -> bool {
    slot.is_none_or(has_open_element_type)
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
///
/// A parameter neither of them mentions is recorded as unnamed, which stands
/// for `any`. A call never has to name its type arguments, so `signal("x")` and
/// `signal<any>("x")` compile the same body; without this the body could not
/// write `T` in a type position at all.
#[must_use]
pub fn fn_bindings(fn_id: usize, type_args: &[DataType], state: &State<'_>) -> BindingFrame {
    let Some(generics) = state.fns[fn_id].generics.as_ref() else {
        return BindingFrame::default();
    };
    let mut named: Vec<(SmolStr, DataType)> = generics.bindings.to_vec();
    for (param, arg) in generics.params.iter().zip(type_args) {
        named.push((param.clone(), arg.clone()));
    }
    let unnamed: Box<[SmolStr]> = generics
        .params
        .iter()
        .filter(|param| !named.iter().any(|(bound, _)| bound == *param))
        .cloned()
        .collect();
    BindingFrame {
        named: named.into_boxed_slice(),
        unnamed,
    }
}

/// The type a builtin method returns for a receiver, applied across a union
/// the way the compile-time receiver check does: a union is accepted only when
/// every member is, so the answer is the union of the per-member answers.
///
/// `rule` gives the return type for one receiver type and `None` for a receiver
/// the method does not take, which is the set `builtin_methods` rejects.
fn builtin_method_return(
    obj_type: &DataType,
    rule: fn(&DataType) -> Option<DataType>,
) -> Option<DataType> {
    let DataType::Union(members) = obj_type else {
        return rule(obj_type);
    };
    let mut types: Vec<DataType> = Vec::with_capacity(members.len());
    for member in members {
        let member_type = rule(member)?;
        if !types.contains(&member_type) {
            types.push(member_type);
        }
    }
    Some(DataType::Union(types.into_boxed_slice()).check_poly())
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
                } else if let Some(fn_id) = state.scope(ctx.file_idx).find_function(&[], name) {
                    // A bare identifier that names a function is a function
                    // reference (a compile-time value passed to a higher-order
                    // function). Its static type is the callee's Fn id.
                    DataType::Fn(fn_id as u16)
                } else if let Some((enum_id, _)) = crate::compiler::resolve_enum_variant(
                    std::slice::from_ref(name),
                    ctx.file_idx,
                    state,
                ) {
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
                        error_op(
                            &l,
                            &r,
                            "+",
                            *span_l,
                            *span_r,
                            ctx.file_idx,
                            state.sources,
                            state.type_names(),
                        );
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
                            state.type_names(),
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
                        state.type_names(),
                    ),
                }
            }
            Self::BoolAnd(x, y, span_l, span_r) | Self::BoolOr(x, y, span_l, span_r) => {
                match (x.infer_type(v, ctx, state), y.infer_type(v, ctx, state)) {
                    (DataType::Unknown | DataType::Bool, DataType::Bool)
                    | (DataType::Bool, DataType::Unknown) => DataType::Bool,
                    (l, r) => {
                        error_op(
                            &l,
                            &r,
                            "&&",
                            *span_l,
                            *span_r,
                            ctx.file_idx,
                            state.sources,
                            state.type_names(),
                        );
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
                    state.type_names(),
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
                    state.type_names(),
                ),
            },
            Self::ArrayGetIndex(array, _, span) => match array.infer_type(v, ctx, state) {
                DataType::Array(array_type) => array_type.map_or(DataType::Null, |t| *t),
                DataType::String => DataType::String,
                DataType::Unknown => DataType::Unknown,
                t => error_type_not_indexable(
                    &t,
                    *span,
                    false,
                    ctx.file_idx,
                    state.sources,
                    state.type_names(),
                ),
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
                        TypeNames {
                            structs: state.structs,
                            enums: state.enums,
                        },
                    );
                }
            }
            Self::ArrayGetSlice(array, _, _, span) => match array.infer_type(v, ctx, state) {
                DataType::Array(array_type) => DataType::Array(array_type),
                DataType::String => DataType::String,
                DataType::Unknown => DataType::Unknown,
                t => error_type_not_indexable(
                    &t,
                    *span,
                    false,
                    ctx.file_idx,
                    state.sources,
                    state.type_names(),
                ),
            },
            // The return type of an indirect call is the return type of the
            // function the callee's type names, at the argument types this
            // call site passes.
            Self::CallValue(callee, args, span, _) => {
                let fn_id = callee_fn_id(callee, *span, v, ctx, state);
                let infered_arg_types = args
                    .iter()
                    .map(|x| x.infer_type(v, ctx, state))
                    .collect::<Vec<DataType>>();
                let fn_name = state.fns[fn_id].name.clone();
                infer_user_fn_return_type(fn_id, &infered_arg_types, &[], &fn_name, v, ctx, state)
            }
            Self::FunctionCall(args, namespace, span, _, type_args) => {
                // A call written with type arguments names either a variant of a
                // generic enum (`Slot<int>::Filled(x)`) or a generic function,
                // which may itself sit behind a module alias
                // (`m::first<int>(xs)`).
                if !type_args.is_empty() {
                    if type_args_name_a_variant(namespace, ctx, state) {
                        let (enum_id, _) =
                            resolve_generic_variant(namespace, type_args, *span, ctx, state);
                        return DataType::Enum(enum_id);
                    }
                    let len = namespace.len() - 1;
                    let fn_name = namespace[len].clone();
                    let (fn_id, call_type_args) = resolve_generic_call(
                        &namespace[..len],
                        &fn_name,
                        type_args,
                        *span,
                        ctx,
                        state,
                    );
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
                // enum type; intercept before the function paths, which is where
                // `handle_functions` intercepts it too.
                if namespace.len() >= 2
                    && let Some((enum_id, _)) =
                        crate::compiler::resolve_enum_variant(namespace, ctx.file_idx, state)
                {
                    return DataType::Enum(enum_id);
                }
                // What follows resolves the name in the order `handle_functions`
                // lowers it: the path picks the family, and inside a family a
                // function in scope wins over a built-in of the same name. One
                // flat table of built-in names, read before the scope, is what
                // typed a program's own `read` as the string `fs::read` returns.
                let (fn_name, path) = namespace.split_last().unwrap();
                let infered_args = |v: &mut Vec<Variable>, state: &mut State<'_>| {
                    args.iter()
                        .map(|x| x.infer_type(v, ctx, state))
                        .collect::<Vec<DataType>>()
                };
                if !path.is_empty() {
                    if path == ["fs"] {
                        return fs_fn_return_type(fn_name).unwrap_or_else(|| {
                            error_unknown_function(
                                fn_name,
                                *span,
                                &Namespace::default(),
                                ctx.file_idx,
                                state.sources,
                            )
                        });
                    }
                    // A call into a `host` or `dylib` block takes its type from
                    // the declaration: `gpio::read` is whatever its block says it
                    // is, not the `read` that returns a string.
                    if let Some(declared) = declared_return_type(namespace, state) {
                        return declared;
                    }
                    let Some(fn_id) = state.scope(ctx.file_idx).find_function(path, fn_name) else {
                        error_unknown_function_in_namespace(
                            fn_name,
                            path,
                            *span,
                            ctx.file_idx,
                            state,
                        );
                    };
                    let infered_arg_types = infered_args(v, state);
                    return infer_user_fn_return_type(
                        fn_id,
                        &infered_arg_types,
                        &[],
                        fn_name,
                        v,
                        ctx,
                        state,
                    );
                }
                // A call to a function-typed parameter (a higher-order function
                // calling the function it was handed): the callee comes from the
                // parameter's static Fn type. Lowering reads the same function
                // off the scope symbol `handle_user_function` declares for that
                // parameter, which inference has not run yet.
                if let Some(DataType::Fn(fn_id)) = v
                    .iter()
                    .rfind(|var| var.name.as_str() == fn_name.as_str())
                    .map(|var| var.var_type.clone())
                {
                    let infered_arg_types = infered_args(v, state);
                    return infer_user_fn_return_type(
                        fn_id as usize,
                        &infered_arg_types,
                        &[],
                        fn_name,
                        v,
                        ctx,
                        state,
                    );
                }
                if let Some(fn_id) = state.scope(ctx.file_idx).find_function(&[], fn_name) {
                    let infered_arg_types = infered_args(v, state);
                    return infer_user_fn_return_type(
                        fn_id,
                        &infered_arg_types,
                        &[],
                        fn_name,
                        v,
                        ctx,
                        state,
                    );
                }
                if let Some(return_type) = builtin_fn_return_type(fn_name) {
                    return return_type;
                }
                // An unqualified call whose name is an enum variant (`Some(x)`)
                // constructs that variant. Functions above keep priority.
                if let Some((enum_id, _)) =
                    crate::compiler::resolve_enum_variant(namespace, ctx.file_idx, state)
                {
                    return DataType::Enum(enum_id);
                }
                error_unknown_function(
                    fn_name,
                    *span,
                    state.scope(ctx.file_idx),
                    ctx.file_idx,
                    state.sources,
                )
            }
            Self::ObjFunctionCall(obj, args, namespace, obj_span, fn_span, _, type_args) => {
                let method = namespace.last().unwrap().as_str();
                // `app.rows(id)` on a `host`/`dylib` block is the namespaced
                // call written with a dot, so its type is the declared return
                // type, exactly as for `app::rows(id)`. This has to come before
                // the receiver is typed, which would report the block's name as
                // an unknown variable. See `methods::dyn_lib_receiver`.
                if namespace.len() == 1
                    && let Some(lib_name) = dyn_lib_receiver(obj, v, state)
                {
                    let lib_name = lib_name.clone();
                    if let Some(return_type) = dyn_lib_return_type(&lib_name, method, state) {
                        return return_type;
                    }
                    error_unknown_function_in_namespace(
                        method,
                        slice::from_ref(&lib_name),
                        *fn_span,
                        ctx.file_idx,
                        state,
                    );
                }
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
                // Each arm below answers for a receiver the compile stage
                // accepts. A call whose value is used reaches inference before
                // anything compiles it, so a receiver or a name the compile
                // stage would reject has to be rejected here, with the same
                // diagnostic it would have raised.
                macro_rules! receiver_type {
                    ($rule:expr, $expected:expr) => {
                        builtin_method_return(&obj_type, $rule).unwrap_or_else(|| {
                            error_invalid_obj_type(
                                $expected,
                                &obj_type,
                                method,
                                *obj_span,
                                state.sources,
                                ctx.file_idx,
                                state.type_names(),
                            )
                        })
                    };
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
                    "repeat" | "reverse" => receiver_type!(
                        |t| match t {
                            DataType::String => Some(DataType::String),
                            DataType::Array(element) => Some(DataType::Array(element.clone())),
                            _ => None,
                        },
                        &[DataType::String, DataType::Array(None)]
                    ),
                    "push" | "sort" | "remove" | "insert" => DataType::Null,
                    "sqrt" | "round" | "floor" => DataType::Float,
                    "abs" => receiver_type!(
                        |t| match t {
                            DataType::Float => Some(DataType::Float),
                            DataType::Int => Some(DataType::Int),
                            _ => None,
                        },
                        &[DataType::Int, DataType::Float]
                    ),
                    "split" => DataType::Array(Some(Box::from(DataType::String))),
                    "partition" => receiver_type!(
                        |t| match t {
                            DataType::Array(element) => Some(DataType::Array(Some(Box::from(
                                DataType::Array(element.clone())
                            )))),
                            _ => None,
                        },
                        &[DataType::Array(None)]
                    ),
                    "get" => receiver_type!(
                        |t| match t {
                            DataType::Map(m) => Some(m.1.clone().unwrap_or(DataType::Unknown)),
                            _ => None,
                        },
                        &[DataType::Map(Box::from((None, None)))]
                    ),
                    "keys" => receiver_type!(
                        |t| match t {
                            DataType::Map(m) => Some(DataType::Array(m.0.clone().map(Box::new))),
                            _ => None,
                        },
                        &[DataType::Map(Box::from((None, None)))]
                    ),
                    "values" => receiver_type!(
                        |t| match t {
                            DataType::Map(m) => Some(DataType::Array(m.1.clone().map(Box::new))),
                            _ => None,
                        },
                        &[DataType::Map(Box::from((None, None)))]
                    ),
                    // No builtin takes this name, and the impl-method and
                    // struct/enum lookups above already missed, so the call
                    // names nothing. `libs/std` documents methods that only
                    // exist once their module is imported, which is the common
                    // way to land here.
                    _ => error_unknown_function(
                        method,
                        *fn_span,
                        &Namespace::default(),
                        ctx.file_idx,
                        state.sources,
                    ),
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
                if let Some((enum_id, _)) =
                    crate::compiler::resolve_enum_variant(path, ctx.file_idx, state)
                {
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
                // hoist is keyed by the file and span the literal was written at
                // and reused: the first encounter registers the function, later
                // ones resolve to the same id, and two files whose closures sit
                // at the same offset stay apart.
                let fn_name = format_args!(
                    "{ANON_FN_PREFIX}{}:{}:{}",
                    ctx.file_idx, span.start, span.end
                )
                .to_smolstr();
                if let Some(id) = state.fns.iter().rposition(|f| f.name == fn_name) {
                    return DataType::Fn(id as u16);
                }
                let returns_null = check_if_returns_void(code);
                let mut callees = Vec::new();
                collect_direct_fn_calls(code, None, &mut callees);
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
                DataType::Fn(id)
            }
            _ => unsafe { unreachable_unchecked() },
        }
    }
}
