//! The attribute table a `.cdlb` carries, and the reader an embedder lists it
//! through.
//!
//! A `host` block may declare attributes (`attribute system(writes: type[] =
//! []) on fn;`), and a script puts them on its functions and structs
//! (`@ecs::system(writes: [Position])`). The compiler checks every use against
//! its declaration and records what it found here. candela gives an attribute
//! no meaning: the host that declared it reads the table and decides what it
//! does, the same way it decides what a `host` function does.

use crate::rt::DataType;
use crate::rt::EnumType;
use crate::rt::Struct;
use serde::Deserialize;
use serde::Serialize;

/// One attribute written on one item.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Attribute {
    /// The namespace of the `host` block that declares the attribute, which is
    /// the name the host registered it under. A script that imports the
    /// declaring module under another name still records this one.
    pub namespace: String,
    /// The attribute's name, as its declaration spells it.
    pub name: String,
    /// The item the attribute is written on.
    pub target: AttributeTarget,
    /// Every parameter the declaration lists, in declaration order, with the
    /// value the use gave it or, where the use left it out, its default.
    pub args: Vec<(String, AttributeValue)>,
}

impl Attribute {
    /// The value of argument `name`.
    #[must_use]
    pub fn arg(&self, name: &str) -> Option<&AttributeValue> {
        self.args.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

/// The item an [`Attribute`] is written on.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AttributeTarget {
    /// A function, by the name it is declared with, and its parameters in
    /// declaration order. A parameter written without a type is `Unknown`.
    Fn {
        name: String,
        params: Vec<(String, DataType)>,
    },
    /// A struct, by its id in the program's struct table.
    Struct(u16),
    /// An enum, by its id in the program's enum table.
    Enum(u16),
}

/// The value one argument of an [`Attribute`] holds.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum AttributeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    /// A type, for a parameter declared `type`. A struct or an enum carries its
    /// id in the program's tables.
    Type(DataType),
    /// A function, for a parameter declared `fn`: the name it is declared
    /// with, its parameters in declaration order (`Unknown` for one written
    /// without a type) and its declared return type (`Unknown` when it
    /// declares none), which is what a host reads to call it.
    Fn {
        name: String,
        params: Vec<(String, DataType)>,
        ret: DataType,
    },
    /// A variant of an enum the program declares, by the enum's id and the
    /// variant's name.
    Variant {
        enum_id: u16,
        name: String,
    },
    List(Vec<Self>),
}

impl AttributeValue {
    /// The elements of a list value.
    #[must_use]
    pub fn as_list(&self) -> Option<&[Self]> {
        if let Self::List(items) = self {
            Some(items)
        } else {
            None
        }
    }
}

/// A struct as the attribute table describes it.
#[derive(Clone, Debug)]
pub struct StructInfo {
    /// The name the program's struct table gives it, which is the name a
    /// [`Value::Struct`](crate::embed::Value::Struct) of it carries.
    pub name: String,
    /// Its fields in declaration order.
    pub fields: Vec<(String, DataType)>,
}

/// An enum as the attribute table describes it.
#[derive(Clone, Debug)]
pub struct EnumInfo {
    pub name: String,
    /// Its variants' names in declaration order.
    pub variants: Vec<String>,
}

/// Every attribute a program's scripts wrote, with the struct and enum tables
/// the types in it refer to.
///
/// [`read_attributes`](crate::artifact::read_attributes) reads one out of a
/// `.cdlb` without loading the program,
/// [`RuntimeProgram::attributes`](crate::artifact::RuntimeProgram::attributes)
/// out of a loaded one, and the compiler's `Program::attributes` out of a
/// script it compiled.
#[derive(Clone, Debug, Default)]
pub struct AttributeTable {
    /// The attributes, in the order the compiler met their items.
    pub attributes: Vec<Attribute>,
    /// The program's structs, indexed by the id a [`DataType::Struct`] or an
    /// [`AttributeTarget::Struct`] carries.
    pub structs: Vec<StructInfo>,
    /// The program's enums, indexed by the id a [`DataType::Enum`] or an
    /// [`AttributeValue::Variant`] carries.
    pub enums: Vec<EnumInfo>,
}

impl AttributeTable {
    /// Builds the table from a program's attributes and its runtime struct and
    /// enum tables.
    #[must_use]
    pub fn new(attributes: Vec<Attribute>, structs: &[Struct], enums: &[EnumType]) -> Self {
        Self {
            attributes,
            structs: structs
                .iter()
                .map(|s| StructInfo {
                    name: s.name.to_string(),
                    fields: s
                        .fields
                        .iter()
                        .map(|(n, t, _)| (n.to_string(), t.clone()))
                        .collect(),
                })
                .collect(),
            enums: enums
                .iter()
                .map(|e| EnumInfo {
                    name: e.name.to_string(),
                    variants: e.variants.iter().map(|v| v.name.to_string()).collect(),
                })
                .collect(),
        }
    }

    /// The attributes declared in host namespace `namespace` under `name`.
    pub fn named<'a>(
        &'a self,
        namespace: &'a str,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Attribute> + 'a {
        self.attributes
            .iter()
            .filter(move |a| a.namespace == namespace && a.name == name)
    }

    /// The type `ty` as candela source writes it, with structs and enums
    /// named from this table.
    #[must_use]
    pub fn type_name(&self, ty: &DataType) -> String {
        match ty {
            DataType::Struct(id) => self
                .structs
                .get(*id as usize)
                .map_or_else(|| String::from("struct"), |s| s.name.clone()),
            DataType::Enum(id) => self
                .enums
                .get(*id as usize)
                .map_or_else(|| String::from("enum"), |e| e.name.clone()),
            DataType::Array(Some(inner)) => format!("{}[]", self.type_name(inner)),
            DataType::Array(None) => String::from("any[]"),
            DataType::Unknown => String::from("any"),
            DataType::Map(kv) => {
                let key =
                    kv.0.as_ref()
                        .map_or_else(|| String::from("any"), |k| self.type_name(k));
                let value =
                    kv.1.as_ref()
                        .map_or_else(|| String::from("any"), |v| self.type_name(v));
                format!("{{{key}: {value}}}")
            }
            DataType::Union(members) => members
                .iter()
                .map(|m| self.type_name(m))
                .collect::<Vec<_>>()
                .join(" | "),
            other => other.to_string(),
        }
    }
}
