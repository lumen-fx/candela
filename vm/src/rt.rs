//! Runtime data types shared by the VM and the compiler.
//!
//! These live in the self-contained `candela-vm` crate so the runtime core
//! (`vm` + `data` + `gc` + `embed` + `artifact`) links without the parser,
//! compiler, or REPL. The `candela` compiler crate depends on this crate and
//! re-exports these types (aliasing `candela_vm::rt` as `crate::rt`, etc.), so
//! its own source keeps its import paths (`compiler_data::Struct`,
//! `type_system::DataType`, `expr::Span`).

use crate::instr::Instr;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::StringPool;
#[cfg(not(target_arch = "wasm32"))]
use libloading::Library;
use serde::Deserialize;
use serde::Serialize;
use smol_strc::SmolStr;
use std::cell::RefCell;
use std::hint::unreachable_unchecked;
use std::path::PathBuf;
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
    Enum(u16),
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
            Self::Enum(_) => write!(f, "enum"),
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
            // Enums (and maps, unions, functions) have no C representation, so
            // they cannot appear in a `dylib`/`host` signature. Reaching here is
            // a defined panic rather than undefined behavior.
            other => {
                panic!("type {other} has no C representation and cannot cross the FFI boundary")
            }
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
/// the compiler's `FnSignature::id`. `types[0]` is the return
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
    /// The library spec exactly as written in the source `dylib "..."` block: a
    /// bare logical name (`z`, `sqlite3`) or a path. Recorded so a `.cdlb`
    /// artifact can re-resolve the library by name at load time; it is not read
    /// on the live call path (the resolved `_lib`/`ptr`/`cif` below are).
    pub library: SmolStr,
    /// The C symbol this binding resolves to. Recorded alongside `library` so a
    /// `.cdlb` can re-bind the symbol at load without the source tree.
    pub symbol: SmolStr,
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

/// Target operating system for dynamic-library filename resolution.
///
/// Kept explicit, rather than only branching on `cfg!(target_os)` at the call
/// site, so the logical-name -> filename mapping ([`resolve_library_filename`])
/// can be unit-tested for every platform on any build host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    Linux,
    Macos,
    Windows,
}

impl TargetOs {
    /// The OS this runtime was built for. Targets that are none of
    /// Linux/macOS/Windows fall back to the Linux convention.
    pub const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Macos
    } else if cfg!(target_os = "windows") {
        Self::Windows
    } else {
        Self::Linux
    };

    /// Dynamic-library filename extension for this OS (no leading dot).
    #[must_use]
    pub const fn dynamic_lib_extension(self) -> &'static str {
        match self {
            Self::Linux => "so",
            Self::Macos => "dylib",
            Self::Windows => "dll",
        }
    }

    /// Filename prefix a bare logical name gets on this OS (`lib` on
    /// Linux/macOS, none on Windows).
    #[must_use]
    pub const fn dynamic_lib_prefix(self) -> &'static str {
        match self {
            Self::Linux | Self::Macos => "lib",
            Self::Windows => "",
        }
    }
}

/// Maps a candela `dylib`/`import` library spec to a concrete filename for `os`.
///
/// A bare logical name (`m`, `sqlite3`, `z`), no path separator and no
/// extension, becomes the platform convention: `libm.so` on Linux,
/// `libm.dylib` on macOS, `m.dll` on Windows. The same candela source therefore
/// names the right file on every target, and the OS loader searches its standard
/// library paths for it.
///
/// A spec that already carries a path separator or an explicit extension is
/// treated as an explicit path and honored: an extension is kept as-is; a path
/// with no extension is completed with the platform extension (and gets no `lib`
/// prefix). This keeps absolute and workspace-relative library paths working
/// across all three targets.
#[must_use]
pub fn resolve_library_filename(spec: &str, os: TargetOs) -> String {
    let has_separator = spec.contains('/') || spec.contains('\\');
    let has_extension = std::path::Path::new(spec).extension().is_some();
    if has_extension {
        spec.to_owned()
    } else if has_separator {
        format!("{spec}.{}", os.dynamic_lib_extension())
    } else {
        format!(
            "{}{spec}.{}",
            os.dynamic_lib_prefix(),
            os.dynamic_lib_extension()
        )
    }
}

thread_local! {
    static DYLIB_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Sets the directory a `dylib` import's library is looked for in on this
/// thread, and returns the one that was in effect, so a caller can put it back.
///
/// A host whose native libraries do not sit beside the script names their
/// directory here: an application that keeps its candela sources in `src/` and
/// its shared libraries in `lib/` points this at `lib/`, and `dylib "md"` finds
/// `libmd.so` there. Passing `None` goes back to searching beside the importing
/// file only.
///
/// The directory is searched first; whatever the library name resolved to
/// before is still tried after it, so a program that names a system library
/// keeps working. The setting is read when a program is compiled and when a
/// `.cdlb` artifact is loaded, so set it before either.
pub fn set_dylib_dir(dir: Option<PathBuf>) -> Option<PathBuf> {
    DYLIB_DIR.replace(dir)
}

/// The directory [`set_dylib_dir`] put in effect on this thread, if any.
#[must_use]
pub fn dylib_dir() -> Option<PathBuf> {
    DYLIB_DIR.with(|dir| dir.borrow().clone())
}

/// Opens `filename` under the [`dylib_dir`] directory when one is set and the
/// file is there, and hands the name to the OS loader otherwise.
///
/// The loader's own error comes back, so a caller reports the same message it
/// did before a directory was ever set.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_library(filename: &str) -> Result<Library, libloading::Error> {
    if let Some(dir) = dylib_dir()
        && let Ok(lib) = unsafe { Library::new(dir.join(filename)) }
    {
        return Ok(lib);
    }
    unsafe { Library::new(filename) }
}

#[cfg(test)]
mod dynlib_tests {
    use super::TargetOs;
    use super::resolve_library_filename;

    #[test]
    fn logical_name_maps_to_each_os_convention() {
        assert_eq!(resolve_library_filename("m", TargetOs::Linux), "libm.so");
        assert_eq!(resolve_library_filename("m", TargetOs::Macos), "libm.dylib");
        assert_eq!(resolve_library_filename("m", TargetOs::Windows), "m.dll");

        assert_eq!(
            resolve_library_filename("sqlite3", TargetOs::Linux),
            "libsqlite3.so"
        );
        assert_eq!(
            resolve_library_filename("sqlite3", TargetOs::Macos),
            "libsqlite3.dylib"
        );
        assert_eq!(
            resolve_library_filename("sqlite3", TargetOs::Windows),
            "sqlite3.dll"
        );
    }

    #[test]
    fn explicit_extension_is_honored_on_every_os() {
        for os in [TargetOs::Linux, TargetOs::Macos, TargetOs::Windows] {
            assert_eq!(resolve_library_filename("libfoo.so.6", os), "libfoo.so.6");
            assert_eq!(
                resolve_library_filename("/usr/lib/libz.so", os),
                "/usr/lib/libz.so"
            );
            assert_eq!(resolve_library_filename("plugin.dll", os), "plugin.dll");
        }
    }

    #[test]
    fn path_without_extension_gets_platform_extension_and_no_prefix() {
        assert_eq!(
            resolve_library_filename("../std_src/math/math", TargetOs::Linux),
            "../std_src/math/math.so"
        );
        assert_eq!(
            resolve_library_filename("../std_src/math/math", TargetOs::Macos),
            "../std_src/math/math.dylib"
        );
        assert_eq!(
            resolve_library_filename("./plugins/foo", TargetOs::Windows),
            "./plugins/foo.dll"
        );
    }
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

/// A native tagged-union enum type. Its values are stored in the object pool
/// like structs: element 0 of the pool entry is the variant tag (an int index
/// into `variants`), and elements `1..` are the variant's payload in order.
#[derive(Debug)]
pub struct EnumType {
    pub name: SmolStr,
    pub variants: Box<[EnumVariant]>,
    pub id: u16,
    pub name_span: Span,
}

#[derive(Debug)]
pub struct EnumVariant {
    pub name: SmolStr,
    /// Payload field types, in declaration order. Empty for a nullary variant.
    pub payload: Box<[DataType]>,
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

impl DataType {
    #[inline(always)]
    #[must_use]
    pub const fn is_indexable(&self) -> bool {
        matches!(self, Self::String | Self::Array(_) | Self::Unknown)
    }

    /// Collapses a union of return types to a single type when they all agree
    /// (ignoring `Unknown`), or to `Unknown` when only unknowns remain.
    #[must_use]
    pub fn check_poly(self) -> Self {
        if let Self::Union(ref elems) = self {
            if let Some(new) = reduce_null_struct(elems) {
                return new;
            }
            let mut concrete = elems
                .iter()
                .filter(|elem_type| **elem_type != Self::Unknown);
            if let Some(first_type) = concrete.next() {
                if concrete.all(|x| x == first_type) {
                    first_type.clone()
                } else {
                    self
                }
            } else if !elems.is_empty() {
                Self::Unknown
            } else {
                unsafe { unreachable_unchecked() }
            }
        } else {
            unsafe { unreachable_unchecked() }
        }
    }
}

fn reduce_null_struct(types: &[DataType]) -> Option<DataType> {
    let mut struct_type = None;
    for t in types {
        match t {
            DataType::Null | DataType::Unknown => {}
            DataType::Struct(_) => {
                if let Some(struct_type) = &struct_type {
                    if struct_type != t {
                        return None;
                    }
                } else {
                    struct_type = Some(t.clone());
                }
            }
            _ => return None,
        }
    }
    struct_type
}

impl PartialEq for DataType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // Array(None) is compatible with any array type
            (Self::Float, Self::Float)
            | (Self::Int, Self::Int)
            | (Self::Bool, Self::Bool)
            | (Self::String, Self::String)
            | (Self::Null, Self::Null)
            | (Self::Unknown, Self::Unknown)
            | (Self::Array(_), Self::Array(None))
            | (Self::Array(None), Self::Array(_)) => true,
            (Self::Array(Some(a)), Self::Array(Some(b))) => a == b,
            (Self::Union(a), Self::Union(b)) => a == b,
            (Self::Struct(a), Self::Struct(b)) => a == b,
            (Self::Enum(a), Self::Enum(b)) => a == b,
            (Self::Fn(_), Self::Fn(_)) => true,
            (Self::Map(a), Self::Map(b)) => {
                (a.0.is_none() || b.0.is_none() || a.0 == b.0)
                    && (a.1.is_none() || b.1.is_none() || a.1 == b.1)
            }
            (t, Self::Union(p)) | (Self::Union(p), t) => p.contains(t),
            _ => false,
        }
    }
}

impl std::hash::Hash for DataType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // All Array variants hash identically, which is required because Array(None) == Array(Some(_))
        match self {
            Self::Array(_) => 0u8.hash(state),
            Self::Float => 1u8.hash(state),
            Self::Int => 2u8.hash(state),
            Self::Bool => 3u8.hash(state),
            Self::String => 4u8.hash(state),
            Self::Null => 6u8.hash(state),
            Self::Unknown => 7u8.hash(state),
            Self::Union(p) => {
                8u8.hash(state);
                p.hash(state);
            }
            Self::Fn(f) => {
                9u8.hash(state);
                f.hash(state);
            }
            Self::Struct(s) => {
                10u8.hash(state);
                s.hash(state);
            }
            Self::Enum(e) => {
                12u8.hash(state);
                e.hash(state);
            }
            Self::Map(m) => {
                11u8.hash(state);
                m.hash(state);
            }
        }
    }
}
