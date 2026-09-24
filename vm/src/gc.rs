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
//! A collection is a cycle of three phases, and the collector can stop between
//! any two steps of it and carry on later:
//!
//! - Idle: nothing to do. A cycle begins when an allocation finds its pool's
//!   free list empty and the pool at its threshold.
//! - Mark: beginning a cycle records how long each pool is, marks every slot
//!   already on a free list, and marks what the roots hold. Marking then walks
//!   the objects it has reached, a chunk of at most [`CHUNK`] entries at a time,
//!   until nothing is left to walk.
//! - Sweep: walks each pool's marks up to the length recorded at the start and
//!   puts every unmarked slot on the free list. Then the cycle is over.
//!
//! Work is counted in units: one per chunk walked, one per 64 slots swept and
//! one per slot freed. A step is given a budget in these units and stops once
//! it has spent it.

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

/// The most elements a freed array or map keeps room for. A freed slot holds
/// on to its buffer so the next value written there, usually a struct or a
/// short list, needs no allocation; a buffer larger than this goes back to the
/// allocator instead of sitting in a dead slot until something reuses it.
const KEEP_FREED_CAPACITY: usize = 64;

/// The most entries of one list or map a single unit of marking walks. A large
/// list is walked a chunk at a time, so no single step has to walk all of it.
pub const CHUNK: u32 = 64;

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
    /// The unmarked slots among the 64 that word `w` covers, below `limit`, as
    /// set bits.
    #[inline(always)]
    fn unmarked_in_word(&self, w: usize, limit: usize) -> u64 {
        let free = !unsafe { *self.0.get_unchecked(w) };
        let first = w * 64;
        if limit - first >= 64 {
            free
        } else {
            free & ((1u64 << (limit - first)) - 1)
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

/// An object the trace has reached and not yet walked to its end: the list or
/// map, and the entry the next chunk starts at.
#[derive(Clone, Copy)]
struct Grey {
    id: u32,
    from: u32,
    map: bool,
}

/// Where a cycle stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Mark,
    /// Sweeping the pool named, arrays first, then maps, then strings.
    Sweep(Swept),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Swept {
    Arrays,
    Maps,
    Strings,
}

/// The marks of the two object pools and the objects still to walk.
///
/// Every slot at or past a pool's limit was allocated after the cycle began,
/// and counts as marked without a bit: nothing a cycle allocates is freed by
/// it. The string pool keeps its own marks, see [`StringPool::shade`].
#[derive(Default)]
struct Tracer {
    array_marks: MarkBits,
    map_marks: MarkBits,
    array_limit: u32,
    map_limit: u32,
    work: Vec<Grey>,
}

impl Tracer {
    /// Marks what `d` refers to, queueing a list or map it marks for its
    /// entries to be walked.
    #[inline(always)]
    fn shade(&mut self, strings: &mut StringPool, d: Data) {
        if d.is_map() {
            let id = d.as_map() as u32;
            if id < self.map_limit && !self.map_marks.insert(id as usize) {
                self.work.push(Grey {
                    id,
                    from: 0,
                    map: true,
                });
            }
        } else if d.is_array() || d.is_struct() || d.is_enum() {
            let id = d.as_array() as u32;
            if id < self.array_limit && !self.array_marks.insert(id as usize) {
                self.work.push(Grey {
                    id,
                    from: 0,
                    map: false,
                });
            }
        } else if d.is_large_str() {
            strings.shade(d.get_str_pool_id());
        }
    }

    /// Walks the next chunk of `g`, queueing the rest of it when it is longer.
    ///
    /// Each entry is tested on its own, never read as a verdict on its
    /// neighbours: a list or map typed `any`, which is what a parsed json
    /// document is made of, mixes scalars and objects freely.
    #[inline(always)]
    fn walk(&mut self, objs: &ObjectPool, maps: &MapPool, strings: &mut StringPool, g: Grey) {
        let from = g.from as usize;
        if g.map {
            let map = &maps[g.id as usize];
            let end = map.len().min(from + CHUNK as usize);
            if let Some(entries) = map.get_range(from..end) {
                for (k, v) in entries {
                    self.shade(strings, *k);
                    self.shade(strings, *v);
                }
            }
            if end < map.len() {
                self.work.push(Grey {
                    from: end as u32,
                    ..g
                });
            }
        } else {
            let list = &objs[g.id as usize];
            let end = list.len().min(from + CHUNK as usize);
            if from < end {
                for &e in unsafe { list.get_unchecked(from..end) } {
                    self.shade(strings, e);
                }
            }
            if end < list.len() {
                self.work.push(Grey {
                    from: end as u32,
                    ..g
                });
            }
        }
    }
}

/// What the collector knows about the pools between two allocations.
///
/// This lives with the pools rather than with one run of the interpreter
/// because the pools do: a host that calls into a resident program many times
/// keeps every object the earlier calls left behind, and a collector that
/// forgot its free lists and started its thresholds over on each call would
/// run a full mark and sweep on the first few allocations of every call once
/// the pools had grown past the starting threshold, and would never reuse a
/// slot the previous call had freed. A cycle in progress carries over between
/// calls the same way.
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
    phase: Phase,
    tracer: Tracer,
    /// The next mark word the sweep reads in the pool it is sweeping.
    sweep_word: usize,
    /// How many cycles have begun.
    cycles: u64,
    /// Every unit of work done, across all cycles.
    units: u64,
    /// The most units one step has spent.
    largest_slice: u32,
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
            phase: Phase::Idle,
            tracer: Tracer::default(),
            sweep_word: 0,
            cycles: 0,
            units: 0,
            largest_slice: 0,
        }
    }
}

impl GcState {
    /// How many collections have begun over these pools.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Whether a cycle is in progress.
    #[inline(always)]
    #[must_use]
    pub(crate) fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Begins a cycle: records each pool's length, marks every free slot, and
    /// marks what the roots hold. Answers the units it spent, which it spends
    /// whatever the budget, since the roots have to be read all at once.
    fn begin(
        &mut self,
        objs: &ObjectPool,
        maps: &MapPool,
        strings: &mut StringPool,
        roots: [&[Data]; 2],
    ) -> u32 {
        debug_assert!(!self.active());
        self.cycles += 1;
        let tracer = &mut self.tracer;
        tracer.array_limit = objs.len() as u32;
        tracer.map_limit = maps.len() as u32;
        tracer.array_marks.reset(objs.len());
        tracer.map_marks.reset(maps.len());
        strings.begin_marking();
        // A free slot is garbage already, and marking it keeps the sweep from
        // putting it on the free list a second time.
        for &id in &self.free_arrays {
            tracer.array_marks.insert(id as usize);
        }
        for &id in &self.free_maps {
            tracer.map_marks.insert(id as usize);
        }
        for &id in &self.free_strings {
            strings.shade(id as usize);
        }
        let mut read = self.free_arrays.len() + self.free_maps.len() + self.free_strings.len();
        for root in roots {
            read += root.len();
            for &d in root {
                tracer.shade(strings, d);
            }
        }
        self.phase = Phase::Mark;
        // One unit per 64 values read, and one per 64 mark words cleared.
        let cleared = (objs.len() + maps.len() + strings.len()) / 64;
        ((read + cleared) / 64 + 1).min(u32::MAX as usize) as u32
    }

    /// Carries the cycle forward by at most `budget` units, and answers
    /// whether it is over. A cycle that has not begun is over already.
    fn step(
        &mut self,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        budget: u32,
    ) -> bool {
        let mut spent = 0u32;
        while spent < budget {
            match self.phase {
                Phase::Idle => break,
                Phase::Mark => {
                    while spent < budget {
                        let Some(g) = self.tracer.work.pop() else {
                            break;
                        };
                        self.tracer.walk(objs, maps, strings, g);
                        spent += 1;
                    }
                    if self.tracer.work.is_empty() {
                        self.phase = Phase::Sweep(Swept::Arrays);
                        self.sweep_word = 0;
                    }
                }
                Phase::Sweep(pool) => {
                    spent += self.sweep(pool, objs, maps, strings, budget - spent);
                }
            }
        }
        self.record(spent);
        self.phase == Phase::Idle
    }

    /// Adds `spent` units to the totals.
    fn record(&mut self, spent: u32) {
        self.units += u64::from(spent);
        self.largest_slice = self.largest_slice.max(spent);
    }

    /// Sweeps `pool` for at most about `budget` units, moving on to the next
    /// pool, or ending the cycle, once it is done. Answers the units spent.
    fn sweep(
        &mut self,
        pool: Swept,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        budget: u32,
    ) -> u32 {
        let mut spent = 0u32;
        let (limit, marks, free) = match pool {
            Swept::Arrays => (
                self.tracer.array_limit as usize,
                &self.tracer.array_marks,
                &mut self.free_arrays,
            ),
            Swept::Maps => (
                self.tracer.map_limit as usize,
                &self.tracer.map_marks,
                &mut self.free_maps,
            ),
            Swept::Strings => (
                strings.mark_limit(),
                strings.marks(),
                &mut self.free_strings,
            ),
        };
        let words = limit.div_ceil(64);
        // The string marks are borrowed from the pool the freed slots are
        // emptied in, so the string sweep gathers the slots first.
        let freed_from = free.len();
        while self.sweep_word < words && spent < budget {
            let w = self.sweep_word;
            let mut unmarked = marks.unmarked_in_word(w, limit);
            while unmarked != 0 {
                free.push((w * 64) as u32 + unmarked.trailing_zeros());
                unmarked &= unmarked - 1;
                spent += 1;
            }
            spent += 1;
            self.sweep_word += 1;
        }
        let done = self.sweep_word >= words;
        match pool {
            Swept::Arrays => {
                for &id in &self.free_arrays[freed_from..] {
                    let slot = objs.get_mut(id as usize);
                    if slot.capacity() > KEEP_FREED_CAPACITY {
                        *slot = Vec::new();
                    }
                }
                if done {
                    self.array_threshold = next_threshold(objs.len(), self.free_arrays.len());
                    self.phase = Phase::Sweep(Swept::Maps);
                }
            }
            Swept::Maps => {
                for &id in &self.free_maps[freed_from..] {
                    let slot = maps.get_mut(id as usize);
                    if slot.capacity() > KEEP_FREED_CAPACITY {
                        *slot = CandelaMap::default();
                    }
                }
                if done {
                    self.map_threshold = next_threshold(maps.len(), self.free_maps.len());
                    self.phase = Phase::Sweep(Swept::Strings);
                }
            }
            Swept::Strings => {
                // A freed slot gives up its text. Interning finds a string by
                // comparing text across the whole pool, so a freed slot that
                // kept its text would be handed out as a live string and then
                // overwritten by the next allocation that reuses the slot.
                for &id in &self.free_strings[freed_from..] {
                    strings.release(id as usize);
                }
                if done {
                    self.string_threshold = next_threshold(strings.len(), self.free_strings.len());
                    strings.end_marking();
                    self.phase = Phase::Idle;
                }
            }
        }
        if done {
            self.sweep_word = 0;
        }
        spent
    }

    /// What an allocation does when it has to grow its pool and either a cycle
    /// is running or the pool has reached its threshold: begins a cycle if none
    /// is running, and runs it to the end.
    #[cold]
    #[inline(never)]
    fn allocation_step(
        &mut self,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        roots: [&[Data]; 2],
    ) {
        let mut spent = 0;
        if !self.active() {
            spent = self.begin(objs, maps, strings, roots);
        }
        self.record(spent);
        self.step(objs, maps, strings, u32::MAX);
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
    if gc.active() || objs.len() >= gc.array_threshold as usize {
        gc.allocation_step(objs, maps, strings, [&registers.0, &recursion_stack.0]);
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
    if gc.active() || maps.len() >= gc.map_threshold as usize {
        gc.allocation_step(objs, maps, strings, [&registers.0, &recursion_stack.0]);
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
    if gc.free_strings.is_empty() && (gc.active() || strings.len() >= gc.string_threshold as usize)
    {
        gc.allocation_step(objs, maps, strings, [&registers.0, &recursion_stack.0]);
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
