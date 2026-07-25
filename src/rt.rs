//! Runtime data types shared by the VM and the compiler.
//!
//! These definitions used to live inside the `compiler` module (in
//! `compiler_data`, `type_system`, and `expr`). They were extracted here so the
//! runtime core (`vm` + `data` + `gc` + `embed` + `artifact`) can be built with
//! `--no-default-features` -- i.e. WITHOUT linking the parser, the compiler, or
//! the REPL. The compiler module (gated behind the `compiler` feature)
//! re-exports every type from here, so its own source keeps its old import
//! paths (`compiler_data::Struct`, `type_system::DataType`, `expr::Span`).

use crate::instr::Instr;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::StringPool;
#[cfg(not(target_arch = "wasm32"))]
use libloading::Library;
use serde::Deserialize;
use serde::Serialize;
use smol_strc::SmolStr;
use std::hint::unreachable_unchecked;
#[cfg(not(target_arch = "wasm32"))]
use std::rc::Rc;

#[cfg(not(target_arch = "wasm32"))]
use libffi::middle::Type;

/// A span of code in a `Source`'s `contents`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[inline(always)]
    #[must_use]
    pub const fn extend(self, span: Self) -> Self {
        Self {
            start: self.start,
            end: span.end,
        }
    }
}

impl From<std::range::Range<usize>> for Span {
    #[inline(always)]
    fn from(value: std::range::Range<usize>) -> Self {
        Self {
            start: value.start as u32,
            end: value.end as u32,
        }
    }
}

impl From<std::ops::Range<usize>> for Span {
    #[inline(always)]
    fn from(value: std::ops::Range<usize>) -> Self {
        Self {
            start: value.start as u32,
            end: value.end as u32,
        }
    }
}

impl From<Span> for std::ops::Range<usize> {
    #[inline(always)]
    fn from(val: Span) -> Self {
        val.start as usize..val.end as usize
    }
}

impl From<(usize, usize)> for Span {
    #[inline(always)]
    fn from((start, end): (usize, usize)) -> Self {
        Self {
            start: start as u32,
            end: end as u32,
        }
    }
}

impl From<(u32, u32)> for Span {
    #[inline(always)]
    fn from((start, end): (u32, u32)) -> Self {
        Self { start, end }
    }
}

/// A resolved candela type. Attached to compiled artifacts (struct field types,
/// dynamic-library / host function signatures) so the VM can marshal values.
// `to_c_type` uses `unsafe` internally, which trips clippy's
// `unsafe_derive_deserialize`; the derive itself introduces no unsoundness (the
// deserialized value is an ordinary enum), so the lint is not applicable here.
#[allow(clippy::unsafe_derive_deserialize)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DataType {
    /// Array(None) = Unknown[]
    Array(Option<Box<Self>>),
    Float,
    Int,
    Bool,
    String,
    Null,
    Unknown,
    Union(Box<[Self]>),
    Fn(u16),
    Struct(u16),
    Map(Box<(Option<Self>, Option<Self>)>),
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Float => write!(f, "float"),
            Self::Int => write!(f, "int"),
            Self::Bool => write!(f, "bool"),
            Self::String => write!(f, "string"),
            Self::Array(array_type) => match array_type {
                Some(array_type) => write!(f, "{array_type}[]"),
                None => write!(f, "Unknown[]"),
            },
            Self::Null => write!(f, "null"),
            Self::Unknown => write!(f, "Unknown"),
            Self::Union(types) => write!(
                f,
                "{}",
                types
                    .into_iter()
                    .map(|x| format!("{x}"))
                    .collect::<Vec<_>>()
                    .join("|")
            ),
            Self::Struct(_) => write!(f, "struct"),
            Self::Map(m) => write!(
                f,
                "{{{}: {}}}",
                m.0.as_ref().unwrap_or(&Self::Unknown),
                m.1.as_ref().unwrap_or(&Self::Unknown)
            ),
            Self::Fn(_) => write!(f, "function"),
        }
    }
}

impl DataType {
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn to_c_type(&self, structs: &[Struct]) -> Type {
        match self {
            Self::Int => libffi::middle::Type::i32(),
            Self::Float => libffi::middle::Type::f64(),
            Self::String | Self::Array(_) => libffi::middle::Type::pointer(),
            Self::Null => libffi::middle::Type::void(),
            Self::Struct(id) => libffi::middle::Type::structure(
                structs[*id as usize]
                    .fields
                    .iter()
                    .map(|(_, field_type, _)| field_type.to_c_type(structs)),
            ),
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

#[derive(Debug)]
pub struct ErrorCatch {
    pub catch_loc: u32,
    pub error_reg: u16,
    pub call_frames_len: u32,
    pub args_len: u32,
}

/// Marshalling signature for a `host` function, indexed by
/// [`crate::compiler::compiler_data::FnSignature::id`]. `types[0]` is the return
/// type; `types[1..]` are the argument types.
#[derive(Debug, Clone)]
pub struct HostFnSig {
    /// [ return_type, arg_types... ]
    pub types: Box<[DataType]>,
    /// The `(namespace, name)` this signature was declared under, used by the
    /// `Engine` to bind the matching registered closure.
    pub namespace: SmolStr,
    pub name: SmolStr,
    /// Declared with `...` in the `host` block: the call site forwards any
    /// number of arguments of any type to the registered closure and the
    /// `Engine` skips signature validation. `types` holds only the return type.
    pub variadic: bool,
}

impl HostFnSig {
    #[inline(always)]
    #[must_use]
    pub fn get_return_type(&self) -> &DataType {
        unsafe { self.types.get_unchecked(0) }
    }
    #[inline(always)]
    #[must_use]
    pub fn get_arg(&self, idx: usize) -> &DataType {
        unsafe { self.types.get_unchecked(1 + idx) }
    }
    #[inline(always)]
    #[must_use]
    pub fn arg_count(&self) -> usize {
        self.types.len() - 1
    }
}

#[derive(Debug)]
#[allow(clippy::pub_underscore_fields)]
pub struct DynamicLibFn {
    /// [ return_type, arg_types... ]
    pub types: Box<[DataType]>,
    // Keeps the loaded library alive for as long as its function pointers are
    // callable; `pub` so the compiler can construct it, `_`-prefixed because it
    // is never read directly.
    #[cfg(not(target_arch = "wasm32"))]
    pub _lib: Rc<Library>,
    #[cfg(not(target_arch = "wasm32"))]
    pub ptr: libffi::middle::CodePtr,
    #[cfg(not(target_arch = "wasm32"))]
    pub cif: libffi::middle::Cif,
}

impl DynamicLibFn {
    #[inline(always)]
    #[must_use]
    pub fn get_return_type(&self) -> &DataType {
        unsafe { self.types.get_unchecked(0) }
    }
}

#[derive(Debug)]
pub struct Struct {
    pub name: SmolStr,
    pub fields: Box<[(SmolStr, DataType, Span)]>,
    pub id: u16,
    pub name_span: Span,
}

pub struct Pools {
    pub objs: ObjectPool,
    pub maps: MapPool,
    pub strings: StringPool,
}

pub struct Source {
    pub filename: SmolStr,
    pub contents: String,
}

#[derive(Copy, Clone)]
pub struct InstrSrc {
    pub instr: Instr,
    pub span: Span,
    pub file_id: u16,
}
