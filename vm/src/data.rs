// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::gc::GcState;
use crate::gc::alloc_string;
use crate::rt::EnumType;
use crate::rt::Struct;
use crate::vm::{MapPool, ObjectPool, RegisterFile, StringPool, char_byte_offset, count_chars};
use smol_strc::SmolStr;
use smol_strc::ToSmolStr;
use std::hash::Hash;
use std::hash::Hasher;
use std::hint::cold_path;
use std::hint::unreachable_unchecked;

const NAN_BASE: u64 =
    0b1111_1111_1111_1000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000;
const PAYLOAD_MASK: u64 = 0b1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111;
const NAN_BOOL: u64 = NAN_BASE | (1 << 48);
const NAN_STRING_SMALL: u64 = NAN_BASE | (2 << 48);
const NAN_STRING_LARGE: u64 = NAN_BASE | (3 << 48);
const NAN_ARRAY: u64 = NAN_BASE | (4 << 48);
const NAN_NULL: u64 = NAN_BASE | (5 << 48);
const NAN_INT: u64 = NAN_BASE | (6 << 48);
const NAN_STRUCT: u64 = NAN_BASE | (7 << 48);
const NAN_MAP: u64 = NAN_BASE | (7 << 48) | (1 << 47);
/// Enum values claim the free all-zero type field on the quiet-NaN base
/// (`NAN_ENUM == NAN_BASE`). The 48-bit payload packs the enum type id
/// (bits 32-47) and an object-pool index (low 32), exactly like the struct
/// encoding.
///
/// Hardware produces a quiet NaN with the sign bit set for an invalid operation
/// such as `sqrt` of a negative number, and its bit pattern is exactly
/// `NAN_BASE`: an enum of type id 0 at pool index 0. [`Data::float`] therefore
/// rewrites any negative quiet NaN to [`CANONICAL_NAN`], which keeps every
/// float outside the tag space.
const NAN_ENUM: u64 = NAN_BASE;
/// The positive quiet NaN every NaN float is stored as. Its sign bit is clear,
/// so it reads back as a float rather than as an enum. NaN carries no value a
/// candela program can inspect, so collapsing sign and payload changes nothing
/// observable.
const CANONICAL_NAN: u64 =
    0b0111_1111_1111_1000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000;
/// The top bit of each of the six bytes an inlined string packs. A string
/// with none of them set holds nothing but single-byte characters.
const SMALL_STR_HIGH_BITS: u64 = 0x0000_8080_8080_8080;
/// The bit that tells a function value from the list it is built as. A
/// function value is an array: its slots are where the body starts and the
/// cells it captured, and the object pool holds it and the collector walks it
/// like any other. The bit is free in an array's payload, since a pool index
/// spans the low 32, so carrying it costs the value nothing and costs a
/// program that never makes one nothing at all.
const NAN_FN_MARK: u64 = 1 << 47;
/// How a function value reads wherever one becomes text. A function carries no
/// signature at run time, so every function reads alike.
pub const FUNCTION_TEXT: &str = "<fn>";
pub const NULL: Data = Data::tagged(NAN_NULL);
pub const FALSE: Data = Data::tagged(NAN_BOOL);
pub const TRUE: Data = Data::tagged(NAN_BOOL | 1);

/// A NaN-boxed word, plus the word an `int` needs.
/// ### Layout:
/// - first 13 bits are NaN
/// - remaining 51 bits: 3 bits for type (4 for the MAP type) & 48 bits of actual payload (47 for the MAP type)
///
/// ### Data type:
/// - BOOL = 001
/// - STRING_SMALL = 010
/// - STRING_LARGE = 011
/// - ARRAY = 100
/// - NULL = 101
/// - INT = 110
/// - STRUCT = 111
/// - MAP = 1111
///
/// ### Functions
/// A function value is an ARRAY with the top payload bit set, which is what
/// tells it from the list it is built as.
///
/// ### Strings
/// Strings are inlined if they're 6 bytes long or less.
/// If they're longer, they're stored in `string_pool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Data {
    /// The tag and, for every type but `int`, the payload. It comes first so
    /// an inlined string still reads out of the low bytes of the value.
    boxed: u64,
    /// An `int`, and zero for every other type. An `int` spans all 64 bits, so
    /// the tagged word has no room left to hold one.
    int: i64,
}

#[derive(Default, Clone, Copy)]
pub struct DataHash(u64);

impl Hasher for DataHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _bytes: &[u8]) {
        unsafe { unreachable_unchecked() }
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }
}

/// The hash a pooled string is filed under as a map key: 64-bit FNV-1a over
/// its bytes. A map key keeps it in the value, and a `.cdlb` records the key
/// with it, so it has to come out the same on every machine and in every
/// build.
#[must_use]
pub fn text_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl Hash for Data {
    /// Both words in one, so an `int` hashes by its value and every other type
    /// hashes by its box exactly as it did when the box was the whole value,
    /// except a pooled string stored as a map key, which hashes by its text
    /// (see [`Data::as_map_key`]). `DataHash` keeps only the last word written,
    /// so writing them apart would collapse every non-`int` onto the same
    /// hash.
    #[inline(always)]
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.boxed ^ (self.int as u64));
    }
}

pub trait PoolString {
    fn pool_as_str(&self) -> &str;
    fn move_to_slot(self, slot: &mut String);
    fn push_to_pool(self, str_pool: &mut StringPool);
    fn str_len(&self) -> usize;
}

impl PoolString for String {
    #[inline(always)]
    fn pool_as_str(&self) -> &str {
        self.as_str()
    }
    #[inline(always)]
    fn move_to_slot(self, slot: &mut String) {
        *slot = self;
    }
    #[inline(always)]
    fn push_to_pool(self, pool: &mut StringPool) {
        pool.push(self);
    }
    #[inline(always)]
    fn str_len(&self) -> usize {
        self.len()
    }
}
impl PoolString for &str {
    #[inline(always)]
    fn pool_as_str(&self) -> &str {
        self
    }
    #[inline(always)]
    fn move_to_slot(self, slot: &mut String) {
        self.clone_into(slot);
    }
    #[inline(always)]
    fn push_to_pool(self, string_pool: &mut StringPool) {
        string_pool.push(self.to_owned());
    }
    #[inline(always)]
    fn str_len(&self) -> usize {
        self.len()
    }
}

impl Data {
    /// A value whose whole content is its NaN box. Every type but `int` is
    /// built through here, which is what keeps the second word zero for them
    /// and so keeps equality and hashing reading one word.
    #[inline(always)]
    const fn tagged(boxed: u64) -> Self {
        Self { boxed, int: 0 }
    }
    /// The pair of words a `.cdlb` records a value as. The artifact keeps the
    /// box and the integer rather than a type tag, so a value comes back
    /// without being inspected.
    #[must_use]
    #[inline(always)]
    pub const fn into_words(self) -> (u64, i64) {
        (self.boxed, self.int)
    }
    /// Reads the value at `src` as two separate words.
    ///
    /// Most instructions write a result a word at a time: the tag and the
    /// integer are stored apart. A copy that then read the value back as one
    /// 16-byte load would have to wait for both stores to reach the cache,
    /// because the processor forwards a pending store only to a load that
    /// fits inside it. Reading the words apart lets each one forward. The
    /// reads are volatile so the compiler cannot fuse them back into one.
    ///
    /// # Safety
    ///
    /// `src` must point to a live value.
    #[inline(always)]
    pub unsafe fn load(src: *const Self) -> Self {
        let words = src.cast::<u64>();
        unsafe {
            Self {
                boxed: words.read_volatile(),
                int: words.add(1).read_volatile() as i64,
            }
        }
    }
    /// Whether this is `false`. A bool keeps its whole value in the box, so
    /// the box alone answers, and the integer word is never read.
    #[inline(always)]
    pub const fn is_false(self) -> bool {
        self.boxed == NAN_BOOL
    }
    /// Whether this is `true`, read from the box alone.
    #[inline(always)]
    pub const fn is_true(self) -> bool {
        self.boxed == NAN_BOOL | 1
    }
    /// Rebuilds a value from the words [`Data::into_words`] handed out.
    #[must_use]
    #[inline(always)]
    pub const fn from_words(boxed: u64, int: i64) -> Self {
        Self { boxed, int }
    }
    #[inline(always)]
    pub const fn tag(self) -> u64 {
        self.boxed & !0xFFFF_FFFF // also includes type_id
    }
    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.boxed == NAN_NULL
    }
    #[inline(always)]
    pub const fn bool(b: bool) -> Self {
        Self::tagged(NAN_BOOL | b as u64)
    }
    #[inline(always)]
    pub fn as_bool(self) -> bool {
        debug_assert!(self.is_bool());
        (self.boxed & 1) != 0
    }
    #[inline(always)]
    pub const fn is_bool(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_BOOL
    }
    #[inline(always)]
    pub const fn float(n: f64) -> Self {
        let bits = n.to_bits();
        // Every float except a negative quiet NaN leaves at least one bit of
        // NAN_BASE clear and so cannot be mistaken for a tagged value. A
        // negative quiet NaN is stored as the positive one instead.
        if (bits & NAN_BASE) == NAN_BASE {
            Self::tagged(CANONICAL_NAN)
        } else {
            Self::tagged(bits)
        }
    }
    #[inline(always)]
    pub const fn as_float(self) -> f64 {
        debug_assert!(self.is_float());
        f64::from_bits(self.boxed)
    }
    #[inline(always)]
    pub const fn is_float(self) -> bool {
        (self.boxed & NAN_BASE) != NAN_BASE
    }
    #[inline(always)]
    /// Convert the given integer to a tagged integer.
    /// The box carries the tag and the second word carries all 64 bits.
    pub const fn int(n: i64) -> Self {
        Self {
            boxed: NAN_INT,
            int: n,
        }
    }
    #[inline(always)]
    pub const fn as_int(self) -> i64 {
        debug_assert!(self.is_int());
        self.int
    }
    #[inline(always)]
    pub const fn is_int(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_INT
    }
    #[inline(always)]
    pub const fn array(id: u32) -> Self {
        Self::tagged(NAN_ARRAY | id as u64)
    }
    #[inline(always)]
    pub const fn as_array(self) -> usize {
        // Enum values share the object pool with arrays/structs and are compared
        // through this accessor in `obj_eq`.
        debug_assert!(self.is_array() || self.is_struct() || self.is_enum());
        (self.boxed & 0xFFFF_FFFF) as usize
    }
    #[inline(always)]
    pub const fn is_array(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_ARRAY
    }
    /// A function value over the pool entry `id`: the array a call reads the
    /// body location and the captured cells out of, marked as the function it
    /// is.
    #[inline(always)]
    pub const fn function(id: u32) -> Self {
        Self::tagged(NAN_ARRAY | NAN_FN_MARK | id as u64)
    }
    /// Whether this value is a function. A function value answers
    /// [`Self::is_array`] too, since everything that holds, walks and frees an
    /// array holds, walks and frees this one.
    #[inline(always)]
    pub const fn is_function(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_ARRAY && (self.boxed & NAN_FN_MARK) != 0
    }
    /// This will create a new inlined string.
    /// In debug it'll panic (just in case).
    /// The caller guarantees that `s` is never longer than 6 bytes.
    #[inline(always)]
    pub fn small_str(s: &str) -> Self {
        debug_assert!(s.len() <= 6);
        let mut payload: u64 = 0;
        // Packs 6 bytes into the payload, filling up the 48 payload bits
        for (i, byte) in s.as_bytes().iter().enumerate() {
            payload |= (*byte as u64) << (i * 8);
        }
        Self::tagged(NAN_STRING_SMALL | (payload & PAYLOAD_MASK))
    }
    #[inline(always)]
    pub const fn large_str_id(id: u64) -> Self {
        Self::tagged(NAN_STRING_LARGE | id)
    }
    /// This value as a map stores it for a key. A string in the pool takes a
    /// hash of its text into its second word, which only an `int` otherwise
    /// uses, arranged so the value hashes as the text does rather than as the
    /// slot: two strings with the same text in different slots then land in
    /// the same bucket, where the map compares them by text. See
    /// [`crate::vm::KeyByText`].
    #[inline(always)]
    #[must_use]
    pub fn as_map_key(self, string_pool: &StringPool) -> Self {
        if self.is_large_str() {
            Self {
                boxed: self.boxed,
                int: (text_hash(self.as_str(string_pool)) ^ self.boxed) as i64,
            }
        } else {
            self
        }
    }
    /// Same as str(), except this never runs the GC because this function is called by the compiler
    #[inline(always)]
    pub fn p_str(s: &str, string_pool: &mut StringPool) -> Self {
        Self::interned(s, string_pool).unwrap_or_else(|| {
            let string_pool_id = string_pool.len() as u64;
            string_pool.push(s.to_owned());
            Self::tagged(NAN_STRING_LARGE | string_pool_id)
        })
    }
    /// `s` as a value without allocating: inline if it is six bytes or
    /// shorter, else the pool slot already holding it, if one does.
    pub fn interned(s: &str, string_pool: &mut StringPool) -> Option<Self> {
        if s.len() <= 6 {
            return Some(Self::small_str(s));
        }
        let id = string_pool.iter().position(|existing| existing == s)?;
        // The program may hold nothing that reaches this slot, so a cycle in
        // progress counts it as reached from here on.
        string_pool.shade(id);
        Some(Self::tagged(NAN_STRING_LARGE | id as u64))
    }
    /// Allocates a string, storing it directly inside the value if it is six
    /// bytes or shorter and in the string pool otherwise. A pooled string can
    /// start a collection, so every value the program still holds has to be
    /// reachable from `registers` or `recursion_stack`.
    #[inline(always)]
    pub fn string<S: PoolString>(
        s: S,
        array_pool: &mut ObjectPool,
        map_pool: &mut MapPool,
        string_pool: &mut StringPool,
        registers: &RegisterFile,
        recursion_stack: &RegisterFile,
        gc: &mut GcState,
    ) -> Self {
        if s.str_len() <= 6 {
            Self::small_str(s.pool_as_str())
        } else {
            let id = alloc_string(
                s,
                array_pool,
                map_pool,
                string_pool,
                registers,
                recursion_stack,
                gc,
            );
            Self::tagged(NAN_STRING_LARGE | id)
        }
    }
    #[inline(always)]
    pub fn as_str(&self, string_pool: &StringPool) -> &str {
        debug_assert!(self.is_string());
        if (self.boxed & !PAYLOAD_MASK) == NAN_STRING_SMALL {
            let len = self.small_str_len();
            let ptr = self as *const Self as *const u8;
            unsafe {
                let slice = std::slice::from_raw_parts(ptr, len);
                std::str::from_utf8_unchecked(slice)
            }
        } else {
            let payload = (self.boxed & PAYLOAD_MASK) as usize;
            unsafe { &*(string_pool[payload].as_str() as *const str) }
        }
    }
    /// True when every character of this string takes a single byte. A
    /// character position on such a string is a byte position, which is the
    /// route indexing and slicing take when they can.
    #[inline(always)]
    pub fn str_is_ascii(&self, string_pool: &StringPool) -> bool {
        if self.is_large_str() {
            string_pool.is_ascii(self.get_str_pool_id())
        } else {
            self.boxed & SMALL_STR_HIGH_BITS == 0
        }
    }
    /// How many characters this string has. Positions on a string count
    /// characters, so this is what `len` answers and what an index or a slice
    /// bound is checked against.
    #[inline(always)]
    pub fn str_char_len(&self, string_pool: &StringPool) -> usize {
        if self.is_large_str() {
            string_pool.char_len(self.get_str_pool_id())
        } else if self.boxed & SMALL_STR_HIGH_BITS == 0 {
            self.small_str_len()
        } else {
            cold_path();
            count_chars(self.as_str(string_pool))
        }
    }
    /// The byte offset character `char_idx` starts at. The string has at least
    /// `char_idx` characters; at exactly that many the answer is its byte
    /// length, which is what a slice bound on the end of the string asks for.
    #[inline(always)]
    pub fn str_byte_offset(&self, char_idx: usize, string_pool: &StringPool) -> usize {
        if self.str_is_ascii(string_pool) {
            char_idx
        } else if self.is_large_str() {
            string_pool.byte_offset(self.get_str_pool_id(), char_idx)
        } else {
            char_byte_offset(self.as_str(string_pool), char_idx)
        }
    }
    /// How many bytes an inlined string takes.
    #[inline(always)]
    const fn small_str_len(&self) -> usize {
        let payload = self.boxed & PAYLOAD_MASK;
        ((64 - payload.leading_zeros()) as usize + 7) >> 3
    }
    #[inline(always)]
    pub const fn is_string(self) -> bool {
        // this works because NAN_TAG_STRING_LARGE == NAN_TAG_STRING_SMALL + (1 << 48)
        (self.boxed & !PAYLOAD_MASK).wrapping_sub(NAN_STRING_SMALL) <= const { 1u64 << 48 }
    }
    /// Increments the integer stored in this Data in-place. Wraps.
    #[inline(always)]
    pub const fn inc_int(&mut self) {
        debug_assert!(self.is_int());
        self.int = self.int.wrapping_add(1);
    }
    /// Decrements the integer stored in this Data in-place. Wraps.
    #[inline(always)]
    pub const fn dec_int(&mut self) {
        debug_assert!(self.is_int());
        self.int = self.int.wrapping_sub(1);
    }
    /// Writes src + 1 into self. Wraps.
    #[inline(always)]
    pub const fn inc_into(&mut self, src: Self) {
        debug_assert!(src.is_int());
        self.boxed = NAN_INT;
        self.int = src.int.wrapping_add(1);
    }
    /// Writes src - 1 into self. Wraps.
    #[inline(always)]
    pub const fn dec_into(&mut self, src: Self) {
        debug_assert!(src.is_int());
        self.boxed = NAN_INT;
        self.int = src.int.wrapping_sub(1);
    }
    #[inline(always)]
    pub const fn is_large_str(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_STRING_LARGE
    }
    #[inline(always)]
    pub const fn get_str_pool_id(self) -> usize {
        debug_assert!(self.is_large_str());
        (self.boxed & PAYLOAD_MASK) as usize
    }
    #[inline(always)]
    pub const fn struct_instance(type_id: u16, id: u32) -> Self {
        Self::tagged(NAN_STRUCT | ((type_id as u64) << 32) | id as u64)
    }
    #[inline(always)]
    pub const fn as_struct(self) -> usize {
        // Enum values reuse the struct field-access instructions
        // (`GetFieldStruct`/`SetFieldStruct`), which extract the low-32 object
        // index regardless of the box tag, so an enum value is accepted here.
        debug_assert!(self.is_struct() || self.is_enum());
        (self.boxed & 0xFFFF_FFFF) as usize
    }
    #[inline(always)]
    pub const fn enum_instance(type_id: u16, id: u32) -> Self {
        Self::tagged(NAN_ENUM | ((type_id as u64) << 32) | id as u64)
    }
    #[inline(always)]
    pub const fn as_enum(self) -> usize {
        debug_assert!(self.is_enum());
        (self.boxed & 0xFFFF_FFFF) as usize
    }
    #[inline(always)]
    pub const fn enum_type_id(self) -> u16 {
        debug_assert!(self.is_enum());
        ((self.boxed >> 32) & 0xFFFF) as u16
    }
    #[inline(always)]
    pub const fn is_enum(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_ENUM
    }
    #[inline(always)]
    pub const fn struct_type_id(self) -> u16 {
        ((self.boxed >> 32) & 0xFFFF) as u16
    }
    #[inline(always)]
    pub const fn is_struct(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_STRUCT && (self.boxed & (1 << 47)) == 0
    }
    #[inline(always)]
    pub const fn map(id: u32) -> Self {
        Self::tagged(NAN_MAP | id as u64)
    }
    #[inline(always)]
    pub const fn as_map(self) -> usize {
        debug_assert!(self.is_map());
        (self.boxed & 0xFFFF_FFFF) as usize
    }
    #[inline(always)]
    pub const fn is_map(self) -> bool {
        (self.boxed & !PAYLOAD_MASK) == NAN_STRUCT && (self.boxed & (1 << 47)) != 0
    }
    /// A short runtime type name, used in downcast error messages. Reads only
    /// the box tag, so it needs no pools.
    #[must_use]
    pub const fn type_name(self) -> &'static str {
        if self.is_int() {
            "int"
        } else if self.is_float() {
            "float"
        } else if self.is_bool() {
            "bool"
        } else if self.is_null() {
            "null"
        } else if self.is_string() {
            "string"
        } else if self.is_array() {
            "list"
        } else if self.is_map() {
            "map"
        } else if self.is_struct() {
            "struct"
        } else if self.is_enum() {
            "enum"
        } else {
            "value"
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn format(
        self,
        obj_pool: &ObjectPool,
        string_pool: &StringPool,
        map_pool: &MapPool,
        structs: &[Struct],
        enums: &[EnumType],
        show_str: bool,
    ) -> SmolStr {
        if self.is_float() {
            format_float(self.as_float())
        } else if self.is_int() {
            self.as_int().to_smolstr()
        } else if self.is_bool() {
            self.as_bool().to_smolstr()
        } else if self.is_string() {
            if show_str {
                self.as_str(string_pool).to_smolstr()
            } else {
                format_args!("\"{}\"", self.as_str(string_pool)).to_smolstr()
            }
        } else if self.is_function() {
            SmolStr::new_static(FUNCTION_TEXT)
        } else if self.is_array() {
            format_args!(
                "[{}]",
                obj_pool[self.as_array()]
                    .iter()
                    .map(|x| x.format(obj_pool, string_pool, map_pool, structs, enums, false))
                    .collect::<Vec<SmolStr>>()
                    .join(",")
            )
            .to_smolstr()
        } else if self.is_null() {
            SmolStr::new_static("null")
        } else if self.is_struct() {
            let declared = structs.get(self.struct_type_id() as usize);
            let s_name = declared.map_or("struct", |s| s.name.as_str());
            format_args!(
                "{} {{{}}}",
                s_name,
                obj_pool[self.as_struct()]
                    .iter()
                    .enumerate()
                    .map(|(idx, x)| {
                        let value =
                            x.format(obj_pool, string_pool, map_pool, structs, enums, false);
                        // A slot the declaration does not name belongs to a
                        // pool entry read as a struct it is not, and has no
                        // name to give. The value it holds still says what is
                        // there.
                        match declared.and_then(|s| s.fields.get(idx)) {
                            Some((field, ..)) => format_args!("{field}:{value}").to_smolstr(),
                            None => value,
                        }
                    })
                    .collect::<Vec<SmolStr>>()
                    .join(",")
            )
            .to_smolstr()
        } else if self.is_enum() {
            let entry = &obj_pool[self.as_enum()];
            // The variant tag is the entry's first word. Look the variant up
            // instead of indexing on trust: a word that is no tag, which is
            // what an entry a register only aliases can hold, would otherwise
            // index the variant table out of range. An entry carrying no
            // readable tag has no variant name to print.
            let variant = enums
                .get(self.enum_type_id() as usize)
                .filter(|_| entry.first().is_some_and(|word| word.is_int()))
                .and_then(|e| e.variants.get(entry[0].as_int() as usize));
            let Some(variant) = variant else {
                return SmolStr::new_static("enum");
            };
            if entry.len() <= 1 {
                variant.name.clone()
            } else {
                format_args!(
                    "{}({})",
                    variant.name,
                    entry[1..]
                        .iter()
                        .map(|x| x.format(obj_pool, string_pool, map_pool, structs, enums, false))
                        .collect::<Vec<SmolStr>>()
                        .join(",")
                )
                .to_smolstr()
            }
        } else if self.is_map() {
            let m = &map_pool[self.as_map()];
            format_args!(
                "{{{}}}",
                m.iter()
                    .map(|(key, val)| {
                        format_args!(
                            "{}:{}",
                            key.format(obj_pool, string_pool, map_pool, structs, enums, false),
                            val.format(obj_pool, string_pool, map_pool, structs, enums, false),
                        )
                        .to_smolstr()
                    })
                    .collect::<Vec<SmolStr>>()
                    .join(",")
            )
            .to_smolstr()
        } else {
            unsafe { unreachable_unchecked() }
        }
    }
}

/// How candela writes a float: with a decimal point or an exponent, so a whole
/// float reads as a float rather than as an int. An infinity and a
/// not-a-number have no such form and print as themselves.
///
/// Every float goes through here on its way to text: `str`, `print`, and a
/// float inside a printed array, map, struct or enum.
#[must_use]
pub fn format_float(value: f64) -> SmolStr {
    SmolStr::from(zmij::Buffer::new().format(value))
}

impl From<f64> for Data {
    #[inline(always)]
    fn from(value: f64) -> Self {
        Self::float(value)
    }
}
impl From<Data> for f64 {
    #[inline(always)]
    fn from(value: Data) -> Self {
        value.as_float()
    }
}

impl From<i64> for Data {
    #[inline(always)]
    fn from(value: i64) -> Self {
        Self::int(value)
    }
}
impl From<Data> for i64 {
    #[inline(always)]
    fn from(value: Data) -> Self {
        value.as_int()
    }
}

impl From<bool> for Data {
    #[inline(always)]
    fn from(value: bool) -> Self {
        Self::tagged(NAN_BOOL | (value as u64))
    }
}
impl From<Data> for bool {
    #[inline(always)]
    fn from(value: Data) -> Self {
        value.as_bool()
    }
}

#[cfg(test)]
mod format_tests {
    use super::Data;
    use crate::rt::{DataType, EnumType, EnumVariant, Span, Struct};
    use crate::vm::{MapPool, ObjectPool, Pool, StringPool};
    use smol_strc::SmolStr;

    fn one_struct() -> Vec<Struct> {
        vec![Struct {
            name: SmolStr::new_static("P"),
            fields: Box::new([
                (
                    SmolStr::new_static("x"),
                    DataType::Int,
                    Span { start: 0, end: 0 },
                ),
                (
                    SmolStr::new_static("y"),
                    DataType::Int,
                    Span { start: 0, end: 0 },
                ),
            ]),
            id: 0,
            name_span: Span { start: 0, end: 0 },
        }]
    }

    /// A struct names its fields wherever it becomes text, nested in a list as
    /// much as on its own, so `str` and a printed collection read the way
    /// `print` does.
    #[test]
    fn a_struct_formats_with_its_field_names() {
        let structs = one_struct();
        let obj_pool: ObjectPool = Pool(vec![
            vec![Data::int(1), Data::int(2)],
            vec![Data::struct_instance(0, 0)],
        ]);
        let rendered = |value: Data| {
            value.format(
                &obj_pool,
                &StringPool::default(),
                &Pool(Vec::new()),
                &structs,
                &[],
                true,
            )
        };
        assert_eq!(rendered(Data::struct_instance(0, 0)), "P {x:1,y:2}");
        assert_eq!(rendered(Data::array(1)), "[P {x:1,y:2}]");
    }

    /// A pool entry read as a struct the table does not declare has no names to
    /// print, and still says what it holds instead of indexing the field list
    /// with a slot that is not there.
    #[test]
    fn a_struct_with_no_declaration_formats_as_its_slots() {
        let obj_pool: ObjectPool = Pool(vec![vec![Data::int(1), Data::int(2)]]);
        let rendered = Data::struct_instance(0, 0).format(
            &obj_pool,
            &StringPool::default(),
            &Pool(Vec::new()),
            &[],
            &[],
            true,
        );
        assert_eq!(rendered, "struct {1,2}");
    }

    /// A function value reads alike wherever it becomes text, on its own and
    /// inside a list, since a function carries no signature to tell one from
    /// another at run time.
    #[test]
    fn a_function_value_formats_as_the_function_marker() {
        let obj_pool: ObjectPool = Pool(vec![
            vec![Data::int(12)],
            vec![Data::function(0)],
            vec![Data::int(12)],
        ]);
        let rendered = |value: Data| {
            value.format(
                &obj_pool,
                &StringPool::default(),
                &Pool(Vec::new()),
                &[],
                &[],
                true,
            )
        };
        assert_eq!(rendered(Data::function(0)), "<fn>");
        assert_eq!(rendered(Data::array(1)), "[<fn>]");
        // The list the value is built as reads as a list, mark or no mark.
        assert_eq!(rendered(Data::array(2)), "[12]");
    }

    /// Everything that holds, walks and frees an array does the same for a
    /// function value: the mark rides in a bit the pool index leaves free.
    #[test]
    fn a_function_value_is_an_array_underneath() {
        let value = Data::function(7);
        assert!(value.is_array());
        assert!(value.is_function());
        assert_eq!(value.as_array(), 7);
        assert!(!Data::array(7).is_function());
    }

    fn one_enum() -> Vec<EnumType> {
        vec![EnumType {
            name: SmolStr::new_static("Value"),
            variants: Box::new([EnumVariant {
                name: SmolStr::new_static("Int"),
                payload: Box::new([DataType::Int]),
                name_span: Span { start: 0, end: 0 },
            }]),
            id: 0,
            name_span: Span { start: 0, end: 0 },
        }]
    }

    /// An enum value nested in an array formats through the same path a
    /// top-level one does, tag and payload included.
    #[test]
    fn an_enum_inside_an_array_formats_as_its_variant() {
        let enums = one_enum();
        let obj_pool: ObjectPool = Pool(vec![
            vec![Data::int(0), Data::int(3)],
            vec![Data::enum_instance(0, 0)],
        ]);
        let rendered = Data::array(1).format(
            &obj_pool,
            &StringPool::default(),
            &Pool(Vec::new()),
            &[],
            &enums,
            true,
        );
        assert_eq!(rendered, "[Int(3)]");
    }

    /// A pool entry whose first word is no variant tag has no variant to name,
    /// and says so rather than indexing the variant table with whatever the
    /// word happened to be.
    #[test]
    fn an_entry_with_no_readable_tag_formats_as_the_bare_type() {
        let enums = one_enum();
        let string_pool = StringPool::default();
        let map_pool: MapPool = Pool(Vec::new());
        for entry in [
            // An array of enum values, read as if it were an enum value.
            vec![Data::enum_instance(0, 0)],
            // A tag past the last variant.
            vec![Data::int(42)],
            // A negative tag.
            vec![Data::int(-1)],
            // No tag at all.
            Vec::new(),
        ] {
            let obj_pool: ObjectPool = Pool(vec![entry]);
            let rendered = Data::enum_instance(0, 0).format(
                &obj_pool,
                &string_pool,
                &map_pool,
                &[],
                &enums,
                true,
            );
            assert_eq!(rendered, "enum");
        }
    }
}

#[cfg(test)]
mod float_boxing_tests {
    use super::Data;

    /// Hardware answers an invalid operation with a quiet NaN whose sign bit is
    /// set, and that bit pattern is the enum tag. Boxing has to keep it a float,
    /// or reading it back walks the enum table with a type id of 0 and a pool
    /// index of 0.
    #[test]
    fn negative_quiet_nan_stays_a_float() {
        let hardware_nan = f64::from_bits(0xFFF8_0000_0000_0000);
        assert!(hardware_nan.is_nan());

        let boxed = Data::float(hardware_nan);
        assert!(boxed.is_float());
        assert!(!boxed.is_enum());
        assert!(!boxed.is_struct());
        assert!(!boxed.is_map());
        assert!(boxed.as_float().is_nan());
    }

    #[test]
    fn every_nan_boxes_to_one_pattern() {
        let signed = Data::float(f64::from_bits(0xFFF8_0000_0000_0000));
        let unsigned = Data::float(f64::NAN);
        let with_payload = Data::float(f64::from_bits(0xFFF8_0000_DEAD_BEEF));
        assert_eq!(signed, unsigned);
        assert_eq!(signed, with_payload);
    }

    #[test]
    fn infinities_and_ordinary_floats_round_trip() {
        for value in [
            0.0,
            -0.0,
            1.5,
            -1.5,
            f64::MIN,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            let boxed = Data::float(value);
            assert!(boxed.is_float(), "{value} must box as a float");
            assert_eq!(boxed.as_float().to_bits(), value.to_bits());
        }
    }

    /// The tagged types have to stay distinguishable from every float, which is
    /// what makes the NaN check above necessary in the first place.
    #[test]
    fn tagged_values_are_not_floats() {
        assert!(!Data::int(7).is_float());
        assert!(!Data::bool(true).is_float());
        assert!(!Data::array(0).is_float());
        assert!(!Data::map(0).is_float());
        assert!(!Data::enum_instance(0, 0).is_float());
        assert!(!super::NULL.is_float());
    }
}
