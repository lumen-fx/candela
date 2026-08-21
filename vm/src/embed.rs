//! Host/script value marshalling for candela's embedding API.
//!
//! These types cross the boundary between a Rust host and a running candela
//! script: [`Value`] is the dynamically-typed carrier, [`HostType`] describes
//! the shapes that can cross, the `FromHostValue`/`IntoHostValue`/
//! `IntoHostResult`/`IntoHostFn` traits adapt Rust closures into registered
//! host functions, [`HostError`] is how one of them fails, [`HostRegistry`]
//! holds the closures a script's `host` blocks bind to, and
//! [`marshal_value`]/[`unmarshal_value`] convert between [`Value`] and the VM's
//! NaN-boxed [`Data`]. They carry no compiler state and the VM depends on them,
//! so they live in the VM-only crate. The `Engine`/`Program` embedding surface
//! (which compiles and drives scripts) lives in the `candela` crate on top.

use crate::data::Data;
use crate::data::DataHash;
use crate::data::NULL;
use crate::rt::DataType;
use crate::rt::HostFnSig;
use crate::rt::Struct;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::StringPool;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
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

/// Whether a declared parameter type can be filled from a host [`Value`].
///
/// This is the gate `candela build` applies when it decides which functions get
/// a host-callable export, and it admits exactly the types
/// [`value_matches_type`] can check an argument against. Struct, enum and
/// function parameters are left out: a host has no way to build one.
#[must_use]
pub fn is_host_callable_type(ty: &DataType) -> bool {
    match ty {
        DataType::Int
        | DataType::Float
        | DataType::Bool
        | DataType::String
        | DataType::Null
        | DataType::Unknown => true,
        DataType::Array(element) => element.as_deref().is_none_or(is_host_callable_type),
        DataType::Map(kv) => {
            matches!(&kv.0, None | Some(DataType::String))
                && kv.1.as_ref().is_none_or(is_host_callable_type)
        }
        DataType::Union(members) => members.iter().all(is_host_callable_type),
        DataType::Fn(_) | DataType::Struct(_) | DataType::Enum(_) => false,
    }
}

/// Whether a host [`Value`] satisfies a declared parameter type.
///
/// `any` (candela's [`DataType::Unknown`]) accepts every value; a union accepts
/// a value any member accepts; an unparameterised array or map accepts any
/// array or map, because the element type was never pinned. Paired with
/// [`is_host_callable_type`], which decides which parameter types reach here at
/// all.
#[must_use]
pub fn value_matches_type(value: &Value, ty: &DataType) -> bool {
    match ty {
        DataType::Unknown => true,
        DataType::Int => matches!(value, Value::Int(_)),
        DataType::Float => matches!(value, Value::Float(_)),
        DataType::Bool => matches!(value, Value::Bool(_)),
        DataType::String => matches!(value, Value::String(_)),
        DataType::Null => matches!(value, Value::Null),
        DataType::Array(element) => match value {
            Value::Array(items) => element
                .as_deref()
                .is_none_or(|el| items.iter().all(|item| value_matches_type(item, el))),
            _ => false,
        },
        DataType::Map(kv) => match value {
            Value::Map(entries) => {
                kv.1.as_ref()
                    .is_none_or(|v| entries.values().all(|entry| value_matches_type(entry, v)))
            }
            _ => false,
        },
        DataType::Union(members) => members
            .iter()
            .any(|member| value_matches_type(value, member)),
        DataType::Fn(_) | DataType::Struct(_) | DataType::Enum(_) => false,
    }
}

/// The type-erased closure the VM dispatches a `host` call to.
pub type HostDispatch = Rc<dyn Fn(&[Value]) -> Result<Value, HostError>>;

/// Why a registered host function failed.
///
/// A closure returns this in place of a value to raise inside the script that
/// called it. The VM reports it the way it reports its own runtime failures,
/// naming the function and pointing at the call, and a script can catch it
/// under the kind `host_fn_error`.
///
/// [`HostError::new`] takes anything that renders, so an error from the work
/// the closure was doing propagates with `map_err(HostError::new)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    message: String,
}

impl HostError {
    /// Builds an error from anything that renders.
    pub fn new(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
        }
    }

    /// What the host reported, as the diagnostic will read it.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<String> for HostError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for HostError {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HostError {}

/// A registered host function: its erased dispatcher plus the argument/return
/// type signature derived from the closure, used to validate it against the
/// `host` block that declares it.
struct RegisteredFn {
    func: HostDispatch,
    arg_types: Vec<HostType>,
    ret_type: HostType,
    /// Registered through [`HostRegistry::register_host_fn_variadic`]: the
    /// closure takes a `&[Value]` slice of any length, so `arg_types`/`ret_type`
    /// are unused and signature validation is skipped (the block must declare
    /// the function with `...`).
    variadic: bool,
}

/// The table of Rust closures a script's `host` blocks bind to.
///
/// Both halves of the toolchain bind through this one table. The `candela`
/// crate's `Engine` binds at compile time, against the `host` signatures a
/// fresh compile produced; [`crate::artifact::load_program`] binds at load
/// time, against the signatures a `.cdlb` recorded. The checks are the same in
/// both cases, so an artifact that loads is bound as strictly as a script that
/// compiles.
#[derive(Default)]
pub struct HostRegistry {
    fns: HashMap<(String, String), RegisteredFn>,
}

impl HostRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a typed host function under `namespace::name`.
    ///
    /// The closure may take any combination of `i64`/`i32`, `f64`, `bool`,
    /// `String` (or a single `&str`), `Vec<T>` and string-keyed map arguments,
    /// and return one of those or `()`. A closure that can fail returns
    /// `Result<T, HostError>` instead, and the error is raised in the script
    /// that called it. The derived types are checked against the `host` block
    /// when the script is bound; a mismatch is an error, never a panic.
    pub fn register_host_fn<Marker, F>(&mut self, namespace: &str, name: &str, f: F)
    where
        F: IntoHostFn<Marker>,
    {
        let (func, arg_types, ret_type) = f.into_host_fn_parts();
        self.fns.insert(
            (namespace.to_owned(), name.to_owned()),
            RegisteredFn {
                func,
                arg_types,
                ret_type,
                variadic: false,
            },
        );
    }

    /// Registers a host function whose signature is given as data.
    ///
    /// The closure takes the arguments as a `&[Value]` slice, as a variadic one
    /// does, but `arg_types` and `ret_type` pin the signature, so the binding is
    /// checked against the `host` block exactly as [`register_host_fn`] is and
    /// the declaration must not use `...`. This is what a host registers
    /// through when the signature is only known at run time: a plugin table, a
    /// generated binding, anything with no Rust type to derive it from.
    ///
    /// [`register_host_fn`]: HostRegistry::register_host_fn
    pub fn register_host_fn_typed<F>(
        &mut self,
        namespace: &str,
        name: &str,
        arg_types: Vec<HostType>,
        ret_type: HostType,
        f: F,
    ) where
        F: Fn(&[Value]) -> Result<Value, HostError> + 'static,
    {
        self.fns.insert(
            (namespace.to_owned(), name.to_owned()),
            RegisteredFn {
                func: Rc::new(f),
                arg_types,
                ret_type,
                variadic: false,
            },
        );
    }

    /// Registers a variadic host function under `namespace::name`.
    ///
    /// The closure receives every argument as a `&[Value]` slice of any length
    /// and returns one [`Value`], or a [`HostError`] to raise in the script that
    /// called it, so arguments of mixed or dynamically-typed shape cross the
    /// boundary without a fixed Rust signature. The `host` block must declare
    /// the function with a `...` argument list, and no arity or per-argument
    /// type checking happens at the call site.
    pub fn register_host_fn_variadic<F>(&mut self, namespace: &str, name: &str, f: F)
    where
        F: Fn(&[Value]) -> Result<Value, HostError> + 'static,
    {
        self.fns.insert(
            (namespace.to_owned(), name.to_owned()),
            RegisteredFn {
                func: Rc::new(f),
                arg_types: Vec::new(),
                ret_type: HostType::Unit,
                variadic: true,
            },
        );
    }

    /// Binds every declared `host` signature to its registered closure,
    /// returning the dispatchers in host-function id order.
    ///
    /// # Errors
    ///
    /// Returns [`HostBindError::Unregistered`] naming every declared function
    /// with no closure behind it, or [`HostBindError::SignatureMismatch`] for
    /// the first closure whose arity or types disagree with the declaration.
    pub fn bind(&self, sigs: &[HostFnSig]) -> Result<Vec<HostDispatch>, HostBindError> {
        let missing: Vec<String> = sigs
            .iter()
            .filter(|sig| !self.fns.contains_key(&key_of(sig)))
            .map(|sig| qualified_name(&sig.namespace, &sig.name))
            .collect();
        if !missing.is_empty() {
            return Err(HostBindError::Unregistered(missing));
        }

        let mut dispatch = Vec::with_capacity(sigs.len());
        for sig in sigs {
            let registered = &self.fns[&key_of(sig)];
            validate_host_fn(sig, registered)?;
            dispatch.push(Rc::clone(&registered.func));
        }
        Ok(dispatch)
    }
}

/// The lookup key a `host` signature binds under.
fn key_of(sig: &HostFnSig) -> (String, String) {
    (sig.namespace.to_string(), sig.name.to_string())
}

/// Renders a host function the way a script writes the call.
pub(crate) fn qualified_name(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_owned()
    } else {
        format!("{namespace}::{name}")
    }
}

/// Checks that a registered closure's derived signature matches the `host`
/// block declaration it is bound to.
fn validate_host_fn(sig: &HostFnSig, registered: &RegisteredFn) -> Result<(), HostBindError> {
    let function = qualified_name(&sig.namespace, &sig.name);
    let err = HostBindError::SignatureMismatch;

    // A variadic declaration must be bound to a variadic closure and vice
    // versa; when both agree there is nothing to check, the closure accepts
    // any argument slice.
    if sig.variadic || registered.variadic {
        if sig.variadic != registered.variadic {
            let (decl, reg) = if sig.variadic {
                ("variadic (`...`)", "a fixed signature")
            } else {
                ("a fixed signature", "variadic")
            };
            return Err(err(format!(
                "host function `{function}` is declared with {decl} but the registered closure has {reg}",
            )));
        }
        return Ok(());
    }

    if sig.arg_count() != registered.arg_types.len() {
        return Err(err(format!(
            "host function `{function}` is declared with {} argument(s) but the registered closure takes {}",
            sig.arg_count(),
            registered.arg_types.len(),
        )));
    }

    for (idx, want) in registered.arg_types.iter().enumerate() {
        let declared = HostType::from_datatype(sig.get_arg(idx)).ok_or_else(|| {
            err(format!(
                "host function `{function}` argument {} has a type that cannot cross the host boundary",
                idx + 1,
            ))
        })?;
        if declared != *want {
            return Err(err(format!(
                "host function `{function}` argument {} is declared `{}` but the registered closure expects `{}`",
                idx + 1,
                declared.describe(),
                want.describe(),
            )));
        }
    }

    let declared_ret = HostType::from_datatype(sig.get_return_type()).ok_or_else(|| {
        err(format!(
            "host function `{function}` has a return type that cannot cross the host boundary",
        ))
    })?;
    if declared_ret != registered.ret_type {
        return Err(err(format!(
            "host function `{function}` is declared to return `{}` but the registered closure returns `{}`",
            declared_ret.describe(),
            registered.ret_type.describe(),
        )));
    }

    Ok(())
}

/// Why a script's `host` blocks could not be bound to a [`HostRegistry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostBindError {
    /// Declared `host` functions with no registered closure, in declaration
    /// order.
    Unregistered(Vec<String>),
    /// A registered closure disagrees with the declaration it is bound to. The
    /// text names the function and what differs.
    SignatureMismatch(String),
}

impl HostBindError {
    /// The stable identifier a `Diagnostic` reports this under.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unregistered(_) => "unregistered_host_fn",
            Self::SignatureMismatch(_) => "host_fn_signature_mismatch",
        }
    }
}

impl fmt::Display for HostBindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unregistered(names) => {
                if let [name] = names.as_slice() {
                    write!(
                        f,
                        "no host function registered for `{name}` (declared in a `host` block)"
                    )
                } else {
                    write!(
                        f,
                        "no host functions registered for `{}` (declared in `host` blocks)",
                        names.join("`, `")
                    )
                }
            }
            Self::SignatureMismatch(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for HostBindError {}

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

/// What a registered host closure returns: a value, or an error to raise in the
/// script that called it.
///
/// Implemented for every [`IntoHostValue`], so an infallible closure returns a
/// plain `i64`/`String`/`Vec<T>`/..., and for `Result<T, HostError>`, so a
/// fallible one returns that instead. Either way the host type checked against
/// the `host` block is `T`'s.
pub trait IntoHostResult {
    fn into_host_result(self) -> Result<Value, HostError>;
    fn host_type() -> HostType;
}

impl<T: IntoHostValue> IntoHostResult for T {
    fn into_host_result(self) -> Result<Value, HostError> {
        Ok(self.into_host_value())
    }
    fn host_type() -> HostType {
        <T as IntoHostValue>::host_type()
    }
}

impl<T: IntoHostValue> IntoHostResult for Result<T, HostError> {
    fn into_host_result(self) -> Result<Value, HostError> {
        self.map(IntoHostValue::into_host_value)
    }
    fn host_type() -> HostType {
        <T as IntoHostValue>::host_type()
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
    R: IntoHostResult,
{
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
        (
            Rc::new(move |_args: &[Value]| self().into_host_result()),
            Vec::new(),
            <R as IntoHostResult>::host_type(),
        )
    }
}

impl<F, R> IntoHostFn<ArityStr1> for F
where
    F: Fn(&str) -> R + 'static,
    R: IntoHostResult,
{
    fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
        (
            Rc::new(move |args: &[Value]| {
                let s = args.first().and_then(Value::as_str).unwrap_or_default();
                self(s).into_host_result()
            }),
            vec![HostType::String],
            <R as IntoHostResult>::host_type(),
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
            R: IntoHostResult,
            $( $ty: FromHostValue + 'static, )+
        {
            fn into_host_fn_parts(self) -> (HostDispatch, Vec<HostType>, HostType) {
                (
                    Rc::new(move |args: &[Value]| {
                        self( $( <$ty as FromHostValue>::from_host_value(&args[$idx]), )+ )
                            .into_host_result()
                    }),
                    vec![ $( <$ty as FromHostValue>::host_type(), )+ ],
                    <R as IntoHostResult>::host_type(),
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
