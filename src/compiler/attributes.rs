//! Attributes a `host` block declares, and the uses a script writes on its
//! functions and structs.
//!
//! A host block names an attribute, the item kind it goes on and the
//! parameters it takes (`attribute system(writes: type[] = []) on fn;`). A
//! script writes it in front of a top-level declaration, behind the module that
//! declares it (`@ecs::system(writes: [Position])`). The compiler checks that
//! the attribute exists, that it targets that kind of item, and that every
//! argument is a value of its parameter's type: a type name that resolves, a
//! function that is declared, a variant of the enum. It records what it found
//! in [`Attribute`]s, which the artifact carries and a host reads. candela
//! gives an attribute no meaning of its own.

use super::FileNamespaces;
use super::Function;
use super::SymbolKind;
use super::compiler_data::TypeNames;
use super::type_system::Generics;
use super::type_system::TypeCtx;
use super::type_system::TypeExpr;
use crate::errors::blue;
use crate::errors::green;
use crate::errors::throw_compiler_error;
use ariadne::Label;
use candela_vm::attributes::Attribute;
use candela_vm::attributes::AttributeTarget;
use candela_vm::attributes::AttributeValue;
use candela_vm::errors::build_report;
use candela_vm::rt::DataType;
use candela_vm::rt::EnumType;
use candela_vm::rt::Source;
use candela_vm::rt::Span;
use candela_vm::rt::Struct;
use smol_strc::SmolStr;

/// An attribute argument as written: a literal, a list, or a name or path,
/// which stands for a type, a function or an enum variant depending on the
/// parameter it is passed for.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValueExpr {
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    String(SmolStr, Span),
    List(Box<[Self]>, Span),
    Path(TypeExpr, Span),
}

impl AttrValueExpr {
    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Int(_, span)
            | Self::Float(_, span)
            | Self::Bool(_, span)
            | Self::String(_, span)
            | Self::List(_, span)
            | Self::Path(_, span) => *span,
        }
    }
}

/// The type a declaration gives one of its parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrParamTypeExpr {
    /// `type`: any type the script can name.
    Type,
    /// `fn`: a function the script declares.
    Fn,
    /// Any other type: `int`, `float`, `bool`, `string`, an enum, or a list
    /// of one of those.
    Value(TypeExpr),
    /// `type[]` or `fn[]`.
    List(Box<Self>),
}

/// One parameter of an attribute declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeParamDecl {
    pub name: SmolStr,
    pub ty: AttrParamTypeExpr,
    pub default: Option<AttrValueExpr>,
    pub span: Span,
}

/// The kind of item an attribute goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeTargetKind {
    Fn,
    Struct,
}

impl AttributeTargetKind {
    const fn keyword(self) -> &'static str {
        match self {
            Self::Fn => "fn",
            Self::Struct => "struct",
        }
    }
}

/// `attribute name(params) on fn;`, written in a `host` block.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeDecl {
    pub name: SmolStr,
    pub params: Box<[AttributeParamDecl]>,
    pub target: AttributeTargetKind,
    pub span: Span,
}

/// `@path(name: value, ...)`, written in front of a declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeUse {
    /// The module path and the attribute's name, last.
    pub path: Box<[SmolStr]>,
    pub args: Box<[(SmolStr, AttrValueExpr, Span)]>,
    pub span: Span,
}

/// What a use is written on.
#[derive(Debug, Clone, Copy)]
pub enum UseTarget {
    Fn(u16),
    Struct(u16),
    /// A generic declaration, which names no one function or struct.
    Generic(AttributeTargetKind),
}

/// The declarations and uses the parsed files hold, waiting for the types and
/// functions they name to resolve.
#[derive(Default)]
pub struct PendingAttributes {
    /// Each declaration, with the host namespace and the file it was written
    /// in. A [`SymbolKind::Attribute`] is an index into this list.
    pub decls: Vec<(SmolStr, u16, AttributeDecl)>,
    /// Each run of uses, with the file it was written in and what it is on.
    pub uses: Vec<(u16, UseTarget, Box<[AttributeUse]>)>,
}

/// A parameter type, resolved.
enum ParamKind {
    Int,
    Float,
    Bool,
    String,
    Type,
    Fn,
    Enum(u16),
    List(Box<Self>),
}

/// The registries a use resolves against.
pub struct AttributeCtx<'a> {
    pub namespaces: &'a FileNamespaces,
    pub sources: &'a [Source],
    pub structs: &'a mut Vec<Struct>,
    pub enums: &'a mut Vec<EnumType>,
    pub fns: &'a mut Vec<Function>,
    pub generics: &'a mut Generics,
}

impl AttributeCtx<'_> {
    const fn type_ctx(&mut self, file_idx: u16) -> TypeCtx<'_> {
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

    fn describe(&self, kind: &ParamKind) -> String {
        match kind {
            ParamKind::Int => String::from("int"),
            ParamKind::Float => String::from("float"),
            ParamKind::Bool => String::from("bool"),
            ParamKind::String => String::from("string"),
            ParamKind::Type => String::from("type"),
            ParamKind::Fn => String::from("fn"),
            ParamKind::Enum(id) => self.enums[*id as usize].name.to_string(),
            ParamKind::List(inner) => format!("{}[]", self.describe(inner)),
        }
    }
}

/// Checks every use against its declaration and answers the attribute table,
/// in the order the uses were written.
pub fn resolve_attributes(
    pending: &PendingAttributes,
    ctx: &mut AttributeCtx<'_>,
) -> Vec<Attribute> {
    if pending.uses.is_empty() && pending.decls.is_empty() {
        return Vec::new();
    }
    // Each declaration's parameters and defaults, resolved where it is written.
    let mut decls: Vec<(
        &SmolStr,
        &AttributeDecl,
        Vec<(ParamKind, Option<AttributeValue>)>,
    )> = Vec::with_capacity(pending.decls.len());
    for (namespace, file_idx, decl) in &pending.decls {
        let mut params = Vec::with_capacity(decl.params.len());
        for (i, param) in decl.params.iter().enumerate() {
            if decl.params[..i].iter().any(|p| p.name == param.name) {
                error_attribute(
                    "attribute_parameter_type",
                    "Repeated attribute parameter",
                    &format!(
                        "{} names the parameter {} twice",
                        blue(&decl.name),
                        blue(&param.name)
                    ),
                    None,
                    param.span,
                    *file_idx,
                    ctx.sources,
                );
            }
            let kind = resolve_param_type(&param.ty, param.span, *file_idx, ctx);
            let default = param
                .default
                .as_ref()
                .map(|value| check_value(value, &kind, &param.name, *file_idx, ctx));
            params.push((kind, default));
        }
        decls.push((namespace, decl, params));
    }

    let mut out = Vec::new();
    for (file_idx, target, uses) in &pending.uses {
        for attr in uses {
            let (path, name) = attr.path.split_at(attr.path.len() - 1);
            let name = &name[0];
            let Some(decl_id) = ctx
                .namespaces
                .get(*file_idx)
                .resolve(path)
                .and_then(|ns| ns.find_attribute(name))
            else {
                error_attribute(
                    "unknown_attribute",
                    "Unknown attribute",
                    &format!("There is no attribute named {}", blue(attr.path.join("::"))),
                    Some(&format!(
                        "An attribute is declared in a host block, as in {}, and named behind the module that declares it, as in {}. The attribute candela itself reads is {}",
                        green("attribute system on fn;"),
                        green("@ecs::system"),
                        green("@cfg(...)")
                    )),
                    attr.span,
                    *file_idx,
                    ctx.sources,
                );
            };
            let (namespace, decl, params) = &decls[decl_id];
            let kind = match target {
                UseTarget::Fn(_) => AttributeTargetKind::Fn,
                UseTarget::Struct(_) => AttributeTargetKind::Struct,
                UseTarget::Generic(kind) => *kind,
            };
            if kind != decl.target {
                error_attribute(
                    "attribute_wrong_target",
                    "Attribute on the wrong kind of item",
                    &format!(
                        "{} goes on a {} declaration, and this is a {}",
                        blue(attr.path.join("::")),
                        blue(decl.target.keyword()),
                        blue(kind.keyword())
                    ),
                    None,
                    attr.span,
                    *file_idx,
                    ctx.sources,
                );
            }
            let target = match *target {
                UseTarget::Fn(fn_id) => {
                    let function = &ctx.fns[fn_id as usize];
                    AttributeTarget::Fn {
                        name: function.name.to_string(),
                        params: function
                            .args
                            .iter()
                            .map(|(n, t)| (n.to_string(), t.clone().unwrap_or(DataType::Unknown)))
                            .collect(),
                    }
                }
                UseTarget::Struct(struct_id) => AttributeTarget::Struct(struct_id),
                UseTarget::Generic(kind) => error_attribute(
                    "attribute_wrong_target",
                    "Attribute on a generic declaration",
                    &format!(
                        "{} goes on one {}, and a generic {} stands for one per set of type arguments",
                        blue(attr.path.join("::")),
                        kind.keyword(),
                        kind.keyword()
                    ),
                    None,
                    attr.span,
                    *file_idx,
                    ctx.sources,
                ),
            };

            let mut given: Vec<Option<AttributeValue>> = params.iter().map(|_| None).collect();
            for (arg_name, value, arg_span) in &attr.args {
                if arg_name.is_empty() {
                    error_attribute(
                        "attribute_bad_argument",
                        "Unnamed attribute argument",
                        "An attribute argument names the parameter it is for",
                        Some(&format!(
                            "Write the parameter's name, a ':' and the value, as in {}",
                            green(format_args!(
                                "@{}({}: ...)",
                                attr.path.join("::"),
                                decl.params.first().map_or("name", |p| p.name.as_str())
                            ))
                        )),
                        *arg_span,
                        *file_idx,
                        ctx.sources,
                    );
                }
                let Some(at) = decl.params.iter().position(|p| p.name == *arg_name) else {
                    let names: Vec<String> =
                        decl.params.iter().map(|p| p.name.to_string()).collect();
                    error_attribute(
                        "attribute_bad_argument",
                        "Unknown attribute argument",
                        &format!(
                            "{} has no parameter named {}",
                            blue(attr.path.join("::")),
                            blue(arg_name)
                        ),
                        Some(&if names.is_empty() {
                            String::from("It takes no arguments")
                        } else {
                            format!("Its parameters are {}", names.join(", "))
                        }),
                        *arg_span,
                        *file_idx,
                        ctx.sources,
                    );
                };
                if given[at].is_some() {
                    error_attribute(
                        "attribute_bad_argument",
                        "Repeated attribute argument",
                        &format!("{} is given more than once", blue(arg_name)),
                        None,
                        *arg_span,
                        *file_idx,
                        ctx.sources,
                    );
                }
                given[at] = Some(check_value(value, &params[at].0, arg_name, *file_idx, ctx));
            }
            let mut args = Vec::with_capacity(params.len());
            for ((param, (_, default)), value) in decl.params.iter().zip(params).zip(given) {
                let Some(value) = value.or_else(|| default.clone()) else {
                    error_attribute(
                        "attribute_bad_argument",
                        "Missing attribute argument",
                        &format!(
                            "{} needs a value for {}, which has no default",
                            blue(attr.path.join("::")),
                            blue(&param.name)
                        ),
                        None,
                        attr.span,
                        *file_idx,
                        ctx.sources,
                    );
                };
                args.push((param.name.to_string(), value));
            }
            out.push(Attribute {
                namespace: namespace.to_string(),
                name: decl.name.to_string(),
                target,
                args,
            });
        }
    }
    out
}

/// Resolves the type a declaration gives a parameter, refusing one no
/// argument could be written for.
fn resolve_param_type(
    ty: &AttrParamTypeExpr,
    span: Span,
    file_idx: u16,
    ctx: &mut AttributeCtx<'_>,
) -> ParamKind {
    match ty {
        AttrParamTypeExpr::Type => ParamKind::Type,
        AttrParamTypeExpr::Fn => ParamKind::Fn,
        AttrParamTypeExpr::List(inner) => {
            ParamKind::List(Box::new(resolve_param_type(inner, span, file_idx, ctx)))
        }
        AttrParamTypeExpr::Value(texpr) => {
            let resolved = texpr.to_datatype(&mut ctx.type_ctx(file_idx));
            param_kind_of(&resolved).unwrap_or_else(|| {
                error_attribute(
                    "attribute_parameter_type",
                    "Attribute parameter type",
                    &format!(
                        "An attribute argument cannot be a {}",
                        blue(
                            TypeNames {
                                structs: ctx.structs,
                                enums: ctx.enums
                            }
                            .of(&resolved)
                        )
                    ),
                    Some("A parameter is a type (type), a function (fn), an int, a float, a bool, a string, an enum, or a list of one of those",),
                    span,
                    file_idx,
                    ctx.sources,
                )
            })
        }
    }
}

fn param_kind_of(ty: &DataType) -> Option<ParamKind> {
    Some(match ty {
        DataType::Int => ParamKind::Int,
        DataType::Float => ParamKind::Float,
        DataType::Bool => ParamKind::Bool,
        DataType::String => ParamKind::String,
        DataType::Enum(id) => ParamKind::Enum(*id),
        DataType::Array(Some(inner)) => ParamKind::List(Box::new(param_kind_of(inner)?)),
        _ => return None,
    })
}

/// Checks one argument against the type of its parameter, resolving the names
/// in it in the scope of file `file_idx`.
fn check_value(
    value: &AttrValueExpr,
    kind: &ParamKind,
    param: &str,
    file_idx: u16,
    ctx: &mut AttributeCtx<'_>,
) -> AttributeValue {
    let resolved = match (kind, value) {
        (ParamKind::Int, AttrValueExpr::Int(i, _)) => Some(AttributeValue::Int(*i)),
        (ParamKind::Float, AttrValueExpr::Float(f, _)) => Some(AttributeValue::Float(*f)),
        (ParamKind::Bool, AttrValueExpr::Bool(b, _)) => Some(AttributeValue::Bool(*b)),
        (ParamKind::String, AttrValueExpr::String(s, _)) => {
            Some(AttributeValue::String(s.to_string()))
        }
        (ParamKind::List(inner), AttrValueExpr::List(items, _)) => Some(AttributeValue::List(
            items
                .iter()
                .map(|item| check_value(item, inner, param, file_idx, ctx))
                .collect(),
        )),
        (ParamKind::Type, AttrValueExpr::Path(texpr, _)) => Some(AttributeValue::Type(
            texpr.to_datatype(&mut ctx.type_ctx(file_idx)),
        )),
        (
            ParamKind::Fn,
            AttrValueExpr::Path(
                texpr @ (TypeExpr::Identifier(..) | TypeExpr::NamespacedIdentifier(..)),
                span,
            ),
        ) => {
            let (path, name): (&[SmolStr], &SmolStr) = match texpr {
                TypeExpr::NamespacedIdentifier(full, _) => {
                    let (path, name) = full.split_at(full.len() - 1);
                    (path, &name[0])
                }
                TypeExpr::Identifier(name, _) => (&[], name),
                _ => unreachable!("the pattern above admits a name or a path only"),
            };
            let Some(fn_id) = ctx.namespaces.get(file_idx).find_function(path, name) else {
                let written = path
                    .iter()
                    .chain(std::iter::once(name))
                    .map(SmolStr::as_str)
                    .collect::<Vec<_>>()
                    .join("::");
                error_attribute(
                    "attribute_bad_argument",
                    "Unknown function in an attribute",
                    &format!("There is no function named {}", blue(written)),
                    Some(&format!(
                        "{} takes the name of a declared function",
                        blue(param)
                    )),
                    *span,
                    file_idx,
                    ctx.sources,
                );
            };
            Some(AttributeValue::Fn(ctx.fns[fn_id].name.to_string()))
        }
        (
            ParamKind::Enum(enum_id),
            AttrValueExpr::Path(TypeExpr::NamespacedIdentifier(full, _), span),
        ) => {
            let (path, variant) = full.split_at(full.len() - 1);
            let variant = &variant[0];
            let (enum_path, enum_name) = path.split_at(path.len().saturating_sub(1));
            let found = enum_name
                .first()
                .and_then(|enum_name| ctx.namespaces.get(file_idx).find_enum(enum_path, enum_name));
            let declared = &ctx.enums[*enum_id as usize];
            let variant_of = declared.variants.iter().find(|v| v.name == *variant);
            match (found, variant_of) {
                (Some(found), Some(v)) if found == *enum_id as usize && v.payload.is_empty() => {
                    Some(AttributeValue::Variant {
                        enum_id: *enum_id,
                        name: variant.to_string(),
                    })
                }
                (Some(found), Some(_)) if found == *enum_id as usize => error_attribute(
                    "attribute_bad_argument",
                    "Attribute argument with a payload",
                    &format!(
                        "{} carries a payload, and an attribute argument is a variant without one",
                        blue(full.join("::"))
                    ),
                    None,
                    *span,
                    file_idx,
                    ctx.sources,
                ),
                _ => None,
            }
        }
        _ => None,
    };
    resolved.unwrap_or_else(|| {
        let expected = ctx.describe(kind);
        let note = match kind {
            ParamKind::Type => {
                String::from("A type argument is a type name, as in Position or int[]")
            }
            ParamKind::Fn => String::from("A function argument is the name of a declared function"),
            ParamKind::Enum(id) => format!(
                "An enum argument is one of its variants, as in {}::{}",
                expected,
                ctx.enums[*id as usize]
                    .variants
                    .first()
                    .map_or("Variant", |v| v.name.as_str())
            ),
            ParamKind::List(_) => {
                String::from("A list argument is written in brackets, as in [a, b]")
            }
            _ => format!("Write a {expected} literal"),
        };
        error_attribute(
            "attribute_bad_argument",
            "Attribute argument type mismatch",
            &format!("{} takes a {}", blue(param), blue(&expected)),
            Some(&note),
            value.span(),
            file_idx,
            ctx.sources,
        )
    })
}

#[inline(never)]
#[cold]
fn error_attribute(
    code: &'static str,
    title: &str,
    label: &str,
    note: Option<&str>,
    span: Span,
    file_idx: u16,
    sources: &[Source],
) -> ! {
    let plain = format!("{title}: {label}");
    throw_compiler_error(
        &|| {
            let src = &sources[file_idx as usize];
            let mut report = build_report(
                ariadne::ReportKind::Error,
                (src.filename.as_str(), span.into()),
            )
            .with_message(title)
            .with_label(
                Label::new((src.filename.as_str(), span.into()))
                    .with_message(label)
                    .with_color(ariadne::Color::Red),
            );
            if let Some(note) = note {
                report = report.with_note(note);
            }
            report.finish()
        },
        sources,
        file_idx,
        span,
        &plain,
        code,
    );
}

impl SymbolKind {
    /// The declaration index of an attribute symbol.
    #[must_use]
    pub const fn attribute(self) -> Option<usize> {
        if let Self::Attribute(id) = self {
            Some(id as usize)
        } else {
            None
        }
    }
}
