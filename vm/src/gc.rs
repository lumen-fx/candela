// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
//! The garbage collector.
//!
//! Every heap value lives in one of three pools: arrays (which also hold
//! structs, enums, function values and closure cells), maps, and strings too
//! long to sit inside a [`Data`]. One collection covers all three, because the
//! trace crosses between them: a list can hold a map, a map can hold a list,
//! and either can hold a pooled string. A slot the trace does not reach goes on
//! its pool's free list, and the next allocation in that pool reuses it.
//!
//! A collection starts when an allocation finds its pool's free list empty and
//! the pool at its threshold.

use crate::data::Data;
use crate::data::PoolString;
use crate::vm::CandelaMap;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::RegisterFile;
use crate::vm::StringPool;

/// The fewest slots a pool holds before its first collection, and the least a
/// collection sets the next one's threshold to.
pub const MIN_GC_THRESHOLD: u32 = 256;

/// The most elements a freed array keeps room for. A freed slot holds on to
/// its buffer so the next array written there, usually a struct or a short
/// list, needs no allocation; a buffer larger than this goes back to the
/// allocator instead of sitting in a dead slot until something reuses it.
const KEEP_FREED_CAPACITY: usize = 64;

/// One mark bit per pool slot.
///
/// A bit per slot rather than a `bool` keeps the marks of a large pool in an
/// eighth of the memory, so clearing them and walking them for the free slots
/// touch an eighth of the cache lines.
#[derive(Default)]
pub struct MarkBits(Vec<u64>);

impl MarkBits {
    /// Clears every mark and sizes the set for `len` slots.
    pub fn reset(&mut self, len: usize) {
        self.0.clear();
        self.0.resize(len.div_ceil(64), 0);
    }
    /// Marks slot `i` and reports whether it was already marked.
    #[inline(always)]
    pub fn insert(&mut self, i: usize) -> bool {
        let word = unsafe { self.0.get_unchecked_mut(i / 64) };
        let bit = 1u64 << (i % 64);
        let was = *word & bit != 0;
        *word |= bit;
        was
    }
    /// Every unmarked slot below `len`, pushed onto `out`.
    fn push_unmarked(&self, len: usize, out: &mut Vec<u32>) {
        for (w, &word) in self.0.iter().enumerate() {
            let mut free = !word;
            while free != 0 {
                let i = w * 64 + free.trailing_zeros() as usize;
                if i >= len {
                    break;
                }
                out.push(i as u32);
                free &= free - 1;
            }
        }
    }
}

/// The threshold the next collection of a pool runs at: twice what survived
/// this one. A pool that doubles its live set between collections pays for
/// each collection with as many fresh allocations as the trace visited, and a
/// pool whose live set stays flat stops growing instead of doubling on every
/// collection whether anything survived or not.
#[inline(always)]
fn next_threshold(pool_len: usize, freed: usize) -> u32 {
    let live = pool_len - freed;
    (live.saturating_mul(2).min(u32::MAX as usize) as u32).max(MIN_GC_THRESHOLD)
}

/// What the collector knows about the pools between two allocations.
///
/// This lives with the pools rather than with one run of the interpreter
/// because the pools do: a host that calls into a resident program many times
/// keeps every object the earlier calls left behind, and a collector that
/// forgot its free lists and started its thresholds over on each call would
/// run a full mark and sweep on the first few allocations of every call once
/// the pools had grown past the starting threshold, and would never reuse a
/// slot the previous call had freed.
pub struct GcState {
    /// Object-pool slots (arrays, structs, enums and function values) a
    /// collection freed.
    pub(crate) free_arrays: Vec<u32>,
    /// Map-pool slots a collection freed.
    pub(crate) free_maps: Vec<u32>,
    /// String-pool slots a collection freed.
    pub(crate) free_strings: Vec<u32>,
    /// Object-pool length at which an array allocation starts a collection.
    array_threshold: u32,
    /// Map-pool length at which a map allocation starts a collection.
    map_threshold: u32,
    /// String-pool length at which a string allocation starts a collection.
    string_threshold: u32,
    array_marks: MarkBits,
    map_marks: MarkBits,
    string_marks: MarkBits,
    /// The objects the trace has reached and not yet walked.
    work: Vec<Data>,
    /// How many collections have run.
    cycles: u64,
}

impl Default for GcState {
    fn default() -> Self {
        Self {
            free_arrays: Vec::new(),
            free_maps: Vec::new(),
            free_strings: Vec::new(),
            array_threshold: MIN_GC_THRESHOLD,
            map_threshold: MIN_GC_THRESHOLD,
            string_threshold: MIN_GC_THRESHOLD,
            array_marks: MarkBits::default(),
            map_marks: MarkBits::default(),
            string_marks: MarkBits::default(),
            work: Vec::new(),
            cycles: 0,
        }
    }
}

impl GcState {
    /// How many collections have run over these pools.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Collects every pool: marks everything the roots reach, then frees every
    /// slot left unmarked.
    #[cold]
    #[inline(never)]
    fn collect(
        &mut self,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        roots: [&[Data]; 2],
    ) {
        self.cycles += 1;
        self.array_marks.reset(objs.len());
        self.map_marks.reset(maps.len());
        self.string_marks.reset(strings.len());
        self.mark(objs, maps, roots);
        self.sweep(objs, maps, strings);
    }

    /// Marks everything the roots can still reach: every array (which covers
    /// structs, enums and function values), every map and every pooled string.
    ///
    /// Every value is tested on its own, never read as a verdict on its
    /// neighbours: a list or map typed `any`, which is what a parsed json
    /// document is made of, mixes scalars and objects freely.
    fn mark(&mut self, objs: &ObjectPool, maps: &MapPool, roots: [&[Data]; 2]) {
        let Self {
            array_marks,
            map_marks,
            string_marks,
            work,
            ..
        } = self;
        for root in roots {
            for &d in root {
                push_heap(d, string_marks, work);
            }
        }
        while let Some(d) = work.pop() {
            if d.is_map() {
                if map_marks.insert(d.as_map()) {
                    continue;
                }
                for (k, v) in &maps[d.as_map()] {
                    push_heap(*k, string_marks, work);
                    push_heap(*v, string_marks, work);
                }
            } else {
                if array_marks.insert(d.as_array()) {
                    continue;
                }
                for &e in &objs[d.as_array()] {
                    push_heap(e, string_marks, work);
                }
            }
        }
    }

    /// Rebuilds every free list from the marks, and sets each pool's next
    /// threshold from what survived.
    fn sweep(&mut self, objs: &mut ObjectPool, maps: &mut MapPool, strings: &mut StringPool) {
        self.free_arrays.clear();
        self.array_marks
            .push_unmarked(objs.len(), &mut self.free_arrays);
        for &id in &self.free_arrays {
            let slot = objs.get_mut(id as usize);
            if slot.capacity() > KEEP_FREED_CAPACITY {
                *slot = Vec::new();
            }
        }
        self.array_threshold = next_threshold(objs.len(), self.free_arrays.len());

        self.free_maps.clear();
        self.map_marks
            .push_unmarked(maps.len(), &mut self.free_maps);
        for &id in &self.free_maps {
            let slot = maps.get_mut(id as usize);
            if slot.capacity() > KEEP_FREED_CAPACITY {
                *slot = CandelaMap::default();
            }
        }
        self.map_threshold = next_threshold(maps.len(), self.free_maps.len());

        self.free_strings.clear();
        self.string_marks
            .push_unmarked(strings.len(), &mut self.free_strings);
        // A freed slot gives up its text. Interning finds a string by
        // comparing text across the whole pool, so a freed slot that kept its
        // text would be handed out as a live string and then overwritten by
        // the next allocation that reuses the slot.
        for &id in &self.free_strings {
            strings.release(id as usize);
        }
        self.string_threshold = next_threshold(strings.len(), self.free_strings.len());
    }
}

/// Queues `d` for the trace when it refers to a pool object, and marks it
/// straight away when it is a pooled string, which refers to nothing further.
#[inline(always)]
fn push_heap(d: Data, string_marks: &mut MarkBits, work: &mut Vec<Data>) {
    if d.is_array() || d.is_struct() || d.is_enum() || d.is_map() {
        work.push(d);
    } else if d.is_large_str() {
        string_marks.insert(d.get_str_pool_id());
    }
}

/// Allocates an empty array in the object pool, reusing a freed slot when
/// there is one.
#[inline(always)]
pub fn alloc_array(
    objs: &mut ObjectPool,
    maps: &mut MapPool,
    strings: &mut StringPool,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcState,
) -> u32 {
    if let Some(id) = gc.free_arrays.pop() {
        objs[id as usize].clear();
        return id;
    }
    if objs.len() >= gc.array_threshold as usize {
        gc.collect(objs, maps, strings, [&registers.0, &recursion_stack.0]);
        if let Some(id) = gc.free_arrays.pop() {
            objs[id as usize].clear();
            return id;
        }
    }
    let id = objs.len() as u32;
    objs.push(Vec::new());
    id
}

/// Allocates an empty map in the map pool, reusing a freed slot when there is
/// one.
#[inline(always)]
pub fn alloc_map(
    objs: &mut ObjectPool,
    maps: &mut MapPool,
    strings: &mut StringPool,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcState,
) -> u32 {
    if let Some(id) = gc.free_maps.pop() {
        maps[id as usize].clear();
        return id;
    }
    if maps.len() >= gc.map_threshold as usize {
        gc.collect(objs, maps, strings, [&registers.0, &recursion_stack.0]);
        if let Some(id) = gc.free_maps.pop() {
            maps[id as usize].clear();
            return id;
        }
    }
    let id = maps.len() as u32;
    maps.push(CandelaMap::default());
    id
}

/// Stores `s` in the string pool, reusing a freed slot when there is one, and
/// answers the slot. The caller has checked that `s` is too long to inline.
#[inline(always)]
pub fn alloc_string<S: PoolString>(
    s: S,
    objs: &mut ObjectPool,
    maps: &mut MapPool,
    strings: &mut StringPool,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcState,
) -> u64 {
    if gc.free_strings.is_empty() && strings.len() >= gc.string_threshold as usize {
        gc.collect(objs, maps, strings, [&registers.0, &recursion_stack.0]);
    }
    if let Some(id) = gc.free_strings.pop() {
        s.move_to_slot(strings.get_mut(id as usize));
        u64::from(id)
    } else {
        let id = strings.len() as u64;
        s.push_to_pool(strings);
        id
    }
}
