//! What a package's native library needs from the candela runtime that loads
//! it, on the Rust side of the C boundary.
//!
//! A library exports [`candela_dylib_init!`]'s function, and the runtime calls
//! it with its event table when the library loads. From then on [`post`],
//! [`hold`] and [`release`] reach the program's event queue from any thread,
//! and [`resolve`] joins a relative path onto the directory the embedder set
//! as the base.
//!
//! Strings cross the boundary as NUL-terminated C strings. One handed to the
//! library is borrowed for the call; read it with [`arg`]. One the library
//! returns is copied by the runtime and never freed, so [`ret`] keeps it in a
//! per-thread buffer that the next return on the same thread replaces.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, Ordering};

/// The version of the table this library was written against.
pub const ABI: u32 = 1;

/// The table the runtime hands `candela_dylib_init`. Mirrors the runtime's
/// own definition field for field; the layout test holds the two together.
#[repr(C)]
pub struct EventTable {
    pub abi: u32,
    pub new_source: extern "C" fn() -> i64,
    pub post: extern "C" fn(i64, *const c_char),
    pub hold: extern "C" fn(i64),
    pub release: extern "C" fn(i64),
    pub base_dir: extern "C" fn() -> *const c_char,
}

static TABLE: AtomicPtr<EventTable> = AtomicPtr::new(std::ptr::null_mut());

/// Keeps the table the runtime handed over, when its version is one this
/// library speaks. Called by the export [`candela_dylib_init!`] defines.
///
/// # Safety
///
/// `table` is null or points at a table that lives as long as the process,
/// which is what the runtime promises.
pub unsafe fn init(table: *const EventTable) {
    if table.is_null() {
        return;
    }
    if unsafe { (*table).abi } == ABI {
        TABLE.store(table.cast_mut(), Ordering::Release);
    }
}

fn table() -> Option<&'static EventTable> {
    let table = TABLE.load(Ordering::Acquire);
    // A table, once stored, lives as long as the process.
    unsafe { table.as_ref() }
}

/// Defines the `candela_dylib_init` export, which hands the runtime's table
/// to [`init`]. Write it once at the top level of the library.
#[macro_export]
macro_rules! candela_dylib_init {
    () => {
        /// Receives the candela runtime's event table when the library loads.
        ///
        /// # Safety
        ///
        /// Called by the runtime with a table that lives as long as the
        /// process.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn candela_dylib_init(table: *const $crate::EventTable) {
            unsafe { $crate::init(table) };
        }
    };
}

/// Queues `payload` for the handler of `source`. Does nothing when the library
/// was loaded by something other than a candela runtime.
pub fn post(source: i64, payload: &str) {
    if let Some(t) = table() {
        // A payload is text the library built; an interior NUL would cut it
        // short, so it is replaced rather than lost.
        let text = CString::new(payload.replace('\0', "\u{fffd}")).unwrap_or_default();
        (t.post)(source, text.as_ptr());
    }
}

/// Marks `source` as having work still running.
pub fn hold(source: i64) {
    if let Some(t) = table() {
        (t.hold)(source);
    }
}

/// Ends one hold on `source`, behind every event already posted under it.
pub fn release(source: i64) {
    if let Some(t) = table() {
        (t.release)(source);
    }
}

/// A new source no other source shares.
#[must_use]
pub fn new_source() -> i64 {
    table().map_or(0, |t| (t.new_source)())
}

/// `path` as the runtime would resolve it: joined onto the embedder's base
/// directory when it is relative and one is set.
#[must_use]
pub fn resolve(path: &str) -> PathBuf {
    let given = Path::new(path);
    if given.is_relative()
        && let Some(t) = table()
    {
        let base = unsafe { CStr::from_ptr((t.base_dir)()) }.to_string_lossy();
        if !base.is_empty() {
            return Path::new(base.as_ref()).join(given);
        }
    }
    given.to_path_buf()
}

/// The string argument at `ptr`, borrowed for the call. A null pointer reads
/// as empty, and bytes that are not UTF-8 are replaced.
///
/// # Safety
///
/// `ptr` is null or points at a NUL-terminated string that outlives the call,
/// which is what the runtime passes.
#[must_use]
pub unsafe fn arg(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

thread_local! {
    static RETURNED: RefCell<CString> = RefCell::new(CString::default());
}

/// Hands `text` back to the runtime as the call's string result. The runtime
/// copies it before this thread returns another.
#[must_use]
pub fn ret(text: &str) -> *const c_char {
    let owned = CString::new(text.replace('\0', "\u{fffd}")).unwrap_or_default();
    RETURNED.with(|slot| {
        *slot.borrow_mut() = owned;
        slot.borrow().as_ptr()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The table here and the runtime's are one C struct, so every field sits
    /// at the same offset in both.
    #[test]
    fn the_table_matches_the_runtime_s() {
        use candela_vm::events::EventTable as Runtime;
        assert_eq!(size_of::<EventTable>(), size_of::<Runtime>());
        assert_eq!(offset_of!(EventTable, abi), offset_of!(Runtime, abi));
        assert_eq!(
            offset_of!(EventTable, new_source),
            offset_of!(Runtime, new_source)
        );
        assert_eq!(offset_of!(EventTable, post), offset_of!(Runtime, post));
        assert_eq!(offset_of!(EventTable, hold), offset_of!(Runtime, hold));
        assert_eq!(
            offset_of!(EventTable, release),
            offset_of!(Runtime, release)
        );
        assert_eq!(
            offset_of!(EventTable, base_dir),
            offset_of!(Runtime, base_dir)
        );
        assert_eq!(ABI, candela_vm::events::EVENT_ABI);
    }

    #[test]
    fn a_returned_string_is_readable_until_the_next_one() {
        let ptr = ret("first");
        assert_eq!(unsafe { arg(ptr) }, "first");
        let ptr = ret("second");
        assert_eq!(unsafe { arg(ptr) }, "second");
    }
}
