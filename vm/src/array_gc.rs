// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::data::Data;
use crate::vm::{GcScratch, MapPool, ObjectPool, RegisterFile};

/// The fewest slots a pool holds before its first collection, and the least a
/// collection sets the next one's threshold to.
pub const MIN_GC_THRESHOLD: u32 = 256;

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
    pub fn push_unmarked(&self, len: usize, out: &mut Vec<u32>) {
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
pub fn next_threshold(pool_len: usize, freed: usize) -> u32 {
    let live = pool_len - freed;
    (live.saturating_mul(2).min(u32::MAX as usize) as u32).max(MIN_GC_THRESHOLD)
}

/// Allocates a new array in the array pool. If reusing an array, it clears it.
pub fn alloc_array(
    obj_pool: &mut ObjectPool,
    map_pool: &MapPool,
    free_arrays: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc_array_threshold: &mut u32,
    gc: &mut GcScratch,
) -> u32 {
    if let Some(id) = free_arrays.pop() {
        obj_pool[id as usize].clear();
        return id;
    }
    if obj_pool.len() >= (*gc_array_threshold as usize) {
        array_gc(
            obj_pool,
            map_pool,
            free_arrays,
            registers,
            recursion_stack,
            gc,
        );
        *gc_array_threshold = next_threshold(obj_pool.len(), free_arrays.len());
        if let Some(id) = free_arrays.pop() {
            obj_pool[id as usize].clear();
            return id;
        }
    }
    let id = obj_pool.len() as u32;
    obj_pool.push(Vec::new());
    id
}

/// The most elements a freed array keeps room for. A freed slot holds on to
/// its buffer so the next array written there, usually a struct or a short
/// list, needs no allocation; a buffer larger than this goes back to the
/// allocator instead of sitting in a dead slot until something reuses it.
const KEEP_FREED_CAPACITY: usize = 64;

/// Rebuilds the array free list from a fresh trace. It runs only once the free
/// list is empty, so every slot the trace leaves unmarked is garbage.
fn array_gc(
    obj_pool: &mut ObjectPool,
    map_pool: &MapPool,
    free_arrays: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcScratch,
) {
    reset_marks(gc, obj_pool.len(), map_pool.len());
    mark::<false>(registers, recursion_stack, obj_pool, map_pool, gc);
    free_arrays.clear();
    gc.array_live.push_unmarked(obj_pool.len(), free_arrays);
    for &id in free_arrays.iter() {
        let slot = &mut obj_pool[id as usize];
        if slot.capacity() > KEEP_FREED_CAPACITY {
            *slot = Vec::new();
        }
    }
}

/// Clears the object marks and gives each pool a mark per entry.
///
/// Both pools are prepared whichever one is about to be swept: the trace
/// crosses between them, so it writes a mark for every array and every map it
/// reaches. Clearing matters as much as sizing, since a mark left over from an
/// earlier collection reads as "already visited" and stops the trace short of
/// objects that are still live.
pub fn reset_marks(gc: &mut GcScratch, arrays: usize, maps: usize) {
    gc.collections += 1;
    gc.array_live.reset(arrays);
    gc.map_live.reset(maps);
}

/// Marks everything the registers can still reach: every array (which covers
/// structs, enums and function values) and every map, and with `STRINGS` every
/// pooled string too, whose marks the caller has sized.
///
/// The root set is the same whichever pool is about to be swept, because the
/// trace crosses between them: an array reachable only through a map in a
/// register is live, and so is a map reachable only through an array. Every
/// value is tested on its own, never read as a verdict on its neighbours: a
/// list or map typed `any`, which is what a parsed json document is made of,
/// mixes scalars and objects freely.
pub fn mark<const STRINGS: bool>(
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    obj_pool: &ObjectPool,
    map_pool: &MapPool,
    gc: &mut GcScratch,
) {
    let GcScratch {
        array_live,
        map_live,
        string_live,
        work,
        ..
    } = gc;
    for &d in registers.0.iter().chain(recursion_stack.0.iter()) {
        push_heap::<STRINGS>(d, string_live, work);
    }
    while let Some(d) = work.pop() {
        if d.is_map() {
            if map_live.insert(d.as_map()) {
                continue;
            }
            for (k, v) in &map_pool[d.as_map()] {
                push_heap::<STRINGS>(*k, string_live, work);
                push_heap::<STRINGS>(*v, string_live, work);
            }
        } else {
            if array_live.insert(d.as_array()) {
                continue;
            }
            for &e in &obj_pool[d.as_array()] {
                push_heap::<STRINGS>(e, string_live, work);
            }
        }
    }
}

/// Queues `d` for the trace when it refers to a pool object, and marks it
/// straight away when it is a pooled string, which refers to nothing further.
#[inline(always)]
fn push_heap<const STRINGS: bool>(d: Data, string_live: &mut MarkBits, work: &mut Vec<Data>) {
    if d.is_array() || d.is_struct() || d.is_enum() || d.is_map() {
        work.push(d);
    } else if STRINGS && d.is_large_str() {
        string_live.insert(d.get_str_pool_id());
    }
}
