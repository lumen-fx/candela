// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::captured_output::out;
use crate::captured_output::outln;
use crate::data::Data;
use crate::data::DataHash;
use crate::data::FALSE;
use crate::data::FUNCTION_TEXT;
use crate::data::NULL;
use crate::data::TRUE;
use crate::data::format_float;
use crate::embed::HostDispatch;
use crate::embed::Value;
use crate::errors::ErrType;
use crate::errors::ErrorCtx;
use crate::errors::RuntimeError;
use crate::errors::call_site_name;
use crate::gc::GcState;
use crate::gc::MarkBits;
use crate::gc::alloc_array;
use crate::gc::alloc_map;
use crate::gc::rescan_list;
use crate::gc::rescan_map;
use crate::gc::shade_overwritten;
use crate::instr::Instr;
use crate::instr::LibFunc;
use crate::instr::LibFuncVoid;
use crate::instr::type_code;
use crate::rt::DataType;
use crate::rt::DynamicLibFn;
use crate::rt::EnumType;
use crate::rt::ErrorCatch;
use crate::rt::HostFnSig;
use crate::rt::Pools;
use crate::rt::Span;
use crate::rt::Struct;
use indexmap::IndexMap;
use lexical_core::FormattedSize;
use memchr::memmem;
use smol_strc::ToSmolStr;
use std::cell::Cell;
use std::fs;
use std::hash::BuildHasherDefault;
use std::hint::cold_path;
use std::hint::unreachable_unchecked;
use std::io::Write;
use std::ops::Index;
use std::ops::IndexMut;

#[path = "ffi.rs"]
mod ffi;

#[cfg(target_arch = "wasm32")]
use crate::errors::wasm_error;

/// How many calls may stand at once. A recursion that never returns pushes a
/// frame per call, so an unbounded one grows the frame stack until the
/// operating system kills the process; the call past this many frames raises
/// `call_depth_exceeded` instead. The limit sits far above what a working
/// recursion needs, and the frames a run this deep stands on stay within a few
/// hundred megabytes even for a function that saves many registers per level.
pub const CALL_DEPTH_LIMIT: usize = 1_000_000;

pub type ObjectPool = Pool<Vec<Data>>;

/// A candela map: `Data`-keyed, hashed by the raw value bits, and walked in the
/// order its keys were first inserted. Every surface that walks a map (a `for`
/// loop, `keys`, `values`, printing, json) reads it in that one order, so a
/// program that builds a map the same way twice prints it the same way twice.
pub type CandelaMap = IndexMap<Data, Data, BuildHasherDefault<DataHash>>;
pub type MapPool = Pool<CandelaMap>;

/// Whether two values are the same as a map key or a list element: the same
/// value, or two strings with the same text. A string longer than six bytes
/// lives in the string pool, and the same text can sit in more than one slot.
#[inline(always)]
#[must_use]
pub fn same_key(a: Data, b: Data, strings: &StringPool) -> bool {
    a == b || (a.is_large_str() && b.is_large_str() && a.as_str(strings) == b.as_str(strings))
}

/// A key as a map looks it up, matching a stored key by [`same_key`]. It
/// hashes as [`Data::as_map_key`] files the key, so one that matches sits in
/// the bucket this looks in.
pub struct KeyByText<'a> {
    pub key: Data,
    pub strings: &'a StringPool,
}

impl std::hash::Hash for KeyByText<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.as_map_key(self.strings).hash(state);
    }
}

impl indexmap::Equivalent<Data> for KeyByText<'_> {
    fn equivalent(&self, stored: &Data) -> bool {
        same_key(self.key, *stored, self.strings)
    }
}

/// Puts `value` under `key`, replacing the value of a key with the same text
/// if the map holds one, and answers the value it replaced. Every insert goes
/// through here, so every key a map holds is filed by [`Data::as_map_key`].
pub fn map_insert(
    map: &mut CandelaMap,
    key: Data,
    value: Data,
    strings: &StringPool,
) -> Option<Data> {
    if let Some(slot) = map.get_mut(&KeyByText { key, strings }) {
        return Some(std::mem::replace(slot, value));
    }
    map.insert(key.as_map_key(strings), value);
    None
}

/// The character count a slot has not been asked for yet.
const UNCOUNTED: u32 = u32::MAX;
/// The slot id a cursor that points at no string carries.
const NO_STRING: u32 = u32::MAX;

/// The most bytes a freed string slot keeps room for.
const KEEP_FREED_STRING_CAPACITY: usize = 256;

/// Where a walk over a pooled string stopped.
#[derive(Clone, Copy)]
struct StrCursor {
    id: u32,
    char_idx: u32,
    byte_off: u32,
}

/// A cursor standing on no string at all.
const NO_CURSOR: StrCursor = StrCursor {
    id: NO_STRING,
    char_idx: 0,
    byte_off: 0,
};

/// One pooled string and what has been worked out about it. The count sits
/// next to the string's own header so reading both costs one cache line.
struct PooledStr {
    text: String,
    char_len: Cell<u32>,
}

/// Every string too long to sit inside a [`Data`], with the character count of
/// each one remembered.
///
/// A position on a string counts characters, so an index or a slice has to find
/// where character `i` starts. Counting a long string's characters for every
/// one of those would turn a loop over it into quadratic work, so a slot's
/// count is taken once and kept until the slot holds a different string. A
/// string whose character count equals its byte length holds nothing but
/// single-byte characters, and on it a character position is a byte position;
/// that is the route indexing and slicing take. The cursor remembers where the
/// last walk stopped, so stepping through a multi-byte string forwards resumes
/// there instead of restarting at its first byte.
///
/// The pool also carries the collector's marks for its strings. Interning a
/// string hands out a slot it found by its text, and the slot it finds has to
/// count as reached by a cycle in progress, which is what [`Self::shade`] does
/// without the interning caller needing the collector.
pub struct StringPool {
    strings: Vec<PooledStr>,
    cursor: Cell<StrCursor>,
    marks: MarkBits,
    /// The pool's length when the cycle in progress began. A slot at or past
    /// it was allocated since, and counts as marked.
    mark_limit: u32,
}

impl Default for StringPool {
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl StringPool {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            strings: Vec::with_capacity(capacity),
            cursor: Cell::new(NO_CURSOR),
            marks: MarkBits::default(),
            mark_limit: 0,
        }
    }
    #[must_use]
    pub fn from_strings(strings: Vec<String>) -> Self {
        Self {
            strings: strings
                .into_iter()
                .map(|text| PooledStr {
                    text,
                    char_len: Cell::new(UNCOUNTED),
                })
                .collect(),
            cursor: Cell::new(NO_CURSOR),
            marks: MarkBits::default(),
            mark_limit: 0,
        }
    }
    #[inline(always)]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.strings.len()
    }
    #[inline(always)]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
    #[inline(always)]
    pub fn push(&mut self, value: String) {
        self.strings.push(PooledStr {
            text: value,
            char_len: Cell::new(UNCOUNTED),
        });
    }
    #[inline(always)]
    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.strings.iter().map(|s| &s.text)
    }
    /// A slot to write a different string into. What was worked out about the
    /// slot describes the string being replaced, so it goes.
    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> &mut String {
        if self.cursor.get().id as usize == index {
            self.cursor.set(NO_CURSOR);
        }
        let slot = unsafe { self.strings.get_unchecked_mut(index) };
        slot.char_len.set(UNCOUNTED);
        &mut slot.text
    }
    /// Starts a cycle's marks: none, over the slots the pool has now.
    pub(crate) fn begin_marking(&mut self) {
        self.marks.reset(self.len());
        self.mark_limit = self.len() as u32;
    }
    /// Ends a cycle's marks. Every slot counts as marked until the next cycle
    /// begins, so shading one is a no-op.
    pub(crate) const fn end_marking(&mut self) {
        self.mark_limit = 0;
    }
    /// Marks slot `id` as reached by the cycle in progress.
    #[inline(always)]
    pub(crate) fn shade(&mut self, id: usize) {
        if id < self.mark_limit as usize {
            self.marks.insert(id);
        }
    }
    /// The pool's length when the cycle in progress began.
    pub(crate) const fn mark_limit(&self) -> usize {
        self.mark_limit as usize
    }
    /// The cycle's marks.
    pub(crate) const fn marks(&self) -> &MarkBits {
        &self.marks
    }
    /// Empties a slot the collector freed. A long string's buffer goes back
    /// to the allocator rather than waiting in a dead slot for the next
    /// string written there.
    pub fn release(&mut self, index: usize) {
        let text = self.get_mut(index);
        if text.capacity() > KEEP_FREED_STRING_CAPACITY {
            *text = String::new();
        } else {
            text.clear();
        }
    }
    /// The character count of the string in `id`, walked once and then
    /// remembered on the slot.
    #[inline(always)]
    #[must_use]
    pub fn char_len(&self, id: usize) -> usize {
        let slot = unsafe { self.strings.get_unchecked(id) };
        let remembered = slot.char_len.get();
        if remembered == UNCOUNTED {
            count_and_remember(slot)
        } else {
            remembered as usize
        }
    }
    /// Whether every character in `id` takes a single byte, answered from the
    /// remembered count rather than a second walk.
    #[inline(always)]
    #[must_use]
    pub fn is_ascii(&self, id: usize) -> bool {
        self.char_len(id) == self[id].len()
    }
    /// The byte offset character `char_idx` starts at. The string has at least
    /// `char_idx` characters; at exactly that many the answer is its byte
    /// length, which is what a slice bound on the end of the string asks for.
    #[must_use]
    pub fn byte_offset(&self, id: usize, char_idx: usize) -> usize {
        let bytes = self[id].as_bytes();
        let cursor = self.cursor.get();
        let (mut at_char, mut at_byte) =
            if cursor.id as usize == id && cursor.char_idx as usize <= char_idx {
                (cursor.char_idx as usize, cursor.byte_off as usize)
            } else {
                (0, 0)
            };
        while at_char < char_idx {
            at_byte += utf8_width(bytes[at_byte]);
            at_char += 1;
        }
        self.cursor.set(StrCursor {
            id: id as u32,
            char_idx: at_char as u32,
            byte_off: at_byte as u32,
        });
        at_byte
    }
}

impl Index<usize> for StringPool {
    type Output = String;
    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        unsafe { &self.strings.get_unchecked(index).text }
    }
}

/// How many bytes the character starting with `first` takes. A byte below
/// `0x80` stands for itself; every other character says how many bytes it
/// spans in the leading one bits of its first byte.
#[inline(always)]
pub(crate) const fn utf8_width(first: u8) -> usize {
    let leading = first.leading_ones() as usize;
    if leading == 0 { 1 } else { leading }
}

/// Counts a slot's characters the first time anything asks, which happens once
/// per string. Out of line so the instructions that read the count stay small.
#[cold]
#[inline(never)]
fn count_and_remember(slot: &PooledStr) -> usize {
    let counted = count_chars(&slot.text);
    if counted < UNCOUNTED as usize {
        slot.char_len.set(counted as u32);
    }
    counted
}

/// The number of characters in `s`. A continuation byte matches `0b10xx_xxxx`,
/// which as a signed byte is below `-0x40`; every other byte starts a
/// character.
#[must_use]
pub(crate) fn count_chars(s: &str) -> usize {
    s.as_bytes().iter().filter(|&&b| (b as i8) >= -0x40).count()
}

/// The byte offset character `char_idx` starts at in `s`, for a `char_idx` no
/// greater than the number of characters in `s`.
#[inline(always)]
#[must_use]
pub(crate) fn char_byte_offset(s: &str, char_idx: usize) -> usize {
    let bytes = s.as_bytes();
    let mut at_byte = 0;
    for _ in 0..char_idx {
        at_byte += utf8_width(bytes[at_byte]);
    }
    at_byte
}

/// Compares two values with the string-comparison instructions.
///
/// The compiler emits these whenever either operand is statically a `string`,
/// so the other operand can still turn out to be something else at run time:
/// a parameter left un-annotated, or a value the checker only knows as `any`.
/// Reading a non-string as a string is not meaningful, so a mismatched pair is
/// unequal. Two values that are neither strings fall back to identity, which is
/// how the plain equality instruction compares them.
#[inline(always)]
fn str_eq(x: Data, y: Data, string_pool: &StringPool) -> bool {
    if x.is_string() && y.is_string() {
        x.as_str(string_pool) == y.as_str(string_pool)
    } else {
        x == y
    }
}

/// Whether `count` is a number of bits an `int` can be shifted by.
///
/// `int` holds 64 bits, so a wider shift, or a negative one, names no result.
/// The compiler asks the same question of a count written as a literal, so that
/// one is reported before the program runs.
#[inline(always)]
#[must_use]
pub const fn shift_count_in_range(count: i64) -> bool {
    count >= 0 && count < i64::BITS as i64
}

/// Whether `v` has the type [`type_code`] `code` names.
fn has_type_code(v: Data, code: i64) -> bool {
    let id = (code >> type_code::KIND_BITS) as u16;
    match code & ((1 << type_code::KIND_BITS) - 1) {
        type_code::INT => v.is_int(),
        type_code::FLOAT => v.is_float(),
        type_code::STRING => v.is_string(),
        type_code::BOOL => v.is_bool(),
        type_code::NULL => v.is_null(),
        type_code::LIST => v.is_array() && !v.is_function(),
        type_code::MAP => v.is_map(),
        type_code::FUNCTION => v.is_function(),
        type_code::STRUCT => v.is_struct() && v.struct_type_id() == id,
        type_code::ENUM => v.is_enum() && v.enum_type_id() == id,
        _ => false,
    }
}

/// The types a list of [`type_code`]s names, as a report writes them.
#[cold]
fn type_codes_text(codes: &[Data], structs: &[Struct], enums: &[EnumType]) -> String {
    let names: Vec<&str> = codes
        .iter()
        .map(|code| {
            let code = code.as_int();
            let id = (code >> type_code::KIND_BITS) as usize;
            match code & ((1 << type_code::KIND_BITS) - 1) {
                type_code::INT => "int",
                type_code::FLOAT => "float",
                type_code::STRING => "string",
                type_code::BOOL => "bool",
                type_code::NULL => "null",
                type_code::LIST => "list",
                type_code::MAP => "map",
                type_code::FUNCTION => "function",
                type_code::STRUCT => structs.get(id).map_or("struct", |s| s.name.as_str()),
                type_code::ENUM => enums.get(id).map_or("enum", |e| e.name.as_str()),
                _ => "value",
            }
        })
        .collect();
    names.join("|")
}

fn obj_eq(
    x: Data,
    y: Data,
    obj_pool: &ObjectPool,
    map_pool: &MapPool,
    string_pool: &StringPool,
) -> bool {
    if x == y {
        return true;
    }
    if x.tag() != y.tag() {
        return false;
    }
    if x.is_string() && y.is_string() {
        return x.as_str(string_pool) == y.as_str(string_pool);
    }
    // Enum values compare like structs: `x.tag() == y.tag()` above already
    // proved same enum type id, and element 0 of the pool entry is the variant
    // tag, so an element-wise compare covers variant and payload together.
    if (x.is_array() || x.is_struct() || x.is_enum())
        && (y.is_array() || y.is_struct() || y.is_enum())
    {
        let x_obj = &obj_pool[x.as_array()];
        let y_obj = &obj_pool[y.as_array()];
        if x_obj.len() != y_obj.len() {
            return false;
        }
        if x_obj == y_obj {
            return true;
        }
        for (x, y) in x_obj.iter().zip(y_obj) {
            if !obj_eq(*x, *y, obj_pool, map_pool, string_pool) {
                return false;
            }
        }
        return true;
    }
    if x.is_map() && y.is_map() {
        let x_map = &map_pool[x.as_map()];
        let y_map = &map_pool[y.as_map()];
        if x_map.len() != y_map.len() {
            return false;
        }
        if x_map == y_map {
            return true;
        }
        for (k, v) in x_map {
            if !y_map
                .get(&KeyByText {
                    key: *k,
                    strings: string_pool,
                })
                .is_some_and(|y_v| obj_eq(*v, *y_v, obj_pool, map_pool, string_pool))
            {
                return false;
            }
        }
        return true;
    }
    false
}

struct CallFrame {
    return_addr: u16,
    return_reg: u16,
    /// The callsite whose live registers this frame pushed onto the recursion
    /// stack, or `None` when the call saved nothing. `SaveFrame` fills it in;
    /// every way out of that call reads it back, so the registers the call
    /// overwrote are restored once, whether the callee returns a value or not.
    saved_callsite: Option<u16>,
}

/// Winds the recursion stack back to where `handle`'s `try` began.
///
/// The call the `try` wrapped is the first frame the catch drops, and on its
/// way in it saved the registers the resuming function still reads. That
/// function carries on where the call left it, so it gets those registers
/// back, the same values a return of the call would have handed over. The
/// calls made below the abort never return, and the levels they saved go with
/// them. A `try` that throws without calling anything has nothing above its
/// own depth and leaves the stack alone.
#[cold]
#[inline(never)]
fn unwind_to_catch(
    handle: &ErrorCatch,
    call_frames: &[CallFrame],
    callsite_registers: &[Vec<u16>],
    recursion_stack: &mut RegisterFile,
    regs: Regs,
) {
    let Some(aborted) = call_frames.get(handle.call_frames_len as usize) else {
        return;
    };
    let base = handle.recursion_len as usize;
    // The aborted call saved nothing when it was an ordinary call, so the
    // resuming function has its registers already and only the deeper levels
    // go.
    let saved_regs: &[u16] = match aborted.saved_callsite {
        Some(callsite_id) => unsafe { callsite_registers.get_unchecked(callsite_id as usize) },
        None => &[],
    };
    restore_registers(base, saved_regs, recursion_stack, regs);
}

/// Makes room on the call stack for one more frame, or raises
/// `call_depth_exceeded` when the run already stands [`CALL_DEPTH_LIMIT`]
/// calls deep. Answers `None` when the call can go ahead, or the instruction a
/// catch resumes at.
///
/// The stack never grows past the limit, so a full stack at the limit is the
/// only way a call is refused, and the calls themselves test one thing: whether
/// the stack is full.
#[cold]
#[inline(never)]
fn frame_room(m: &mut Machine<'_>, instructions: &[Instr], i: usize, regs: Regs) -> Option<usize> {
    let depth = m.call_frames.len();
    if depth >= CALL_DEPTH_LIMIT {
        let instr = unsafe { *instructions.get_unchecked(i) };
        return Some(catch_error(
            ErrType::CallDepthExceeded(call_site_name(m.err_ctx, instr), CALL_DEPTH_LIMIT),
            instr,
            &mut m.failure,
            &mut m.error_handles,
            &mut m.call_frames,
            &mut m.args,
            m.callsite_registers,
            &mut m.recursion_stack,
            regs,
            m.r,
            m.obj_pool,
            m.map_pool,
            m.str_pool,
            m.gc,
        ));
    }
    m.call_frames
        .reserve_exact(depth.max(64).min(CALL_DEPTH_LIMIT - depth));
    None
}

/// Makes room on the recursion stack for `count` more saved registers.
#[cold]
#[inline(never)]
fn grow_recursion_stack(stack: &mut Vec<Data>, count: usize) {
    stack.reserve(count);
}

/// Hands a run-time error to the innermost `try` that is still open. When none
/// is, it records the error in `failure` and answers [`FAILED`], which ends the
/// run.
///
/// A caught error unwinds the calls the `try` wrapped, puts the error's name in
/// the catch's register, and answers the instruction the catch starts at. The
/// message is built while the throwing frame is still standing: a thrown value
/// the unwind is about to overwrite is still a garbage-collection root here.
///
/// Every instruction that can fail calls this out of line, so the dispatch
/// loop carries none of it.
#[cold]
#[inline(never)]
fn catch_error(
    err: ErrType,
    instr: Instr,
    failure: &mut Option<RuntimeError>,
    error_handles: &mut Vec<ErrorCatch>,
    call_frames: &mut Vec<CallFrame>,
    args: &mut Vec<u16>,
    callsite_registers: &[Vec<u16>],
    recursion_stack: &mut RegisterFile,
    mut regs: Regs,
    r: &RegisterFile,
    obj_pool: &mut ObjectPool,
    map_pool: &mut MapPool,
    str_pool: &mut StringPool,
    gc: &mut GcState,
) -> usize {
    let Some(err_handle) = error_handles.pop() else {
        *failure = Some(RuntimeError::new(instr, err));
        return FAILED;
    };
    let caught = Data::string(
        err.kind(),
        obj_pool,
        map_pool,
        str_pool,
        r,
        recursion_stack,
        gc,
    );
    unwind_to_catch(
        &err_handle,
        call_frames,
        callsite_registers,
        recursion_stack,
        regs,
    );
    unsafe {
        args.set_len(err_handle.args_len as usize);
        call_frames.set_len(err_handle.call_frames_len as usize);
    }
    regs[err_handle.error_reg] = caught;
    err_handle.catch_loc as usize
}

/// Puts back the registers `SaveFrame` pushed for `callsite` on the way into a
/// call that is now returning, and drops them from the recursion stack. The top
/// of the stack belongs to that call, so the saved values sit in the last
/// `callsite_registers[callsite].len()` slots.
#[inline(always)]
fn restore_saved_registers(
    callsite: u16,
    callsite_registers: &[Vec<u16>],
    recursion_stack: &mut RegisterFile,
    regs: Regs,
) {
    let saved = unsafe { callsite_registers.get_unchecked(callsite as usize) };
    restore_registers(
        recursion_stack.len() - saved.len(),
        saved,
        recursion_stack,
        regs,
    );
}

/// Copies the `regs` a call saved at `base` back into the register file, and
/// drops everything from `base` up off the recursion stack.
#[inline(always)]
fn restore_registers(
    base: usize,
    saved_regs: &[u16],
    recursion_stack: &mut RegisterFile,
    mut regs: Regs,
) {
    let saved = unsafe { recursion_stack.0.as_ptr().add(base) };
    // Most calls save one register, and a loop costs more to set up than
    // that one copy.
    if let [reg] = saved_regs {
        regs[*reg] = unsafe { Data::load(saved) };
    } else {
        for (k, reg) in saved_regs.iter().enumerate() {
            regs[*reg] = unsafe { Data::load(saved.add(k)) };
        }
    }
    unsafe {
        recursion_stack.0.set_len(base);
    }
}

pub trait UncheckedVecOps<T> {
    fn pop_unchecked(&mut self) -> T;
    /// Pushes onto a vector that has room for one more.
    fn push_unchecked(&mut self, value: T);
}
pub trait UncheckedSliceOps<T: Copy> {
    /// # Safety
    ///
    /// `src` must not be longer than `self`; the copy is unchecked in release
    /// builds.
    unsafe fn copy_from_slice_unchecked(&mut self, src: &[T]);
}

impl<T> UncheckedVecOps<T> for Vec<T> {
    #[inline(always)]
    fn pop_unchecked(&mut self) -> T {
        debug_assert!(!self.is_empty());
        let new_len = self.len() - 1;
        unsafe {
            self.set_len(new_len);
            self.as_mut_ptr().add(new_len).read()
        }
    }
    #[inline(always)]
    fn push_unchecked(&mut self, value: T) {
        debug_assert!(self.len() < self.capacity());
        unsafe {
            self.as_mut_ptr().add(self.len()).write(value);
            self.set_len(self.len() + 1);
        }
    }
}

impl<T: Copy> UncheckedSliceOps<T> for [T] {
    #[inline(always)]
    unsafe fn copy_from_slice_unchecked(&mut self, src: &[T]) {
        debug_assert_eq!(self.len(), src.len());
        unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), self.as_mut_ptr(), self.len()) };
    }
}

#[repr(transparent)]
pub struct Pool<T>(pub Vec<T>);

impl<T> Pool<T> {
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.0.len()
    }
    #[inline(always)]
    pub fn push(&mut self, value: T) {
        self.0.push(value);
    }
    #[inline(always)]
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }
    #[inline(always)]
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }
    #[inline(always)]
    pub fn get_mut(&mut self, index: usize) -> &mut T {
        unsafe { self.0.get_unchecked_mut(index) }
    }
    #[inline(always)]
    pub const fn as_mut_ptr(&mut self) -> *mut T {
        self.0.as_mut_ptr()
    }
    #[inline(always)]
    pub fn split_at_mut(&mut self, mid: usize) -> (&mut [T], &mut [T]) {
        unsafe { self.0.split_at_mut_unchecked(mid) }
    }
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> Index<usize> for Pool<T> {
    type Output = T;
    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        unsafe { self.0.get_unchecked(index) }
    }
}
impl<T> IndexMut<usize> for Pool<T> {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut T {
        unsafe { self.0.get_unchecked_mut(index) }
    }
}

#[repr(transparent)]
pub struct RegisterFile(pub Vec<Data>);

impl RegisterFile {
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.0.len()
    }
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Index<u16> for RegisterFile {
    type Output = Data;
    #[inline(always)]
    fn index(&self, index: u16) -> &Self::Output {
        unsafe { self.0.get_unchecked(index as usize) }
    }
}
impl IndexMut<u16> for RegisterFile {
    #[inline(always)]
    fn index_mut(&mut self, index: u16) -> &mut Data {
        unsafe { self.0.get_unchecked_mut(index as usize) }
    }
}
impl Index<usize> for RegisterFile {
    type Output = Data;
    #[inline(always)]
    fn index(&self, index: usize) -> &Self::Output {
        unsafe { self.0.get_unchecked(index) }
    }
}
impl IndexMut<usize> for RegisterFile {
    #[inline(always)]
    fn index_mut(&mut self, index: usize) -> &mut Data {
        unsafe { self.0.get_unchecked_mut(index) }
    }
}

/// The registers of one run, addressed through the register file's base
/// pointer. [`execute`] makes one at the start of a run and reads and writes
/// every register through it; the file is never resized while a program runs,
/// so the pointer stays valid to the end.
#[derive(Clone, Copy)]
pub struct Regs(*mut Data);

impl Regs {
    #[inline(always)]
    fn new(file: &mut RegisterFile) -> Self {
        Self(file.0.as_mut_ptr())
    }
}

impl Regs {
    /// The value in register `index`, read a word at a time (see
    /// [`Data::load`]). Every instruction that copies a whole value out of a
    /// register reads it this way.
    #[inline(always)]
    fn get(self, index: u16) -> Data {
        unsafe { Data::load(self.0.add(index as usize)) }
    }
}

impl Index<u16> for Regs {
    type Output = Data;
    #[inline(always)]
    fn index(&self, index: u16) -> &Data {
        unsafe { &*self.0.add(index as usize) }
    }
}
impl IndexMut<u16> for Regs {
    #[inline(always)]
    fn index_mut(&mut self, index: u16) -> &mut Data {
        unsafe { &mut *self.0.add(index as usize) }
    }
}

/// What [`run_cold`] answers when the program has stopped.
const HALTED: usize = usize::MAX;
/// What [`catch_error`] and [`run_cold`] answer when an error no `try` catches
/// has stopped the program. It sits just below [`HALTED`], so one comparison
/// tells either from an instruction index.
const FAILED: usize = usize::MAX - 1;

/// Everything one run of [`execute`] works with besides the instruction index
/// and the registers.
///
/// The dispatch loop runs the instructions a program spends its time in, and
/// hands the rest to [`run_cold`] out of line. Both reach the pools, the call
/// stack and the rest through this one value, which leaves the loop little to
/// keep in machine registers besides the instruction pointer and the base of
/// the register file.
struct Machine<'a> {
    /// The register file, read by the collector to find what is still live.
    /// Instructions reach the registers through [`Regs`].
    r: &'a RegisterFile,
    obj_pool: &'a mut ObjectPool,
    map_pool: &'a mut MapPool,
    str_pool: &'a mut StringPool,
    gc: &'a mut GcState,
    err_ctx: &'a ErrorCtx<'a>,
    callsite_registers: &'a [Vec<u16>],
    dyn_libs: &'a [DynamicLibFn],
    structs: &'a [Struct],
    enums: &'a [EnumType],
    host_sigs: &'a [HostFnSig],
    host_dispatch: &'a [HostDispatch],
    args: Vec<u16>,
    call_frames: Vec<CallFrame>,
    recursion_stack: RegisterFile,
    handle: crate::captured_output::OutputHandle,
    dyn_lib_args: Vec<u64>,
    host_call_args: Vec<Value>,
    keep_alive: Vec<Box<[u8]>>,
    error_handles: Vec<ErrorCatch>,
    /// The error that stopped the run, once one no `try` caught has.
    failure: Option<RuntimeError>,
}

/// Runs `instructions` from `start` until the program halts.
///
/// # Errors
///
/// Returns the [`RuntimeError`] that stopped the program when one no `try`
/// caught was raised. The registers and pools are left as the error found
/// them.
#[allow(unused_unsafe)]
pub fn execute(
    instructions: &[Instr],
    r: &mut RegisterFile,
    Pools {
        objs: obj_pool,
        maps: map_pool,
        strings: str_pool,
        gc,
    }: &mut Pools,
    err_ctx: &ErrorCtx<'_>,
    callsite_registers: &[Vec<u16>],
    dyn_libs: &[DynamicLibFn],
    structs: &[Struct],
    enums: &[EnumType],
    allocated_arg_count: usize,
    allocated_call_depth: usize,
    // Embedding: `host` function signatures and the Rust closures they dispatch
    // to, both indexed by host-function id. Empty for the CLI/REPL/WASM paths.
    // Marshalling is driven by the runtime `Data` tag, so the signatures are
    // read only to name a function in a report (kept here, too, for future
    // per-argument coercion).
    host_sigs: &[HostFnSig],
    host_dispatch: &[HostDispatch],
    // Instruction index to begin execution at. `0` runs `main`; the embedding
    // `Program::call` passes the entry index of an appended call trampoline.
    start: usize,
) -> Result<(), RuntimeError> {
    // The instruction running, as a pointer: the loop steps and jumps it
    // directly, and turns it back into an index only where one is recorded.
    let base = instructions.as_ptr();
    let mut ip = unsafe { base.add(start) };
    // Every register access goes through the file's base pointer, which stays
    // put for the whole run: nothing resizes the register file while a program
    // runs. Reading it through `r` instead would reload the pointer from the
    // file on every access.
    let mut regs = Regs::new(r);
    let recursion_capacity = allocated_call_depth * r.len();
    let mut m = Machine {
        r,
        obj_pool,
        map_pool,
        str_pool,
        gc,
        err_ctx,
        callsite_registers,
        dyn_libs,
        structs,
        enums,
        host_sigs,
        host_dispatch,
        args: Vec::with_capacity(allocated_arg_count),
        call_frames: Vec::with_capacity(allocated_call_depth.clamp(1, CALL_DEPTH_LIMIT)),
        recursion_stack: RegisterFile(Vec::with_capacity(recursion_capacity)),
        handle: crate::captured_output::stdout(),
        dyn_lib_args: Vec::new(),
        host_call_args: Vec::new(),
        keep_alive: Vec::new(),
        error_handles: Vec::new(),
        failure: None,
    };

    macro_rules! error_with_catch {
        ($err:expr) => {
            let resume = catch_error(
                $err,
                unsafe { *ip },
                &mut m.failure,
                &mut m.error_handles,
                &mut m.call_frames,
                &mut m.args,
                m.callsite_registers,
                &mut m.recursion_stack,
                regs,
                m.r,
                m.obj_pool,
                m.map_pool,
                m.str_pool,
                m.gc,
            );
            if resume == FAILED {
                break;
            }
            ip = unsafe { base.add(resume) };
            continue;
        };
    }

    loop {
        match unsafe { *ip } {
            Instr::Jmp(size) => {
                ip = unsafe { ip.add(size as usize) };
                continue;
            }
            Instr::JmpBack(size) => {
                ip = unsafe { ip.sub(size as usize) };
                continue;
            }
            Instr::Mov(tgt, dest) => regs[dest] = regs.get(tgt),
            Instr::MovChain(dest, mid, src) => {
                regs[dest] = regs.get(mid);
                regs[mid] = regs.get(src);
            }
            Instr::SetInt(dest, n) => regs[dest] = Data::int(i64::from(n)),
            Instr::SetBool(b, dest) => regs[dest] = b.into(),
            Instr::CallFunc(new_loc, return_id) => {
                if m.call_frames.len() == m.call_frames.capacity()
                    && let Some(catch) = frame_room(&mut m, instructions, index_of(base, ip), regs)
                {
                    if catch == FAILED {
                        break;
                    }
                    ip = unsafe { base.add(catch) };
                    continue;
                }
                m.call_frames.push_unchecked(CallFrame {
                    return_addr: index_of(base, ip) as u16,
                    return_reg: return_id,
                    saved_callsite: None,
                });
                ip = unsafe { base.add(new_loc as usize) };
                continue;
            }
            Instr::CallFuncRecursive(new_loc, _) => {
                ip = unsafe { base.add(new_loc as usize) };
                continue;
            }
            Instr::CallIndirect(callee) => {
                // The first slot of a function value is where its body starts.
                // The `SaveFrame` just above pushed the frame, so the jump is
                // all that is left to do.
                let value_id = regs[callee].as_array();
                ip = unsafe { base.add(m.obj_pool[value_id].get_unchecked(0).as_int() as usize) };
                continue;
            }
            Instr::VoidReturn => {
                // Nothing to return, but a recursive call still has to hand the
                // caller back the registers the call overwrote before jumping.
                let call_frame = m.call_frames.pop_unchecked();
                if let Some(callsite_id) = call_frame.saved_callsite {
                    restore_saved_registers(
                        callsite_id,
                        m.callsite_registers,
                        &mut m.recursion_stack,
                        regs,
                    );
                }
                ip = unsafe { base.add(call_frame.return_addr as usize) };
            }
            Instr::SaveFrame(relative_func_loc, return_register, callsite_id) => {
                if m.call_frames.len() == m.call_frames.capacity()
                    && let Some(catch) = frame_room(&mut m, instructions, index_of(base, ip), regs)
                {
                    if catch == FAILED {
                        break;
                    }
                    ip = unsafe { base.add(catch) };
                    continue;
                }
                m.call_frames.push_unchecked(CallFrame {
                    return_addr: (index_of(base, ip) as u16) + relative_func_loc,
                    return_reg: return_register,
                    saved_callsite: Some(callsite_id),
                });
                let saved = unsafe { m.callsite_registers.get_unchecked(callsite_id as usize) };
                let stack = &mut m.recursion_stack.0;
                if stack.capacity() - stack.len() < saved.len() {
                    grow_recursion_stack(stack, saved.len());
                }
                unsafe {
                    let top = stack.as_mut_ptr().add(stack.len());
                    // Most calls save one register, and a loop costs more to
                    // set up than that one copy.
                    if let [reg] = saved[..] {
                        top.write(regs.get(reg));
                    } else {
                        for (k, &reg) in saved.iter().enumerate() {
                            top.add(k).write(regs.get(reg));
                        }
                    }
                    stack.set_len(stack.len() + saved.len());
                }
            }
            Instr::Return(tgt) => {
                // Pop the latest call frame, set the return value and jump back to the callsite
                let call_frame = m.call_frames.pop_unchecked();
                ip = unsafe { base.add(call_frame.return_addr as usize) };
                regs[call_frame.return_reg] = regs.get(tgt);
            }
            Instr::RecursiveReturn(tgt) => {
                let call_frame = m.call_frames.pop_unchecked();
                let temp = regs.get(tgt);
                if let Some(callsite_id) = call_frame.saved_callsite {
                    restore_saved_registers(
                        callsite_id,
                        m.callsite_registers,
                        &mut m.recursion_stack,
                        regs,
                    );
                }
                ip = unsafe { base.add(call_frame.return_addr as usize) };
                regs[call_frame.return_reg] = temp;
            }
            Instr::IsFalseJmp(cond_id, size) => {
                if regs[cond_id].is_false() {
                    ip = unsafe { ip.add(size as usize) };
                    continue;
                }
            }
            Instr::IsTrueJmp(cond_id, size) => {
                if regs[cond_id].is_true() {
                    ip = unsafe { ip.add(size as usize) };
                    continue;
                }
            }
            Instr::AddFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() + regs[o2].as_float()).into();
            }
            Instr::AddInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() + regs[o2].as_int()).into();
            }
            Instr::MulFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() * regs[o2].as_float()).into();
            }
            Instr::MulInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() * regs[o2].as_int()).into();
            }
            Instr::DivFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() / regs[o2].as_float()).into();
            }
            Instr::SubFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() - regs[o2].as_float()).into();
            }
            Instr::SubInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() - regs[o2].as_int()).into();
            }
            Instr::IncInt(reg) => regs[reg].inc_int(),
            Instr::DecInt(reg) => regs[reg].dec_int(),
            Instr::IncIntTo(src, dst) => {
                let s = regs[src];
                regs[dst].inc_into(s);
            }
            Instr::DecIntTo(src, dst) => {
                let s = regs[src];
                regs[dst].dec_into(s);
            }
            Instr::Eq(o1, o2, dest) => {
                regs[dest] = (regs.get(o1) == regs.get(o2)).into();
            }
            Instr::NotEqJmp(o1, o2, jump_size) => {
                if regs.get(o1) != regs.get(o2) {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::NotEq(o1, o2, dest) => {
                regs[dest] = (regs.get(o1) != regs.get(o2)).into();
            }
            Instr::EqJmp(o1, o2, jump_size) => {
                if regs.get(o1) == regs.get(o2) {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::SupFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() > regs[o2].as_float()).into();
            }
            Instr::SupInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() > regs[o2].as_int()).into();
            }
            Instr::InfEqFloatJmp(o1, o2, jump_size) => {
                if regs[o1].as_float() <= regs[o2].as_float() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::InfEqIntJmp(o1, o2, jump_size) => {
                if regs[o1].as_int() <= regs[o2].as_int() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::SupEqFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() >= regs[o2].as_float()).into();
            }
            Instr::SupEqInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() >= regs[o2].as_int()).into();
            }
            Instr::InfFloatJmp(o1, o2, jump_size) => {
                if regs[o1].as_float() < regs[o2].as_float() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::InfIntJmp(o1, o2, jump_size) => {
                if regs[o1].as_int() < regs[o2].as_int() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::StepIntJmpBack(counter, end, jump_size) => {
                regs[counter].inc_int();
                if regs[counter].as_int() < regs[end].as_int() {
                    ip = unsafe { ip.sub(jump_size as usize) };
                    continue;
                }
            }
            Instr::AddIntImm(src, imm, dest) => {
                regs[dest] = (regs[src].as_int() + i64::from(imm)).into();
            }
            Instr::InfIntJmpBack(o1, o2, jump_size) => {
                if regs[o1].as_int() < regs[o2].as_int() {
                    ip = unsafe { ip.sub(jump_size as usize) };
                    continue;
                }
            }
            Instr::InfFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() < regs[o2].as_float()).into();
            }
            Instr::InfInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() < regs[o2].as_int()).into();
            }
            Instr::SupEqFloatJmp(o1, o2, jump_size) => {
                if regs[o1].as_float() >= regs[o2].as_float() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::SupEqIntJmp(o1, o2, jump_size) => {
                if regs[o1].as_int() >= regs[o2].as_int() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::InfEqFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() <= regs[o2].as_float()).into();
            }
            Instr::InfEqInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() <= regs[o2].as_int()).into();
            }
            Instr::SupFloatJmp(o1, o2, jump_size) => {
                if regs[o1].as_float() > regs[o2].as_float() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::SupIntJmp(o1, o2, jump_size) => {
                if regs[o1].as_int() > regs[o2].as_int() {
                    ip = unsafe { ip.add(jump_size as usize) };
                    continue;
                }
            }
            Instr::BoolAnd(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_bool() && regs[o2].as_bool()).into();
            }
            Instr::BoolOr(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_bool() || regs[o2].as_bool()).into();
            }
            Instr::NegBool(src, dest) => {
                regs[dest] = (!regs[src].as_bool()).into();
            }
            Instr::NegFloat(tgt, dest) => {
                regs[dest] = (-regs[tgt].as_float()).into();
            }
            Instr::NegInt(tgt, dest) => {
                regs[dest] = (-regs[tgt].as_int()).into();
            }
            Instr::StoreFuncArg(id) => m.args.push(id),
            // Each store into the heap marks the value it overwrites while the
            // collector is marking; see the barrier rules in `gc`.
            Instr::ObjElemMov(new_elem_reg_id, array_id, idx) => {
                let arr = m.obj_pool.get_mut(array_id as usize);
                let slot = unsafe { arr.get_unchecked_mut(idx as usize) };
                if m.gc.marking() {
                    shade_overwritten(m.gc, m.str_pool, *slot);
                }
                *slot = regs.get(new_elem_reg_id);
            }
            Instr::SetElementObj(array_reg_id, new_elem_reg_id, idx) => {
                let array = m.obj_pool.get_mut(regs[array_reg_id].as_array());
                let index = regs[idx].as_int();
                if index < 0 || (index as u64) >= array.len() as u64 {
                    error_with_catch!(ErrType::IndexOutOfBounds(array.len(), index));
                }
                let slot = unsafe { array.get_unchecked_mut(index as usize) };
                if m.gc.marking() {
                    shade_overwritten(m.gc, m.str_pool, *slot);
                }
                *slot = regs.get(new_elem_reg_id);
            }
            Instr::SetFieldStruct(struct_reg_id, new_elem_reg_id, idx) => {
                let s = m.obj_pool.get_mut(regs[struct_reg_id].as_struct());
                let slot = unsafe { s.get_unchecked_mut(idx as usize) };
                if m.gc.marking() {
                    shade_overwritten(m.gc, m.str_pool, *slot);
                }
                *slot = regs.get(new_elem_reg_id);
            }
            // The field holds a float before and after, so the store needs no
            // barrier: the collector never follows a float.
            Instr::AddFieldFloat(struct_reg_id, value_reg_id, idx) => {
                let value = regs[value_reg_id].as_float();
                let s = m.obj_pool.get_mut(regs[struct_reg_id].as_struct());
                let slot = unsafe { s.get_unchecked_mut(idx as usize) };
                *slot = (slot.as_float() + value).into();
            }
            Instr::GetIndexArray(array_reg_id, index, dest) => {
                let idx = regs[index].as_int();
                let arr_id = regs[array_reg_id].as_array();
                let array = &m.obj_pool[arr_id];
                if idx < 0 || (idx as u64) >= array.len() as u64 {
                    error_with_catch!(ErrType::IndexOutOfBounds(array.len(), idx));
                }
                regs[dest] = unsafe { Data::load(array.as_ptr().add(idx as usize)) };
            }
            Instr::GetFieldStruct(struct_reg_id, index, dest) => {
                let struct_id = regs[struct_reg_id].as_struct();
                let s = &m.obj_pool[struct_id];
                regs[dest] = unsafe { Data::load(s.as_ptr().add(index as usize)) }
            }
            Instr::LoadCell(cell, dest) => {
                let slot = &m.obj_pool[regs[cell].as_array()];
                regs[dest] = unsafe { Data::load(slot.as_ptr()) };
            }
            Instr::StoreCell(cell, src) => {
                let value = regs.get(src);
                let cell = m.obj_pool.get_mut(regs[cell].as_array());
                let slot = unsafe { cell.get_unchecked_mut(0) };
                if m.gc.marking() {
                    shade_overwritten(m.gc, m.str_pool, *slot);
                }
                *slot = value;
            }
            Instr::Push(array, element) => {
                m.obj_pool
                    .get_mut(regs[array].as_array())
                    .push(regs.get(element));
            }
            Instr::CloneStruct(src_reg, dest_reg) => {
                let new_id = alloc_array(
                    m.obj_pool,
                    m.map_pool,
                    m.str_pool,
                    m.r,
                    &m.recursion_stack,
                    m.gc,
                ) as usize;
                let src_reg = regs[src_reg];
                let src = &m.obj_pool[src_reg.as_struct()];
                let len = src.len();
                unsafe {
                    let src_ptr = src.as_ptr();
                    let dst = m.obj_pool.get_mut(new_id);
                    dst.reserve_exact(len);
                    dst.set_len(len);
                    std::ptr::copy_nonoverlapping(src_ptr, dst.as_mut_ptr(), len);
                }
                regs[dest_reg] = Data::struct_instance(src_reg.struct_type_id(), new_id as u32);
            }
            // Named one by one rather than caught with `_`: a match that
            // names every instruction compiles to a jump table that needs no
            // range check before the jump.
            Instr::Print(..)
            | Instr::ObjNotEqJmp(..)
            | Instr::ObjEqJmp(..)
            | Instr::StrNotEqJmp(..)
            | Instr::StrEqJmp(..)
            | Instr::AddArray(..)
            | Instr::AddStr(..)
            | Instr::DivInt(..)
            | Instr::ModFloat(..)
            | Instr::ModInt(..)
            | Instr::PowFloat(..)
            | Instr::PowInt(..)
            | Instr::BitAndInt(..)
            | Instr::BitOrInt(..)
            | Instr::BitXorInt(..)
            | Instr::BitNotInt(..)
            | Instr::ShlInt(..)
            | Instr::ShrInt(..)
            | Instr::ObjEq(..)
            | Instr::ObjNotEq(..)
            | Instr::StrEq(..)
            | Instr::StrNotEq(..)
            | Instr::SupStr(..)
            | Instr::SupEqStr(..)
            | Instr::InfStr(..)
            | Instr::InfEqStr(..)
            | Instr::CallDynamicLibFunc(..)
            | Instr::CallHostFunc(..)
            | Instr::CallLibFunc(..)
            | Instr::CallLibFuncVoid(..)
            | Instr::StartErrorCatch(..)
            | Instr::StopErrorCatch
            | Instr::ThrowError(..)
            | Instr::EmptyArray(..)
            | Instr::EmptyFnValue(..)
            | Instr::CloneArray(..)
            | Instr::CloneEnum(..)
            | Instr::SetElementString(..)
            | Instr::GetIndexString(..)
            | Instr::GetSliceArray(..)
            | Instr::GetSliceString(..)
            | Instr::Remove(..)
            | Instr::MapGet(..)
            | Instr::MapInsert(..)
            | Instr::MapInsertReg(..)
            | Instr::MapRemove(..)
            | Instr::CloneMap(..)
            | Instr::NewCell(..)
            | Instr::Halt(..) => {
                let next = run_cold(&mut m, instructions, index_of(base, ip), regs);
                if next >= FAILED {
                    break;
                }
                ip = unsafe { base.add(next) };
                continue;
            }
        }
        ip = unsafe { ip.add(1) };
    }
    m.failure.map_or(Ok(()), Err)
}

/// The index of the instruction `ip` points at.
#[inline(always)]
fn index_of(base: *const Instr, ip: *const Instr) -> usize {
    unsafe { ip.offset_from_unsigned(base) }
}

/// Runs one instruction the dispatch loop in [`execute`] does not run itself,
/// and answers the index to carry on at, or [`HALTED`] once the program has
/// stopped.
///
/// These are the instructions that allocate, format, call out of the VM or
/// otherwise do enough work that one more call is noise next to it. Keeping
/// them out of the loop keeps the loop small enough for its hot state to stay
/// in machine registers.
#[allow(unused_unsafe)]
#[cold]
#[inline(never)]
fn run_cold(m: &mut Machine<'_>, instructions: &[Instr], mut i: usize, mut regs: Regs) -> usize {
    let Machine {
        r,
        obj_pool,
        map_pool,
        str_pool,
        gc,
        err_ctx: _,
        callsite_registers,
        dyn_libs,
        structs,
        enums,
        host_sigs,
        host_dispatch,
        args,
        call_frames,
        recursion_stack,
        handle,
        dyn_lib_args,
        host_call_args,
        keep_alive,
        error_handles,
        failure,
    } = m;
    let r: &RegisterFile = r;
    let obj_pool: &mut ObjectPool = obj_pool;
    let map_pool: &mut MapPool = map_pool;
    let str_pool: &mut StringPool = str_pool;
    let gc: &mut GcState = gc;
    let callsite_registers: &[Vec<u16>] = callsite_registers;
    let dyn_libs: &[DynamicLibFn] = dyn_libs;
    let structs: &[Struct] = structs;
    let enums: &[EnumType] = enums;
    let host_sigs: &[HostFnSig] = host_sigs;
    let host_dispatch: &[HostDispatch] = host_dispatch;

    macro_rules! string {
        ($e: expr) => {
            Data::string($e, obj_pool, map_pool, str_pool, r, recursion_stack, gc)
        };
    }

    macro_rules! error_with_catch {
        ($err:expr) => {
            i = caught!($err);
            continue;
        };
        ($err:expr, $label:lifetime) => {
            i = caught!($err);
            continue $label;
        };
    }

    macro_rules! caught {
        ($err:expr) => {
            catch_error(
                $err,
                unsafe { *instructions.get_unchecked(i) },
                failure,
                error_handles,
                call_frames,
                args,
                callsite_registers,
                recursion_stack,
                regs,
                r,
                obj_pool,
                map_pool,
                str_pool,
                gc,
            )
        };
    }

    /// Orders two string registers with `$op` and writes the answer.
    ///
    /// The compiler emits these for two operands it typed as `string`, so one
    /// can still turn out to be something else at run time: a value that
    /// crossed a boundary the compiler could not check, such as a variadic
    /// host function returning something other than what its block declares.
    /// Ordering bytes that are not a string is not meaningful, so such an
    /// operand raises, the way joining one does.
    macro_rules! str_order {
        ($o1:expr, $o2:expr, $dest:expr, $op:tt) => {
            let d1 = regs[$o1];
            let d2 = regs[$o2];
            if !d1.is_string() || !d2.is_string() {
                let culprit = if d1.is_string() { d2 } else { d1 };
                error_with_catch!(ErrType::NotAString(culprit.type_name()));
            }
            regs[$dest] = (d1.as_str(str_pool) $op d2.as_str(str_pool)).into();
        };
    }

    // A one-pass loop: an instruction that jumps sets `i` and leaves with
    // `continue`, the way it would in the dispatch loop.
    #[allow(clippy::never_loop)]
    'main: for _ in 0..1 {
        match unsafe { *instructions.get_unchecked(i) } {
            // No library loads here: each binding is one of the runtime's own
            // functions, which the standard library's modules resolve to.
            #[cfg(target_arch = "wasm32")]
            Instr::CallDynamicLibFunc(fn_id, dest) => {
                use crate::intrinsics::{Arg, Ret};
                let func = unsafe { dyn_libs.get_unchecked(fn_id as usize) };
                let ret = {
                    // A short string lives inside its value, so the values are
                    // held here for as long as the arguments borrow them.
                    let mut values = [NULL; 2];
                    for (value, reg) in values.iter_mut().zip(args.iter()) {
                        *value = regs[*reg];
                    }
                    let mut call = [Arg::Int(0); 2];
                    for (slot, data) in call.iter_mut().zip(values.iter()) {
                        *slot = if data.is_int() {
                            Arg::Int(data.as_int())
                        } else if data.is_float() {
                            Arg::Float(data.as_float())
                        } else if data.is_string() {
                            Arg::Str(data.as_str(str_pool))
                        } else {
                            Arg::Int(0)
                        };
                    }
                    (func.intrinsic)(&call[..args.len().min(2)])
                };
                args.clear();
                regs[dest] = match ret {
                    Ret::Int(v) => v.into(),
                    Ret::Float(v) => v.into(),
                    Ret::Str(v) => string!(v),
                    Ret::Null => NULL,
                };
            }
            #[cfg(not(target_arch = "wasm32"))]
            Instr::CallDynamicLibFunc(fn_id, dest) => {
                dyn_lib_args.clear();
                dyn_lib_args.reserve_exact(args.len());
                let args_ptr = dyn_lib_args.as_mut_ptr();
                // Each argument libffi reads, borrowed from its slot in
                // `dyn_lib_args` or from `keep_alive`.
                let mut ffi_args: Vec<libffi::middle::Arg> = Vec::with_capacity(args.len());
                keep_alive.clear();

                for idx in 0..args.len() {
                    let data = regs[unsafe { *args.get_unchecked(idx) }];
                    if data.is_int() {
                        unsafe {
                            args_ptr.add(idx).write(data.as_int() as u64);
                            ffi_args.push(libffi::middle::Arg::new(&*args_ptr.add(idx)));
                        }
                    } else if data.is_float() {
                        unsafe {
                            args_ptr.add(idx).write(data.as_float().to_bits());
                            ffi_args.push(libffi::middle::Arg::new(&*args_ptr.add(idx)));
                        }
                    } else if data.is_string() {
                        let bytes = if let Ok(b) = std::ffi::CString::new(data.as_str(str_pool)) {
                            b.into_bytes_with_nul().into_boxed_slice()
                        } else {
                            error_with_catch!(ErrType::NullByteInString, 'main);
                        };
                        let ptr = bytes.as_ptr() as u64;
                        keep_alive.push(bytes);
                        unsafe {
                            args_ptr.add(idx).write(ptr);
                            ffi_args.push(libffi::middle::Arg::new(&*args_ptr.add(idx)));
                        }
                    } else if data.is_array() {
                        let ptr = ffi::array_to_c_ptr(data, obj_pool, str_pool, keep_alive) as u64;
                        unsafe {
                            args_ptr.add(idx).write(ptr);
                            ffi_args.push(libffi::middle::Arg::new(&*args_ptr.add(idx)));
                        }
                    } else if data.is_struct() {
                        let b = ffi::candela_struct_to_c_struct(
                            data.as_struct(),
                            obj_pool,
                            str_pool,
                            keep_alive,
                        )
                        .into_boxed_slice();
                        let ptr = &raw const *b;
                        keep_alive.push(b);
                        ffi_args.push(libffi::middle::Arg::new(unsafe { &*ptr }));
                    } else {
                        unsafe { unreachable_unchecked() }
                    }
                }
                args.clear();

                let func = unsafe { dyn_libs.get_unchecked(fn_id as usize) };
                // Call the function, and convert the result back into Data
                regs[dest] = unsafe {
                    match func.get_return_type() {
                        DataType::Int => func.cif.call::<i64>(func.ptr, &ffi_args).into(),
                        DataType::Float => func.cif.call::<f64>(func.ptr, &ffi_args).into(),
                        DataType::String => {
                            let ptr = func
                                .cif
                                .call::<*const std::ffi::c_char>(func.ptr, &ffi_args);
                            if ptr.is_null() {
                                cold_path();
                                NULL
                            } else {
                                string!(
                                    std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
                                )
                            }
                        }
                        DataType::Struct(struct_idx) => {
                            let struct_fields =
                                unsafe { &structs.get_unchecked(*struct_idx as usize).fields };
                            let (struct_size, _, field_offsets) =
                                ffi::get_struct_size_datatype(struct_fields, structs);
                            let mut return_buf: Vec<u8> = vec![0; struct_size];
                            func.cif.call_return_into(
                                func.ptr,
                                &ffi_args,
                                libffi::middle::ret(&mut return_buf[..]),
                            );

                            let new_id = ffi::c_struct_to_candela_struct(
                                &return_buf,
                                &field_offsets,
                                obj_pool,
                                map_pool,
                                str_pool,
                                struct_fields,
                                r,
                                recursion_stack,
                                gc,
                                structs,
                            );
                            Data::struct_instance(*struct_idx, new_id)
                        }
                        DataType::Null => {
                            func.cif.call::<()>(func.ptr, &ffi_args);
                            NULL
                        }
                        DataType::Array(_) => {
                            error_with_catch!(ErrType::CArrayReturnTypeNotSupported);
                        }
                        t => {
                            error_with_catch!(ErrType::InvalidReturnType(t));
                        }
                    }
                };
            }
            Instr::CallHostFunc(fn_id, dest) => {
                let dispatch = unsafe { host_dispatch.get_unchecked(fn_id as usize) };

                // Marshal each argument register (scalar or heap handle) into a
                // host `Value`, recursively for arrays/maps/structs.
                host_call_args.clear();
                for idx in 0..args.len() {
                    let data = regs[unsafe { *args.get_unchecked(idx) }];
                    host_call_args.push(crate::embed::unmarshal_value(
                        data, obj_pool, map_pool, str_pool, structs, enums,
                    ));
                }
                args.clear();

                match (**dispatch)(host_call_args) {
                    // Marshal the result back, allocating arrays/maps into the
                    // pools. `Value::Null` (including a void closure's `()`)
                    // becomes NULL, which is what register 0 expects for a
                    // discarded result.
                    Ok(value) => {
                        regs[dest] = crate::embed::marshal_value(
                            &value,
                            r,
                            recursion_stack,
                            obj_pool,
                            map_pool,
                            str_pool,
                            gc,
                        );
                    }
                    // The closure raised. Report it where the script called it,
                    // naming the function the way the script spells it.
                    Err(err) => {
                        let function = host_sigs.get(fn_id as usize).map_or_else(
                            || format!("#{fn_id}"),
                            |sig| crate::embed::qualified_name(&sig.namespace, &sig.name),
                        );
                        error_with_catch!(ErrType::HostFn(&function, err.message()));
                    }
                }
            }
            Instr::AddStr(o1, o2, dest) => {
                let d1 = regs[o1];
                let d2 = regs[o2];
                // Concatenation is compiled from static string types, so both
                // operands are strings unless a value crossed a boundary the
                // compiler could not check: a variadic host function returning
                // something other than what its block declares. Reading that as
                // a string would read unrelated bits, so it raises instead.
                if !d1.is_string() || !d2.is_string() {
                    let culprit = if d1.is_string() { d2 } else { d1 };
                    error_with_catch!(ErrType::NotAString(culprit.type_name()));
                }
                let left = d1.as_str(str_pool);
                let right = d2.as_str(str_pool);
                let mut s = String::with_capacity(left.len() + right.len());
                s.push_str(left);
                s.push_str(right);
                regs[dest] = string!(s);
            }
            Instr::EmptyArray(arr_reg_id) => {
                let array_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                regs[arr_reg_id] = Data::array(array_id);
            }
            Instr::EmptyFnValue(value_reg_id) => {
                let array_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                regs[value_reg_id] = Data::function(array_id);
            }
            Instr::CloneArray(src_reg, dest_reg, len) => {
                let src_id = regs[src_reg].as_array();
                let new_id =
                    alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc) as usize;
                let src_ptr = obj_pool[src_id].as_ptr();
                let dst = obj_pool.get_mut(new_id);
                dst.reserve_exact(len as usize);
                unsafe {
                    dst.set_len(len as usize);
                    std::ptr::copy_nonoverlapping(src_ptr, dst.as_mut_ptr(), len as usize);
                }
                regs[dest_reg] = Data::array(new_id as u32);
            }
            Instr::CloneEnum(src_reg, dest_reg) => {
                let new_id =
                    alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc) as usize;
                let src_reg = regs[src_reg];
                let src = &obj_pool[src_reg.as_enum()];
                let len = src.len();
                unsafe {
                    let src_ptr = src.as_ptr();
                    let dst = obj_pool.get_mut(new_id);
                    dst.reserve_exact(len);
                    dst.set_len(len);
                    std::ptr::copy_nonoverlapping(src_ptr, dst.as_mut_ptr(), len);
                }
                regs[dest_reg] = Data::enum_instance(src_reg.enum_type_id(), new_id as u32);
            }
            Instr::AddArray(o1, o2, dest) => {
                let arr_a_id = regs[o1].as_array();
                let arr_b_id = regs[o2].as_array();
                let array_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                let array_idx = array_id as usize;
                unsafe {
                    let array_pool_ptr = obj_pool.as_mut_ptr();
                    let arr_a = &*array_pool_ptr.add(arr_a_id);
                    let arr_b = &*array_pool_ptr.add(arr_b_id);
                    let combined = &mut *array_pool_ptr.add(array_idx);

                    combined.reserve(arr_a.len() + arr_b.len());
                    combined.extend_from_slice(arr_a);
                    combined.extend_from_slice(arr_b);
                }
                regs[dest] = Data::array(array_id);
            }
            Instr::DivInt(o1, o2, dest) => {
                let b = regs[o2].as_int();
                if b == 0 {
                    error_with_catch!(ErrType::DivisionByZero);
                }
                regs[dest] = (regs[o1].as_int() / b).into();
            }
            Instr::ModFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float() % regs[o2].as_float()).into();
            }
            Instr::ModInt(o1, o2, dest) => {
                let b = regs[o2].as_int();
                if b == 0 {
                    error_with_catch!(ErrType::ModuloByZero);
                }
                regs[dest] = (regs[o1].as_int() % b).into();
            }
            Instr::PowFloat(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_float().powf(regs[o2].as_float())).into();
            }
            Instr::PowInt(o1, o2, dest) => {
                let exp = regs[o2].as_int();
                if exp < 0 {
                    error_with_catch!(ErrType::NegativeExponent(exp));
                }
                regs[dest] = (regs[o1].as_int().pow(exp as u32)).into();
            }
            Instr::BitAndInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() & regs[o2].as_int()).into();
            }
            Instr::BitOrInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() | regs[o2].as_int()).into();
            }
            Instr::BitXorInt(o1, o2, dest) => {
                regs[dest] = (regs[o1].as_int() ^ regs[o2].as_int()).into();
            }
            Instr::BitNotInt(o1, dest) => {
                regs[dest] = (!regs[o1].as_int()).into();
            }
            Instr::ShlInt(o1, o2, dest) => {
                let count = regs[o2].as_int();
                if !shift_count_in_range(count) {
                    error_with_catch!(ErrType::ShiftCountOutOfRange(count));
                }
                regs[dest] = (regs[o1].as_int() << count).into();
            }
            Instr::ShrInt(o1, o2, dest) => {
                let count = regs[o2].as_int();
                if !shift_count_in_range(count) {
                    error_with_catch!(ErrType::ShiftCountOutOfRange(count));
                }
                regs[dest] = (regs[o1].as_int() >> count).into();
            }
            Instr::ObjEq(o1, o2, dest) => {
                regs[dest] = obj_eq(regs[o1], regs[o2], obj_pool, map_pool, str_pool).into();
            }
            Instr::ObjNotEqJmp(o1, o2, jump_size) => {
                if !obj_eq(regs[o1], regs[o2], obj_pool, map_pool, str_pool) {
                    i += jump_size as usize;
                    continue;
                }
            }
            Instr::StrNotEqJmp(o1, o2, jump_size) => {
                if !str_eq(regs[o1], regs[o2], str_pool) {
                    i += jump_size as usize;
                    continue;
                }
            }
            Instr::ObjNotEq(o1, o2, dest) => {
                regs[dest] = (!obj_eq(regs[o1], regs[o2], obj_pool, map_pool, str_pool)).into();
            }
            Instr::StrEq(o1, o2, dest) => {
                regs[dest] = str_eq(regs[o1], regs[o2], str_pool).into();
            }
            Instr::StrNotEq(o1, o2, dest) => {
                regs[dest] = (!str_eq(regs[o1], regs[o2], str_pool)).into();
            }
            Instr::ObjEqJmp(o1, o2, jump_size) => {
                if obj_eq(regs[o1], regs[o2], obj_pool, map_pool, str_pool) {
                    i += jump_size as usize;
                    continue;
                }
            }
            Instr::StrEqJmp(o1, o2, jump_size) => {
                if str_eq(regs[o1], regs[o2], str_pool) {
                    i += jump_size as usize;
                    continue;
                }
            }
            Instr::SupStr(o1, o2, dest) => {
                str_order!(o1, o2, dest, >);
            }
            Instr::SupEqStr(o1, o2, dest) => {
                str_order!(o1, o2, dest, >=);
            }
            Instr::InfStr(o1, o2, dest) => {
                str_order!(o1, o2, dest, <);
            }
            Instr::InfEqStr(o1, o2, dest) => {
                str_order!(o1, o2, dest, <=);
            }
            Instr::Print(tgt) => {
                let tgt = regs[tgt];
                if tgt.is_string() {
                    outln!(*handle, "{}", tgt.as_str(str_pool));
                } else if tgt.is_int() {
                    outln!(*handle, "{}", tgt.as_int());
                } else if tgt.is_float() {
                    outln!(*handle, "{}", format_float(tgt.as_float()));
                } else if tgt.is_bool() {
                    outln!(*handle, "{}", tgt.as_bool());
                } else if tgt.is_function() {
                    outln!(*handle, "{FUNCTION_TEXT}");
                } else if tgt.is_array() {
                    let array = &obj_pool[tgt.as_array()];
                    out!(*handle, "[");
                    for (idx, item) in array.iter().enumerate() {
                        if idx != 0 {
                            out!(*handle, ",");
                        }
                        out!(
                            *handle,
                            "{}",
                            item.format(obj_pool, str_pool, map_pool, structs, enums, false)
                        );
                    }
                    outln!(*handle, "]");
                } else if tgt.is_struct() {
                    let s = unsafe { structs.get_unchecked(tgt.struct_type_id() as usize) };
                    let s_name = &s.name;
                    let s_fields = &s.fields;
                    out!(*handle, "{s_name} {{");
                    for (idx, item) in obj_pool[tgt.as_struct()].iter().enumerate() {
                        if idx != 0 {
                            out!(*handle, ",");
                        }
                        out!(
                            *handle,
                            "{}:{}",
                            unsafe { &s_fields.get_unchecked(idx).0 },
                            item.format(obj_pool, str_pool, map_pool, structs, enums, false)
                        );
                    }
                    outln!(*handle, "}}");
                } else if tgt.is_enum() {
                    let e = unsafe { enums.get_unchecked(tgt.enum_type_id() as usize) };
                    let entry = &obj_pool[tgt.as_enum()];
                    let tag = entry[0].as_int() as usize;
                    let variant = unsafe { e.variants.get_unchecked(tag) };
                    if entry.len() <= 1 {
                        outln!(*handle, "{}", variant.name);
                    } else {
                        out!(*handle, "{}(", variant.name);
                        for (idx, item) in entry[1..].iter().enumerate() {
                            if idx != 0 {
                                out!(*handle, ",");
                            }
                            out!(
                                *handle,
                                "{}",
                                item.format(obj_pool, str_pool, map_pool, structs, enums, false)
                            );
                        }
                        outln!(*handle, ")");
                    }
                } else if tgt.is_map() {
                    let m = &map_pool[tgt.as_map()];
                    out!(*handle, "{{");
                    for (i, (key, val)) in m.iter().enumerate() {
                        if i != 0 {
                            out!(*handle, ",");
                        }
                        out!(
                            *handle,
                            "{}:{}",
                            key.format(obj_pool, str_pool, map_pool, structs, enums, false),
                            val.format(obj_pool, str_pool, map_pool, structs, enums, false),
                        );
                    }
                    outln!(*handle, "}}");
                }
            }
            Instr::SetElementString(string_reg_id, new_str_reg_id, idx) => {
                let index = regs[idx].as_int();
                let temp_str_reg_id = regs[string_reg_id];
                let source_string = temp_str_reg_id.as_str(str_pool);
                if index < 0 || (index as u64) >= source_string.len() as u64 {
                    error_with_catch!(ErrType::IndexOutOfBounds(source_string.len(), index));
                }
                let mut temp = source_string.to_owned();
                temp.remove(index as usize);
                temp.insert_str(index as usize, regs[new_str_reg_id].as_str(str_pool));
                regs[string_reg_id] = string!(temp);
            }
            Instr::GetSliceArray(array_reg_id, idx_start_id, dest_reg_id) => {
                let idx_start = regs[idx_start_id].as_int();
                let idx_end = regs[args.pop_unchecked()].as_int();
                let arr_id = regs[array_reg_id].as_array();
                let array = &obj_pool[arr_id];
                // A slice runs from `start` to `end` with `0 <= start <= end <= len`,
                // so a start sitting on the end of the list is in range and yields
                // an empty list. A negative bound casts to a huge `u64` and is
                // rejected by the same comparison.
                if (idx_end as u64) > array.len() as u64
                    || (idx_start as u64) > array.len() as u64
                    || idx_start > idx_end
                {
                    error_with_catch!(ErrType::SliceOutOfBounds(array.len(), idx_start, idx_end));
                }
                let new_array_id =
                    alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                unsafe {
                    if arr_id < (new_array_id as usize) {
                        let (left, right) = obj_pool.split_at_mut(new_array_id as usize);
                        right.get_unchecked_mut(0).extend_from_slice(
                            left.get_unchecked(arr_id)
                                .get_unchecked((idx_start as usize)..(idx_end as usize)),
                        );
                    } else {
                        let (left, right) = obj_pool.split_at_mut(arr_id);
                        left.get_unchecked_mut(new_array_id as usize)
                            .extend_from_slice(
                                right
                                    .get_unchecked(0)
                                    .get_unchecked((idx_start as usize)..(idx_end as usize)),
                            );
                    }
                }
                regs[dest_reg_id] = Data::array(new_array_id);
            }
            // A string is indexed by character, so the position has to be
            // turned into the byte offset that character starts at. On a
            // string of single-byte characters the two are the same and the
            // index reads straight out of the bytes.
            Instr::GetIndexString(tgt, index, dest) => {
                let idx = regs[index].as_int();
                let tgt_data = regs[tgt];
                let char_len = tgt_data.str_char_len(str_pool);
                if idx < 0 || (idx as u64) >= char_len as u64 {
                    error_with_catch!(ErrType::IndexOutOfBounds(char_len, idx));
                }
                let bytes = tgt_data.as_str(str_pool).as_bytes();
                // As many characters as bytes means every character takes one
                // of them, so the position is already the offset and the
                // character it names is that one byte. A character takes at
                // most four bytes either way, so the answer always fits the
                // inlined string form.
                let (start, end) = if char_len == bytes.len() {
                    (idx as usize, idx as usize + 1)
                } else {
                    let start = tgt_data.str_byte_offset(idx as usize, str_pool);
                    (
                        start,
                        start + utf8_width(unsafe { *bytes.get_unchecked(start) }),
                    )
                };
                regs[dest] = Data::small_str(unsafe {
                    std::str::from_utf8_unchecked(bytes.get_unchecked(start..end))
                });
            }
            Instr::GetSliceString(str_reg_id, idx_start, dest_reg_id) => {
                let idx_start = regs[idx_start].as_int();
                let idx_end = regs[args.pop_unchecked()].as_int();
                let src = regs[str_reg_id];
                let char_len = src.str_char_len(str_pool);
                // Same rule as a list slice: a start on the end of the string is
                // in range and yields "".
                if (idx_end as u64) > char_len as u64
                    || (idx_start as u64) > char_len as u64
                    || idx_start > idx_end
                {
                    error_with_catch!(ErrType::SliceOutOfBounds(char_len, idx_start, idx_end));
                }
                let text = src.as_str(str_pool);
                let (start, end) = if char_len == text.len() {
                    (idx_start as usize, idx_end as usize)
                } else {
                    // The end is found by walking on from the start rather
                    // than from the first byte again, which is also what
                    // leaves a walk over the string standing at the start of
                    // this cut for the next one.
                    let start = src.str_byte_offset(idx_start as usize, str_pool);
                    let span = char_byte_offset(&text[start..], (idx_end - idx_start) as usize);
                    (start, start + span)
                };
                // The cut is copied out before the pool is written to, because
                // storing the result can move the string it was taken from.
                let part = src.as_str(str_pool)[start..end].to_smolstr();
                regs[dest_reg_id] = string!(part.as_str());
            }
            Instr::NewCell(src, dest) => {
                let cell_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                obj_pool.get_mut(cell_id as usize).push(regs.get(src));
                regs[dest] = Data::array(cell_id);
            }
            Instr::Remove(array, idx) => {
                let id = regs[array].as_array();
                let arr = obj_pool.get_mut(id);
                let index = regs[idx].as_int();
                if index < 0 || (index as u64) >= arr.len() as u64 {
                    error_with_catch!(ErrType::IndexOutOfBounds(arr.len(), index));
                }
                let removed = arr.remove(index as usize);
                if gc.marking() {
                    shade_overwritten(gc, str_pool, removed);
                    rescan_list(gc, id);
                }
            }
            Instr::MapGet(map_reg_id, key_reg_id, dest_reg_id) => {
                regs[dest_reg_id] = if let Some(elem) = unsafe {
                    map_pool[regs[map_reg_id].as_map()].get(&KeyByText {
                        key: regs[key_reg_id],
                        strings: str_pool,
                    })
                } {
                    *elem
                } else {
                    cold_path();
                    error_with_catch!(ErrType::UnknownMapKey(
                        regs[key_reg_id]
                            .format(obj_pool, str_pool, map_pool, structs, enums, false)
                            .as_str()
                    ));
                };
            }
            Instr::MapInsert(map_pool_id, key_reg_id, val_reg_id) => {
                let old = map_insert(
                    &mut map_pool[map_pool_id as usize],
                    regs[key_reg_id],
                    regs[val_reg_id],
                    str_pool,
                );
                if let Some(old) = old
                    && gc.marking()
                {
                    shade_overwritten(gc, str_pool, old);
                }
            }
            Instr::MapInsertReg(map_reg_id, key_reg_id, val_reg_id) => {
                let old = map_insert(
                    &mut map_pool[regs[map_reg_id].as_map()],
                    regs[key_reg_id],
                    regs[val_reg_id],
                    str_pool,
                );
                if let Some(old) = old
                    && gc.marking()
                {
                    shade_overwritten(gc, str_pool, old);
                }
            }
            // A key that is not in the map leaves it as it was: taking an entry
            // out is a request for the key to be absent, and it already is.
            Instr::MapRemove(map_reg_id, key_reg_id) => {
                let id = regs[map_reg_id].as_map();
                // `shift_remove`, not `swap_remove`: the entries after the one
                // taken out keep their order, and a key put back afterwards
                // lands at the end.
                let removed = map_pool[id].shift_remove_entry(&KeyByText {
                    key: regs[key_reg_id],
                    strings: str_pool,
                });
                if let Some((key, value)) = removed
                    && gc.marking()
                {
                    shade_overwritten(gc, str_pool, key);
                    shade_overwritten(gc, str_pool, value);
                    rescan_map(gc, id);
                }
            }
            Instr::CloneMap(src_reg, dest_reg) => {
                let new_map = map_pool[regs[src_reg].as_map()].clone();
                let new_id = alloc_map(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                unsafe {
                    map_pool[new_id as usize] = new_map;
                }
                regs[dest_reg] = Data::map(new_id);
            }
            Instr::CallLibFunc(LibFunc::Uppercase, source_string_reg_id, dest_reg_id) => {
                regs[dest_reg_id] =
                    string!(regs[source_string_reg_id].as_str(str_pool).to_uppercase());
            }
            Instr::CallLibFunc(LibFunc::Lowercase, source_string_reg_id, dest_reg_id) => {
                regs[dest_reg_id] =
                    string!(regs[source_string_reg_id].as_str(str_pool).to_lowercase());
            }
            Instr::CallLibFunc(LibFunc::Contains, tgt, dest) => {
                let reg = regs[tgt];
                if reg.is_string() {
                    let str = reg.as_str(str_pool);
                    let temp_arg = regs[args.pop_unchecked()];
                    let arg = temp_arg.as_str(str_pool);
                    // regs[dest] = str.contains(arg).into();
                    regs[dest] = memmem::find(str.as_bytes(), arg.as_bytes())
                        .is_some()
                        .into();
                } else if reg.is_array() {
                    let arg = regs[args.pop_unchecked()];
                    regs[dest] = obj_pool[reg.as_array()]
                        .iter()
                        .any(|&x| same_key(x, arg, str_pool))
                        .into();
                } else if reg.is_map() {
                    let arg = regs[args.pop_unchecked()];
                    regs[dest] = map_pool[reg.as_map()]
                        .contains_key(&KeyByText {
                            key: arg,
                            strings: str_pool,
                        })
                        .into();
                }
            }
            Instr::CallLibFunc(LibFunc::Trim, tgt, dest) => {
                regs[dest] = string!(regs[tgt].as_str(str_pool).trim());
            }
            Instr::CallLibFunc(LibFunc::TrimSequence, tgt, dest) => {
                let temp_arg = regs[args.pop_unchecked()];
                let arg = temp_arg.as_str(str_pool);
                let chars: Vec<char> = arg.chars().collect();
                regs[dest] = string!(regs[tgt].as_str(str_pool).trim_matches(&chars[..]));
            }
            Instr::CallLibFunc(LibFunc::Find, tgt, dest) => {
                let reg = regs[tgt];
                if reg.is_string() {
                    let str = reg.as_str(str_pool);
                    let temp_elem = regs[args.pop_unchecked()];
                    let element = temp_elem.as_str(str_pool);
                    regs[dest] =
                        if let Some(byte_idx) = memmem::find(str.as_bytes(), element.as_bytes()) {
                            // The search works in bytes, and the answer is a
                            // character position: on a string of single-byte
                            // characters they are the same, otherwise count the
                            // characters the match sits past.
                            if reg.str_is_ascii(str_pool) {
                                byte_idx as i64
                            } else {
                                count_chars(&str[..byte_idx]) as i64
                            }
                        } else {
                            cold_path();
                            -1
                        }
                        .into();
                } else if reg.is_array() {
                    let arr_id = reg.as_array();
                    let element = regs[args.pop_unchecked()];
                    regs[dest] = if let Some(idx) = obj_pool[arr_id]
                        .iter()
                        .position(|&x| same_key(x, element, str_pool))
                    {
                        idx as i64
                    } else {
                        cold_path();
                        -1
                    }
                    .into();
                }
            }
            Instr::CallLibFunc(LibFunc::IsFloat, tgt, dest) => {
                let temp_tgt = regs[tgt];
                let num = temp_tgt.as_str(str_pool);
                regs[dest] = (num.parse::<i64>().is_err() && num.parse::<f64>().is_ok()).into();
            }
            Instr::CallLibFunc(LibFunc::IsInt, tgt, dest) => {
                regs[dest] = regs[tgt].as_str(str_pool).parse::<i64>().is_ok().into();
            }
            Instr::CallLibFunc(LibFunc::TrimLeft, tgt, dest) => {
                regs[dest] = string!(regs[tgt].as_str(str_pool).trim_start());
            }
            Instr::CallLibFunc(LibFunc::TrimRight, tgt, dest) => {
                regs[dest] = string!(regs[tgt].as_str(str_pool).trim_end());
            }
            Instr::CallLibFunc(LibFunc::TrimSequenceLeft, tgt, dest) => {
                let chars: Vec<char> = regs[args.pop_unchecked()]
                    .as_str(str_pool)
                    .chars()
                    .collect();
                regs[dest] = string!(regs[tgt].as_str(str_pool).trim_start_matches(&chars[..]));
            }
            Instr::CallLibFunc(LibFunc::TrimSequenceRight, tgt, dest) => {
                let chars: Vec<char> = regs[args.pop_unchecked()]
                    .as_str(str_pool)
                    .chars()
                    .collect();
                regs[dest] = string!(regs[tgt].as_str(str_pool).trim_end_matches(&chars[..]));
            }
            Instr::CallLibFunc(LibFunc::Repeat, tgt, dest) => {
                let reg = regs[tgt];
                if reg.is_string() {
                    let str = reg.as_str(str_pool);
                    let repeat_count = regs[args.pop_unchecked()].as_int();
                    regs[dest] = string!(str.repeat(repeat_count as usize));
                } else if reg.is_array() {
                    let repeat_count = regs[args.pop_unchecked()].as_int();
                    let array_id =
                        alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                    obj_pool[array_id as usize] =
                        obj_pool[reg.as_array()].repeat(repeat_count as usize);
                    regs[dest] = Data::array(array_id);
                }
            }
            Instr::CallLibFunc(LibFunc::Round, tgt, dest) => {
                regs[dest] = regs[tgt].as_float().round().into();
            }
            Instr::CallLibFunc(LibFunc::Abs, tgt, dest) => {
                let tgt = regs[tgt];
                regs[dest] = if tgt.is_float() {
                    tgt.as_float().abs().into()
                } else {
                    tgt.as_int().abs().into()
                }
            }
            Instr::CallLibFunc(LibFunc::Reverse, tgt, dest) => {
                regs[dest] = string!(regs[tgt].as_str(str_pool).chars().rev().collect::<String>());
            }
            Instr::CallLibFuncVoid(LibFuncVoid::Reverse, tgt, _) => {
                let id = regs[tgt].as_array();
                obj_pool.get_mut(id).reverse();
                if gc.marking() {
                    rescan_list(gc, id);
                }
            }
            Instr::CallLibFunc(LibFunc::SqrtFloat, tgt, dest) => {
                regs[dest] = regs[tgt].as_float().sqrt().into();
            }
            Instr::CallLibFunc(LibFunc::Float, tgt, dest) => {
                let reg = regs[tgt];
                if reg.is_int() {
                    regs[dest] = (reg.as_int() as f64).into();
                } else if reg.is_string() {
                    let str = reg.as_str(str_pool);
                    regs[dest] = if let Ok(f) = lexical_core::parse::<f64>(str.as_bytes()) {
                        f.into()
                    } else {
                        error_with_catch!(ErrType::InvalidFloat);
                    }
                }
            }
            Instr::CallLibFunc(LibFunc::Int, tgt, dest) => {
                let reg = regs[tgt];
                if reg.is_float() {
                    regs[dest] = (reg.as_float() as i64).into();
                } else if reg.is_string() {
                    let str = reg.as_str(str_pool);
                    regs[dest] = if let Ok(i) = lexical_core::parse::<i64>(str.as_bytes()) {
                        i.into()
                    } else {
                        cold_path();
                        error_with_catch!(ErrType::InvalidInt);
                    }
                }
            }
            Instr::CallLibFunc(LibFunc::Str, tgt, dest) => {
                let value = regs[tgt];
                regs[dest] = if value.is_int() {
                    let mut buffer = [0u8; i64::FORMATTED_SIZE_DECIMAL];
                    let digits = lexical_core::write(value.as_int(), &mut buffer);
                    string!(unsafe { str::from_utf8_unchecked(digits) })
                } else if value.is_float() {
                    string!(format_float(value.as_float()).as_str())
                } else if value.is_bool() {
                    Data::small_str(if value.as_bool() { "true" } else { "false" })
                } else if value.is_string() {
                    value
                } else {
                    string!(
                        value
                            .format(obj_pool, str_pool, map_pool, structs, enums, false)
                            .as_str()
                    )
                };
            }
            Instr::CallLibFunc(LibFunc::Bool, tgt, dest) => {
                let temp_tgt = regs[tgt];
                let str = temp_tgt.as_str(str_pool);
                regs[dest] = if str == "true" {
                    TRUE
                } else if str == "false" {
                    FALSE
                } else {
                    cold_path();
                    error_with_catch!(ErrType::InvalidBool);
                }
            }
            #[cfg(target_arch = "wasm32")]
            Instr::CallLibFunc(LibFunc::Input, _, _) => wasm_error("WASM does not support input()"),
            #[cfg(not(target_arch = "wasm32"))]
            Instr::CallLibFunc(LibFunc::Input, tgt, dest) => {
                let temp_tgt = regs[tgt];
                let str_msg = temp_tgt.as_str(str_pool);
                out!(*handle, "{str_msg}");
                // The prompt has no newline of its own, so it sits in the
                // buffer until something pushes it out. A lost prompt is not
                // worth ending the run over either.
                let _ = handle.flush();
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).unwrap();
                regs[dest] = string!(line.trim_end_matches(['\n', '\r']));
            }
            Instr::CallLibFunc(LibFunc::Floor, tgt, dest) => {
                regs[dest] = regs[tgt].as_float().floor().into();
            }
            Instr::CallLibFunc(LibFunc::TheAnswer, _, dest) => {
                outln!(
                    *handle,
                    "The answer to the Ultimate Question of Life, the Universe, and Everything is 42."
                );
                regs[dest] = 42.into();
            }
            Instr::CallLibFunc(LibFunc::Len, tgt, dest) => {
                let reg = regs[tgt];
                if reg.is_array() {
                    regs[dest] = (obj_pool[reg.as_array()].len() as i64).into();
                } else if reg.is_string() {
                    regs[dest] = (reg.str_char_len(str_pool) as i64).into();
                } else if reg.is_map() {
                    regs[dest] = (map_pool[reg.as_map()].len() as i64).into();
                } else {
                    unsafe { unreachable_unchecked() }
                }
            }
            Instr::CallLibFunc(LibFunc::Keys, tgt, dest) => {
                let map_data = regs[tgt];
                let out_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                let keys: Vec<Data> = map_pool[map_data.as_map()].keys().copied().collect();
                let out = obj_pool.get_mut(out_id as usize);
                out.clear();
                out.extend(keys);
                regs[dest] = Data::array(out_id);
            }
            Instr::CallLibFunc(LibFunc::Values, tgt, dest) => {
                let map_data = regs[tgt];
                let out_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                let vals: Vec<Data> = map_pool[map_data.as_map()].values().copied().collect();
                let out = obj_pool.get_mut(out_id as usize);
                out.clear();
                out.extend(vals);
                regs[dest] = Data::array(out_id);
            }
            Instr::CallLibFunc(LibFunc::JsonParse, tgt, dest) => {
                let input = regs[tgt].as_str(str_pool).to_owned();
                match crate::json::json_parse(
                    &input,
                    obj_pool,
                    map_pool,
                    str_pool,
                    r,
                    recursion_stack,
                    gc,
                ) {
                    Ok(value) => regs[dest] = value,
                    Err(reason) => {
                        error_with_catch!(ErrType::JsonParse(reason));
                    }
                }
            }
            Instr::CallLibFunc(LibFunc::JsonStringify, tgt, dest) => {
                let json = crate::json::json_stringify(
                    regs[tgt], obj_pool, map_pool, str_pool, structs, enums,
                );
                regs[dest] = string!(json);
            }
            Instr::CallLibFunc(LibFunc::IsIntVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_int().into();
            }
            Instr::CallLibFunc(LibFunc::IsFloatVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_float().into();
            }
            Instr::CallLibFunc(LibFunc::IsStrVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_string().into();
            }
            Instr::CallLibFunc(LibFunc::IsBoolVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_bool().into();
            }
            Instr::CallLibFunc(LibFunc::IsListVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_array().into();
            }
            Instr::CallLibFunc(LibFunc::IsMapVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_map().into();
            }
            Instr::CallLibFunc(LibFunc::IsNullVal, tgt, dest) => {
                regs[dest] = regs[tgt].is_null().into();
            }
            Instr::CallLibFunc(LibFunc::AsIntVal, tgt, dest) => {
                let v = regs[tgt];
                if v.is_int() {
                    regs[dest] = v;
                } else {
                    error_with_catch!(ErrType::BadDowncast("int", v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::AsFloatVal, tgt, dest) => {
                let v = regs[tgt];
                if v.is_float() {
                    regs[dest] = v;
                } else {
                    error_with_catch!(ErrType::BadDowncast("float", v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::AsStrVal, tgt, dest) => {
                let v = regs[tgt];
                if v.is_string() {
                    regs[dest] = v;
                } else {
                    error_with_catch!(ErrType::BadDowncast("string", v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::AsBoolVal, tgt, dest) => {
                let v = regs[tgt];
                if v.is_bool() {
                    regs[dest] = v;
                } else {
                    error_with_catch!(ErrType::BadDowncast("bool", v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::AsListVal, tgt, dest) => {
                let v = regs[tgt];
                if v.is_array() {
                    regs[dest] = v;
                } else {
                    error_with_catch!(ErrType::BadDowncast("list", v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::AsMapVal, tgt, dest) => {
                let v = regs[tgt];
                if v.is_map() {
                    regs[dest] = v;
                } else {
                    error_with_catch!(ErrType::BadDowncast("map", v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::AsTypeVal, tgt, dest) => {
                let codes = &obj_pool[regs[args.pop_unchecked()].as_array()];
                let v = regs[tgt];
                if codes.iter().any(|code| has_type_code(v, code.as_int())) {
                    regs[dest] = v;
                } else {
                    let wanted = type_codes_text(codes, structs, enums);
                    error_with_catch!(ErrType::BadDowncast(&wanted, v.type_name()));
                }
            }
            Instr::CallLibFunc(LibFunc::StartsWith, source_register, dest_register) => {
                regs[dest_register] = regs[source_register]
                    .as_str(str_pool)
                    .starts_with(regs[args.pop_unchecked()].as_str(str_pool))
                    .into();
            }
            Instr::CallLibFunc(LibFunc::EndsWith, source_register, dest_register) => {
                regs[dest_register] = regs[source_register]
                    .as_str(str_pool)
                    .ends_with(regs[args.pop_unchecked()].as_str(str_pool))
                    .into();
            }
            #[allow(clippy::no_effect_replace)]
            Instr::CallLibFunc(LibFunc::Replace, source_register, dest_register) => {
                regs[dest_register] = string!(regs[source_register].as_str(str_pool).replace(
                    regs[args.pop_unchecked()].as_str(str_pool),
                    regs[args.pop_unchecked()].as_str(str_pool),
                ));
            }
            Instr::CallLibFunc(LibFunc::Split, source_register, dest_register) => {
                let source = regs[source_register];
                let separator = unsafe { args.pop_unchecked() };
                if source.is_string() {
                    let output_id =
                        alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                    // Each part's allocation can run the collector, so the
                    // list sits on the recursion stack, a root, while it is
                    // filled. The source and separator stay in their
                    // registers, so their text stays where it is.
                    recursion_stack.0.push(Data::array(output_id));
                    let source = source.as_str(str_pool);
                    let separator_data = regs[separator];
                    let separator = separator_data.as_str(str_pool);
                    obj_pool
                        .get_mut(output_id as usize)
                        .reserve(source.len() / 4);
                    let separator_len = separator.len();
                    if separator_len == 0 {
                        // An empty separator answers the characters. Matching
                        // it at every byte offset instead would report a match
                        // before the first character and after the last, and
                        // would cut a multi-byte character in half. A character
                        // is at most four bytes, so every part fits the inline
                        // string form and none reaches the pool.
                        let output = obj_pool.get_mut(output_id as usize);
                        for (byte_i, character) in source.char_indices() {
                            output.push(Data::small_str(
                                &source[byte_i..byte_i + character.len_utf8()],
                            ));
                        }
                    } else {
                        let mut i = 0;
                        for part_i in memmem::find_iter(source.as_bytes(), separator.as_bytes()) {
                            let part = string!(&source[i..part_i]);
                            obj_pool.get_mut(output_id as usize).push(part);
                            i = part_i + separator_len;
                        }
                        let part = string!(&source[i..]);
                        obj_pool.get_mut(output_id as usize).push(part);
                    }
                    recursion_stack.0.pop();
                    regs[dest_register] = Data::array(output_id);
                } else if source.is_array() {
                    let source_array_id = source.as_array();
                    let separator = regs[separator];
                    let source_array = &obj_pool[source_array_id];

                    let mut split_ranges: Vec<(usize, usize)> =
                        Vec::with_capacity(source_array.len() + 1);

                    let mut start = 0;
                    for (idx, item) in source_array.iter().enumerate() {
                        if *item == separator {
                            split_ranges.push((start, idx));
                            start = idx + 1;
                        }
                    }
                    split_ranges.push((start, source_array.len()));

                    // One array per range. Each goes onto the recursion stack
                    // as soon as it is made, because that stack is a
                    // collection root and a Rust-local buffer is not: the next
                    // range's allocation can run the collector, which would
                    // free the lists made before it and hand their slots out
                    // again.
                    let base = recursion_stack.len();
                    for (start, end) in split_ranges {
                        let dest_array_id =
                            alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc)
                                as usize;
                        unsafe {
                            if dest_array_id < source_array_id {
                                let (left, right) = obj_pool.split_at_mut(source_array_id);
                                left.get_unchecked_mut(dest_array_id).extend_from_slice(
                                    right.get_unchecked(0).get_unchecked(start..end),
                                );
                            } else {
                                let (left, right) = obj_pool.split_at_mut(dest_array_id);
                                right.get_unchecked_mut(0).extend_from_slice(
                                    left.get_unchecked(source_array_id)
                                        .get_unchecked(start..end),
                                );
                            }
                        }
                        recursion_stack.0.push(Data::array(dest_array_id as u32));
                    }

                    let array_id =
                        alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                    let parts = obj_pool.get_mut(array_id as usize);
                    parts.extend(recursion_stack.0.drain(base..));

                    regs[dest_register] = Data::array(array_id);
                }
            }
            Instr::CallLibFunc(LibFunc::Range, max, dest) => {
                let min = args.pop().map_or(0, |reg_id| regs[reg_id].as_int());
                let max = regs[max].as_int();
                let output_array_id =
                    alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                let range_arr = obj_pool.get_mut(output_array_id as usize);
                range_arr.extend((min..max).map(Data::from));
                regs[dest] = Data::array(output_array_id);
            }
            Instr::CallLibFunc(LibFunc::JoinStringArray, tgt, dest) => {
                let temp_separator: Option<Data> = args.pop().map(|arg| regs[arg]);
                let separator = temp_separator.as_ref().map_or("", |d| d.as_str(str_pool));
                let array = &obj_pool[regs[tgt].as_array()];
                let total_len: usize = array
                    .iter()
                    .map(|x| x.as_str(str_pool).len())
                    .sum::<usize>()
                    + separator
                        .len()
                        .saturating_mul(array.len().saturating_sub(1));
                let mut output = String::with_capacity(total_len);
                for (i, x) in array.iter().enumerate() {
                    if i > 0 {
                        output.push_str(separator);
                    }
                    output.push_str(x.as_str(str_pool));
                }
                regs[dest] = string!(output);
            }
            // -----
            // FILE SYSTEM FUNCTIONS
            // -----
            Instr::CallLibFunc(LibFunc::FsRead, path, dest_reg_id) => {
                regs[dest_reg_id] =
                    string!(match fs::read_to_string(regs[path].as_str(str_pool)) {
                        Ok(p) => p,
                        Err(e) => {
                            cold_path();
                            error_with_catch!(ErrType::from(e.kind()));
                        }
                    });
            }
            Instr::CallLibFunc(LibFunc::FsExists, path, dest_reg_id) => {
                regs[dest_reg_id] = match fs::exists(regs[path].as_str(str_pool)) {
                    Ok(b) => b.into(),
                    Err(e) => {
                        error_with_catch!(ErrType::from(e.kind()));
                    }
                }
            }
            // Overwrites a file, will create it if it doesn't exist
            Instr::CallLibFuncVoid(LibFuncVoid::FsWrite, path, contents) => {
                if let Err(e) =
                    fs::write(regs[path].as_str(str_pool), regs[contents].as_str(str_pool))
                {
                    error_with_catch!(ErrType::from(e.kind()));
                }
            }
            // Appends to a file, will create if it doesn't exist
            Instr::CallLibFuncVoid(LibFuncVoid::FsAppend, path, contents) => {
                match fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(regs[path].as_str(str_pool))
                {
                    Ok(mut f) => {
                        if let Err(e) = f.write_all(regs[contents].as_str(str_pool).as_bytes()) {
                            error_with_catch!(ErrType::from(e.kind()));
                        }
                    }
                    Err(e) => {
                        error_with_catch!(ErrType::from(e.kind()));
                    }
                }
            }
            // Deletes the file located at `path`, throwing an error if it doesn't exist.
            Instr::CallLibFuncVoid(LibFuncVoid::FsDelete, path, _) => {
                if let Err(e) = fs::remove_file(regs[path].as_str(str_pool)) {
                    error_with_catch!(ErrType::from(e.kind()));
                }
            }
            // Deletes the empty directory located at `path`
            Instr::CallLibFuncVoid(LibFuncVoid::FsDeleteDir, path, _) => {
                if let Err(e) = fs::remove_dir(regs[path].as_str(str_pool)) {
                    error_with_catch!(ErrType::from(e.kind()));
                }
            }
            #[cfg(target_arch = "wasm32")]
            Instr::CallLibFunc(LibFunc::Argv, _, dest) => {
                regs[dest] = Data::array(alloc_array(
                    obj_pool,
                    map_pool,
                    str_pool,
                    r,
                    recursion_stack,
                    gc,
                ))
            }
            #[cfg(not(target_arch = "wasm32"))]
            Instr::CallLibFunc(LibFunc::Argv, _, dest) => {
                // The strings are rooted on the recursion stack while the rest
                // are made, since each one's allocation can run the collector,
                // and the list is made last for the same reason.
                let base = recursion_stack.len();
                for arg in std::env::args().skip(crate::rt::argv_skip()) {
                    let s = string!(arg);
                    recursion_stack.0.push(s);
                }
                let array_id = alloc_array(obj_pool, map_pool, str_pool, r, recursion_stack, gc);
                let list = obj_pool.get_mut(array_id as usize);
                list.extend(recursion_stack.0.drain(base..));
                regs[dest] = Data::array(array_id);
            }
            Instr::CallLibFuncVoid(LibFuncVoid::Sort, tgt, _) => {
                let id = regs[tgt].as_array();
                if gc.marking() {
                    rescan_list(gc, id);
                }
                let array = obj_pool.get_mut(id);
                if !array.is_empty() {
                    if array[0].is_int() {
                        array.sort_unstable_by_key(|x| x.as_int());
                    } else if array[0].is_float() {
                        array.sort_unstable_by(|a, b| {
                            a.as_float()
                                .partial_cmp(&b.as_float())
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                    } else if array[0].is_string() {
                        array.sort_unstable_by(|a, b| a.as_str(str_pool).cmp(b.as_str(str_pool)));
                    }
                }
            }
            Instr::StartErrorCatch(jmp_size, err_reg_id) => {
                error_handles.push(ErrorCatch {
                    catch_loc: (i as u32) + (jmp_size as u32),
                    error_reg: err_reg_id,
                    call_frames_len: call_frames.len() as u32,
                    args_len: args.len() as u32,
                    recursion_len: recursion_stack.len() as u32,
                });
            }
            Instr::StopErrorCatch => unsafe {
                error_handles.pop_unchecked();
            },
            Instr::ThrowError(error_reg_id) => {
                error_with_catch!(ErrType::Custom(regs[error_reg_id].as_str(str_pool)));
            }
            Instr::Halt(code) => {
                cold_path();

                #[cfg(not(target_arch = "wasm32"))]
                if code != 0 {
                    // An exit status is a C `int`, so a wider code arrives at
                    // the operating system as its low 32 bits.
                    std::process::exit(regs[code].as_int() as i32);
                }

                i = HALTED;
                break;
            }
            // The dispatch loop runs every other instruction itself.
            _ => unreachable!(),
        }
        i += 1;
    }
    i
}
