//! Host/script value marshalling for candela's embedding API.
//!
//! These types cross the boundary between a Rust host and a running candela
//! script: [`Value`] is the dynamically-typed carrier, [`HostType`] describes
//! the shapes that can cross, the `FromHostValue`/`IntoHostValue`/`IntoHostFn`
//! traits adapt Rust closures into registered host functions, and
//! [`marshal_value`]/[`unmarshal_value`] convert between [`Value`] and the VM's
//! NaN-boxed [`Data`]. They carry no compiler state and the VM depends on them,
//! so they live in the VM-only crate. The `Engine`/`Program` embedding surface
//! (which compiles and drives scripts) lives in the `candela` crate on top.

use crate::data::Data;
use crate::data::DataHash;
use crate::data::NULL;
use crate::rt::DataType;
use crate::rt::Struct;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::StringPool;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::rc::Rc;

/// A candela runtime map: `Data`-keyed, hashed by the raw NaN-boxed bits.
type CandelaMap = HashMap<Data, Data, BuildHasherDefault<DataHash>>;

/// A dynamically-typed value passed across the host/script boundary.
///
/// candela integers are 32-bit internally (NaN-boxed); [`Value::Int`] widens them
/// to `i64` for host ergonomics and narrows on the way back in.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    /// A candela array `T[]`. Elements are expected to be homogeneous, matching
    /// candela's static array typing.
    Array(Vec<Self>),
    /// A candela string-keyed map `{string: V}` (or a struct read back as a record).
    /// Ordered so equality and iteration are deterministic on the host side.
    Map(BTreeMap<String, Self>),
}

impl Value {
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        if let Self::Int(i) = self {
            Some(*i)
        } else {
            None
        }
    }
    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        if let Self::Float(f) = self {
            Some(*f)
        } else {
            None
        }
    }
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    #[must_use]
    pub const fn as_str(&self) -> Option<&str> {
        if let Self::String(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }
    #[must_use]
    pub const fn as_array(&self) -> Option<&Vec<Self>> {
        if let Self::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
    #[must_use]
    pub const fn as_map(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Map(m) = self {
            Some(m)
        } else {
            None
        }
    }
    #[must_use]
    pub fn into_string(self) -> Option<String> {
        if let Self::String(s) = self {
            Some(s)
        } else {
            None
        }
    }
    #[must_use]
    pub fn into_array(self) -> Option<Vec<Self>> {
        if let Self::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
    #[must_use]
    pub fn into_map(self) -> Option<BTreeMap<String, Self>> {
        if let Self::Map(m) = self {
            Some(m)
        } else {
            None
        }
    }
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Int(i64::from(v))
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.to_owned())
    }
}
impl From<()> for Value {
    fn from((): ()) -> Self {
        Self::Null
    }
}
impl<T: Into<Self>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Self::Array(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Self>> From<BTreeMap<String, T>> for Value {
    fn from(m: BTreeMap<String, T>) -> Self {
        Self::Map(m.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}
impl<T: Into<Self>> From<HashMap<String, T>> for Value {
    fn from(m: HashMap<String, T>) -> Self {
        Self::Map(m.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

/// The type kinds that can cross the host boundary.
///
/// Used to type-check a registered closure against its `host` block declaration.
/// `Array`/`Map` are recursive so nested shapes (e.g. `{string: string}[]`)
/// validate structurally; `Map` is always string-keyed, so only the value type
/// is carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostType {
    Int,
    Float,
    Bool,
    String,
    Unit,
    Array(Box<Self>),
    Map(Box<Self>),
}

impl HostType {
    /// Maps a candela [`DataType`] onto the host type kind it marshals as, or
    /// `None` for a type that cannot cross the boundary.
    #[must_use]
    pub fn from_datatype(dt: &DataType) -> Option<Self> {
        match dt {
            DataType::Int => Some(Self::Int),
            DataType::Float => Some(Self::Float),
            DataType::Bool => Some(Self::Bool),
            DataType::String => Some(Self::String),
            DataType::Null => Some(Self::Unit),
            DataType::Array(Some(inner)) => {
                Some(Self::Array(Box::new(Self::from_datatype(inner)?)))
            }
            DataType::Map(kv) => {
                // Only string-keyed maps cross the boundary.
                match &kv.0 {
                    Some(DataType::String) | None => {}
                    Some(_) => return None,
                }
                let value =
                    kv.1.as_ref()
                        .map_or(Some(Self::Unit), Self::from_datatype)?;
                Some(Self::Map(Box::new(value)))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Int => "int".to_owned(),
            Self::Float => "float".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::String => "string".to_owned(),
            Self::Unit => "null".to_owned(),
            Self::Array(inner) => format!("{}[]", inner.describe()),
            Self::Map(value) => format!("{{string: {}}}", value.describe()),
        }
    }
}

/// The type-erased closure the VM dispatches a `host` call to.
pub type HostDispatch = Rc<dyn Fn(&[Value]) -> Value>;

/// Extracts a Rust argument from a [`Value`] for a registered host closure.
pub trait FromHostValue: Sized {
    fn from_host_value(v: &Value) -> Self;
    fn host_type() -> HostType;
}

impl FromHostValue for i64 {
    fn from_host_value(v: &Value) -> Self {
        v.as_i64().unwrap_or(0)
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl FromHostValue for i32 {
    fn from_host_value(v: &Value) -> Self {
        v.as_i64().unwrap_or(0) as Self
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl FromHostValue for f64 {
    fn from_host_value(v: &Value) -> Self {
        v.as_f64().unwrap_or(0.0)
    }
    fn host_type() -> HostType {
        HostType::Float
    }
}
impl FromHostValue for bool {
    fn from_host_value(v: &Value) -> Self {
        v.as_bool().unwrap_or(false)
    }
    fn host_type() -> HostType {
        HostType::Bool
    }
}
impl FromHostValue for String {
    fn from_host_value(v: &Value) -> Self {
        v.as_str().unwrap_or_default().to_owned()
    }
    fn host_type() -> HostType {
        HostType::String
    }
}
impl<T: FromHostValue> FromHostValue for Vec<T> {
    fn from_host_value(v: &Value) -> Self {
        v.as_array()
            .map(|items| items.iter().map(T::from_host_value).collect())
            .unwrap_or_default()
    }
    fn host_type() -> HostType {
        HostType::Array(Box::new(T::host_type()))
    }
}
impl<T: FromHostValue> FromHostValue for BTreeMap<String, T> {
    fn from_host_value(v: &Value) -> Self {
        v.as_map()
            .map(|m| {
                m.iter()
                    .map(|(k, val)| (k.clone(), T::from_host_value(val)))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn host_type() -> HostType {
        HostType::Map(Box::new(T::host_type()))
    }
}
impl<T: FromHostValue, S: std::hash::BuildHasher + Default> FromHostValue
    for HashMap<String, T, S>
{
    fn from_host_value(v: &Value) -> Self {
        v.as_map()
            .map(|m| {
                m.iter()
                    .map(|(k, val)| (k.clone(), T::from_host_value(val)))
                    .collect()
            })
            .unwrap_or_default()
    }
    fn host_type() -> HostType {
        HostType::Map(Box::new(T::host_type()))
    }
}

/// Converts a host closure's return value into a [`Value`].
pub trait IntoHostValue {
    fn into_host_value(self) -> Value;
    fn host_type() -> HostType;
}

impl IntoHostValue for i64 {
    fn into_host_value(self) -> Value {
        Value::Int(self)
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl IntoHostValue for i32 {
    fn into_host_value(self) -> Value {
        Value::Int(i64::from(self))
    }
    fn host_type() -> HostType {
        HostType::Int
    }
}
impl IntoHostValue for f64 {
    fn into_host_value(self) -> Value {
        Value::Float(self)
    }
    fn host_type() -> HostType {
        HostType::Float
    }
}
impl IntoHostValue for bool {
    fn into_host_value(self) -> Value {
        Value::Bool(self)
    }
    fn host_type() -> HostType {
        HostType::Bool
    }
}
impl IntoHostValue for String {
    fn into_host_value(self) -> Value {
        Value::String(self)
    }
    fn host_type() -> HostType {
        HostType::String
    }
}
impl IntoHostValue for &str {
    fn into_host_value(self) -> Value {
        Value::String(self.to_owned())
    }
    fn host_type() -> HostType {
        HostType::String
    }
}
impl IntoHostValue for () {
    fn into_host_value(self) -> Value {
        Value::Null
    }
    fn host_type() -> HostType {
        HostType::Unit
    }
}
impl<T: IntoHostValue> IntoHostValue for Vec<T> {
    fn into_host_value(self) -> Value {
        Value::Array(
            self.into_iter()
                .map(IntoHostValue::into_host_value)
                .collect(),
        )
    }
    fn host_type() -> HostType {
        HostType::Array(Box::new(T::host_type()))
    }
}
impl<T: IntoHostValue> IntoHostValue for BTreeMap<String, T> {
    fn into_host_value(self) -> Value {
        Value::Map(
            self.into_iter()
                .map(|(k, v)| (k, v.into_host_value()))
                .collect(),
        )
    }
    fn host_type() -> HostType {
        HostType::Map(Box::new(T::host_type()))
    }
}
impl<T: IntoHostValue, S: std::hash::BuildHasher> IntoHostValue for HashMap<String, T, S> {
    fn into_host_value(self) -> Value {
        Value::Map(
            self.into_iter()
                .map(|(k, v)| (k, v.into_host_value()))
                .collect(),
        )
    }
    fn host_type() -> HostType {
        HostType::Map(Box::new(T::host_type()))
    }
}

/// Adapts a Rust closure into a registered host function.
///
/// The `Marker` type parameter disambiguates the blanket impls by arity (and,
/// for `&str`, by borrow), the same trick `rhai`/`bevy` use to make
/// `register_fn` accept closures of many shapes without annotations.
pub trait IntoHostFn<Marker> {
    /// Internal adapter: yields the erased dispatcher plus the argument and
    /// return type signature derived from the closure. Not meant to be called
    /// directly; `Engine::register_host_fn` drives it.
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType);
}

/// Marker for a nullary closure.
pub struct Arity0;
/// Marker for a closure whose sole parameter is a borrowed `&str`.
pub struct ArityStr1;

impl<F, R> IntoHostFn<Arity0> for F
where
    F: Fn() -> R + 'static,
    R: IntoHostValue,
{
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
        (
            Rc::new(move |_args: &[Value]| self().into_host_value()),
            Vec::new(),
            <R as IntoHostValue>::host_type(),
        )
    }
}

impl<F, R> IntoHostFn<ArityStr1> for F
where
    F: Fn(&str) -> R + 'static,
    R: IntoHostValue,
{
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
        (
            Rc::new(move |args: &[Value]| {
                let s = args.first().and_then(Value::as_str).unwrap_or_default();
                self(s).into_host_value()
            }),
            vec![HostType::String],
            <R as IntoHostValue>::host_type(),
        )
    }
}

/// Generates a `IntoHostFn` impl for an owned-argument closure of a given arity.
/// Each argument marker is the tuple of its parameter types, which keeps the
/// impls disjoint from one another and from the `&str`/nullary specializations.
macro_rules! impl_into_host_fn {
    ($($ty:ident $idx:tt),+) => {
        impl<F, R, $($ty,)+> IntoHostFn<($($ty,)+)> for F
        where
            F: Fn($($ty,)+) -> R + 'static,
            R: IntoHostValue,
            $( $ty: FromHostValue + 'static, )+
        {
            fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
                (
                    Rc::new(move |args: &[Value]| {
                        self( $( <$ty as FromHostValue>::from_host_value(&args[$idx]), )+ )
                            .into_host_value()
                    }),
                    vec![ $( <$ty as FromHostValue>::host_type(), )+ ],
                    <R as IntoHostValue>::host_type(),
                )
            }
        }
    };
}

impl_into_host_fn!(A0 0);
impl_into_host_fn!(A0 0, A1 1);
impl_into_host_fn!(A0 0, A1 1, A2 2);
impl_into_host_fn!(A0 0, A1 1, A2 2, A3 3);
impl_into_host_fn!(A0 0, A1 1, A2 2, A3 3, A4 4);

/// Allocates a [`Value`] into candela's heap pools and returns the handle [`Data`].
///
/// Scalars become NaN-boxed values directly; arrays/maps are pushed into the
/// object/map pools (nested structures allocated depth-first) and referenced by
/// handle. Strings go through [`Data::p_str`], which interns without triggering
/// the GC. This is safe because these direct pushes never invoke the collector, so
/// intermediate handles cannot be reclaimed mid-construction.
pub fn marshal_value(
    v: &Value,
    objs: &mut ObjectPool,
    maps: &mut MapPool,
    strings: &mut StringPool,
) -> Data {
    match v {
        Value::Null => NULL,
        Value::Int(i) => Data::int(*i as i32),
        Value::Float(f) => Data::float(*f),
        Value::Bool(b) => Data::bool(*b),
        Value::String(s) => Data::p_str(s, strings),
        Value::Array(items) => {
            let elems: Vec<Data> = items
                .iter()
                .map(|e| marshal_value(e, objs, maps, strings))
                .collect();
            let id = objs.len() as u32;
            objs.push(elems);
            Data::array(id)
        }
        Value::Map(entries) => {
            let mut map: CandelaMap = HashMap::default();
            for (k, val) in entries {
                let key = Data::p_str(k, strings);
                let value = marshal_value(val, objs, maps, strings);
                map.insert(key, value);
            }
            let id = maps.len() as u32;
            maps.push(map);
            Data::map(id)
        }
    }
}

/// Reads a runtime [`Data`] back into a host [`Value`], recursively for arrays,
/// maps and structs. Arrays and structs live in the shared object pool; structs
/// are surfaced as string-keyed [`Value::Map`]s using their declared field
/// names. Map keys that are not strings are skipped.
pub fn unmarshal_value(
    d: Data,
    objs: &ObjectPool,
    maps: &MapPool,
    strings: &StringPool,
    structs: &[Struct],
) -> Value {
    if d.is_int() {
        Value::Int(i64::from(d.as_int()))
    } else if d.is_bool() {
        Value::Bool(d.as_bool())
    } else if d.is_string() {
        Value::String(d.as_str(strings).to_owned())
    } else if d.is_null() {
        Value::Null
    } else if d.is_array() {
        let items = objs[d.as_array()]
            .iter()
            .map(|e| unmarshal_value(*e, objs, maps, strings, structs))
            .collect();
        Value::Array(items)
    } else if d.is_struct() {
        let fields = &structs[d.struct_type_id() as usize].fields;
        let values = &objs[d.as_struct()];
        let record = fields
            .iter()
            .zip(values.iter())
            .map(|((name, _, _), val)| {
                (
                    name.to_string(),
                    unmarshal_value(*val, objs, maps, strings, structs),
                )
            })
            .collect();
        Value::Map(record)
    } else if d.is_map() {
        let record = maps[d.as_map()]
            .iter()
            .filter(|(k, _)| k.is_string())
            .map(|(k, val)| {
                (
                    k.as_str(strings).to_owned(),
                    unmarshal_value(*val, objs, maps, strings, structs),
                )
            })
            .collect();
        Value::Map(record)
    } else if d.is_float() {
        Value::Float(d.as_float())
    } else {
        Value::Null
    }
}
