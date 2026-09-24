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
//! - Idle: nothing to do. A cycle begins when a pool that has reached its
//!   threshold is down to its reserve of free slots: enough for the program
//!   to allocate from while the cycle marks, so the pool does not have to
//!   grow for it. While the cycle runs, every allocation pays for a share of
//!   its work, done a slice at a time, so the program never stops for more
//!   than one slice.
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
//!
//! A host can do the same work while its program is idle, between frames, by
//! calling [`GcState::collect`] with a budget of its own. That drives the same
//! cycle the allocations drive: a cycle an allocation began is carried on by
//! the host, and the other way round. A host that collects this way is taken
//! to pace the collector, and the thresholds at which an allocation begins a
//! cycle rise to leave the work to it.
//!
//! The program keeps running between steps, so marking is
//! snapshot-at-the-beginning: every object reachable when the cycle began is
//! marked before the sweep starts, whatever the program does to the heap in
//! the meantime. Three rules keep that true:
//!
//! - A store that overwrites or removes a value from a list, struct, cell or
//!   map while marking runs marks the value it takes out first (the deletion
//!   barrier, [`shade_overwritten`]). A value the program read out of the heap
//!   and kept only in a register is still marked that way.
//! - A list or map whose entries move in place (a removal, a sort, a reverse)
//!   after marking has walked part of it is walked again from the start
//!   ([`rescan_list`], [`rescan_map`]), since an entry could have moved from
//!   the part not yet walked into the part already walked.
//! - Interning a string hands out a slot found by its text, which the program
//!   may have held no reference to; the slot is marked as it is handed out.
//!
//! Registers and the recursion stack take no barrier. They are read once, when
//! the cycle begins, and anything they receive later was either reachable then
//! or allocated since.
//!
//! Building with the `gc-torture` feature runs a cycle on every allocation, a
//! unit of work at a time, and checks at the end of every mark phase that a
//! fresh trace from the roots finds nothing unmarked.

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

/// Collect on every allocation, a unit at a time, and check every mark phase
/// against a fresh trace. See the module documentation.
const TORTURE: bool = cfg!(feature = "gc-torture");

/// The least a threshold falls to: under torture every allocation starts a
/// cycle.
const THRESHOLD_FLOOR: u32 = if TORTURE { 0 } else { MIN_GC_THRESHOLD };

/// Units of collector work each allocation pays for while a cycle runs. A
/// cycle with `n` units of work finishes within `n / ALLOC_DEBT` allocations.
const ALLOC_DEBT: u32 = 16;

/// The units one allocation-driven step does: the allocations' debt is paid
/// once it reaches this, which bounds how long one allocation stops the
/// program.
const SLICE: u32 = 512;

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
    /// Whether slot `i` is marked.
    #[inline(always)]
    #[must_use]
    pub fn contains(&self, i: usize) -> bool {
        unsafe { self.0.get_unchecked(i / 64) & (1u64 << (i % 64)) != 0 }
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

/// The threshold the next collection of a pool runs at: `factor` times what
/// survived this one. A pool that doubles its live set between collections
/// pays for each collection with as many fresh allocations as the trace
/// visited, and a pool whose live set stays flat stops growing instead of
/// doubling on every collection whether anything survived or not.
#[inline(always)]
fn next_threshold(pool_len: usize, freed: usize, factor: usize) -> u32 {
    if TORTURE {
        return 0;
    }
    let live = pool_len - freed;
    (live.saturating_mul(factor).min(u32::MAX as usize) as u32).max(MIN_GC_THRESHOLD)
}

/// How far past its live set a pool grows before an allocation begins a cycle,
/// for a program nobody else collects for.
const GROWTH: usize = 2;

/// The same, once the host collects between frames: the allocations leave room
/// for the host to do the work first, and begin a cycle themselves only when
/// it falls behind.
const HOST_PACED_GROWTH: usize = 4;

/// The share of a pool its free slots have to fall below before an idle
/// collection begins a cycle for it: a pool with more free than this has room
/// to allocate in without growing.
const IDLE_FREE_SHARE: usize = 8;

/// What the collector has done over a set of pools, and how large they are.
///
/// Work is counted in the units a budget is given in, never in time: the count
/// is the same on every machine and every run, and a host that wants time
/// measures its own calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    /// How many collection cycles have begun.
    pub cycles: u64,
    /// Every unit of collection work done, across all cycles.
    pub units: u64,
    /// The most units one step did: the longest the collector held up the
    /// program at once, whether an allocation or the host asked for the step.
    pub largest_slice: u32,
    /// The object pool: lists, structs, enum values, functions and closure
    /// cells.
    pub arrays: PoolStats,
    /// The map pool.
    pub maps: PoolStats,
    /// The pool of strings too long to sit inside a value.
    pub strings: PoolStats,
}

/// How large one pool is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// Every slot the pool holds, live or free.
    pub len: usize,
    /// The slots on its free list, waiting for an allocation to reuse them.
    pub free: usize,
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

/// A pool's free slots, kept in two lists so that the common allocation asks
/// the collector nothing: it takes a slot from `ready`, and only when that is
/// empty does it go to the collector, which decides from there.
struct FreeSlots<'a> {
    /// Slots an allocation takes without asking the collector.
    ready: &'a mut Vec<u32>,
    /// The rest: the reserve that feeds the next cycle, and every free slot
    /// while a cycle runs, so each allocation then pays its share.
    held: &'a mut Vec<u32>,
}

impl FreeSlots<'_> {
    /// A free slot, ready or held.
    fn take(&mut self) -> Option<u32> {
        self.ready.pop().or_else(|| self.held.pop())
    }
    /// Holds every free slot back, so each allocation goes to the collector.
    /// The lists trade places rather than copy when one is empty, which is the
    /// usual case, since a pool's free list can run to most of the pool.
    fn hold_all(&mut self) {
        if self.held.is_empty() {
            std::mem::swap(self.ready, self.held);
        } else {
            self.held.append(self.ready);
        }
    }
    /// Makes every free slot ready except `reserve` of them.
    fn release_above(&mut self, reserve: usize) {
        if self.ready.is_empty() {
            std::mem::swap(self.ready, self.held);
            let keep = reserve.min(self.ready.len());
            let from = self.ready.len() - keep;
            self.held.extend(self.ready.drain(from..));
        } else if self.held.len() > reserve {
            self.ready.extend(self.held.drain(reserve..));
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
    /// Object-pool slots (arrays, structs, enums and function values) an
    /// allocation takes without asking the collector.
    pub(crate) free_arrays: Vec<u32>,
    /// Map-pool slots an allocation takes without asking the collector.
    pub(crate) free_maps: Vec<u32>,
    /// String-pool slots an allocation takes without asking the collector.
    pub(crate) free_strings: Vec<u32>,
    /// Object-pool length from which a cycle may begin.
    array_threshold: u32,
    /// Map-pool length from which a cycle may begin.
    map_threshold: u32,
    /// String-pool length from which a cycle may begin.
    string_threshold: u32,
    phase: Phase,
    tracer: Tracer,
    /// The next mark word the sweep reads in the pool it is sweeping.
    sweep_word: usize,
    /// The free slots below the limit of the pool being swept: those already
    /// free when the cycle began, plus those the sweep has freed so far. The
    /// program reuses freed slots while the sweep runs, so the free list's
    /// length says nothing about what survived.
    swept_free: usize,
    /// Each pool's free slots when the cycle began: arrays, maps, strings.
    free_at_begin: [usize; 3],
    /// Units of work the allocations of the cycle in progress have paid for
    /// and no step has done yet.
    debt: u32,
    /// How many free slots each pool keeps back for the next cycle to
    /// allocate from while it marks. Set from how much the last cycle marked.
    reserve: usize,
    /// Units the cycle in progress has spent marking.
    mark_units: usize,
    /// How many cycles have begun.
    cycles: u64,
    /// Every unit of work done, across all cycles.
    units: u64,
    /// The most units one step has spent.
    largest_slice: u32,
    /// Each pool's free slots held back from the ready lists above: arrays,
    /// maps, strings. See [`FreeSlots`].
    held: [Vec<u32>; 3],
    /// Each pool's occupied slots when the last cycle swept it: arrays, maps,
    /// strings. An idle collection measures growth from here.
    occupied_after: [usize; 3],
    /// Whether the host has collected, which raises the thresholds.
    host_paced: bool,
}

impl Default for GcState {
    fn default() -> Self {
        Self {
            free_arrays: Vec::new(),
            free_maps: Vec::new(),
            free_strings: Vec::new(),
            array_threshold: THRESHOLD_FLOOR,
            map_threshold: THRESHOLD_FLOOR,
            string_threshold: THRESHOLD_FLOOR,
            phase: Phase::Idle,
            tracer: Tracer::default(),
            sweep_word: 0,
            swept_free: 0,
            free_at_begin: [0; 3],
            debt: 0,
            reserve: 0,
            mark_units: 0,
            cycles: 0,
            units: 0,
            largest_slice: 0,
            held: [Vec::new(), Vec::new(), Vec::new()],
            occupied_after: [0; 3],
            host_paced: false,
        }
    }
}

impl GcState {
    /// Whether this build runs a cycle on every allocation, a unit of work at
    /// a time, which is what the `gc-torture` feature turns on.
    pub const TORTURE: bool = TORTURE;

    /// How many collections have begun over these pools.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    /// What the collector has done over `objs`, `maps` and `strings`, the
    /// pools this state collects.
    #[must_use]
    pub fn stats(&self, objs: &ObjectPool, maps: &MapPool, strings: &StringPool) -> GcStats {
        GcStats {
            cycles: self.cycles,
            units: self.units,
            largest_slice: self.largest_slice,
            arrays: PoolStats {
                len: objs.len(),
                free: self.free_len(Swept::Arrays),
            },
            maps: PoolStats {
                len: maps.len(),
                free: self.free_len(Swept::Maps),
            },
            strings: PoolStats {
                len: strings.len(),
                free: self.free_len(Swept::Strings),
            },
        }
    }

    /// Collection work for a host to do while its program is idle: carries
    /// the cycle in progress forward by about `budget` units, beginning one
    /// first if the pools have grown enough since the last to be worth it.
    /// Answers whether no collection work is left.
    ///
    /// `registers` are the roots. The host calls this between calls into the
    /// program, when the registers are the only roots there are.
    ///
    /// Beginning a cycle reads every root at once, whatever the budget; after
    /// that a step stops within one chunk or one mark word of the budget. A
    /// cycle begins only when some pool has grown by half its live set (and by
    /// at least [`MIN_GC_THRESHOLD`] slots) since the last one and has fewer
    /// than an eighth of its slots free; otherwise the call does nothing.
    pub fn collect(
        &mut self,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        registers: &[Data],
        budget: u32,
    ) -> bool {
        self.host_paced = true;
        if !self.active() && !self.worth_collecting(objs.len(), maps.len(), strings.len()) {
            return true;
        }
        if budget == 0 {
            return false;
        }
        let roots = [registers, &[][..]];
        let mut spent = 0;
        if !self.active() {
            spent = self.begin(objs, maps, strings, roots);
        }
        spent += self.advance(objs, maps, strings, roots, budget);
        // Work the host did is work the allocations no longer owe.
        self.debt = self.debt.saturating_sub(spent);
        self.record(spent);
        !self.active()
    }

    /// Whether an idle collection should begin a cycle: some pool has grown
    /// by half its live set since the last cycle and has few free slots left.
    fn worth_collecting(&self, arrays: usize, maps: usize, strings: usize) -> bool {
        let lens = [arrays, maps, strings];
        [Swept::Arrays, Swept::Maps, Swept::Strings]
            .into_iter()
            .enumerate()
            .any(|(i, pool)| {
                let free = self.free_len(pool);
                let occupied = lens[i].saturating_sub(free);
                let live = self.occupied_after[i];
                let grown = occupied.saturating_sub(live);
                grown >= (live / 2).max(MIN_GC_THRESHOLD as usize)
                    && free < lens[i] / IDLE_FREE_SHARE
            })
    }

    /// How many free slots `pool` has, ready and held.
    fn free_len(&self, pool: Swept) -> usize {
        let (ready, held) = match pool {
            Swept::Arrays => (&self.free_arrays, &self.held[0]),
            Swept::Maps => (&self.free_maps, &self.held[1]),
            Swept::Strings => (&self.free_strings, &self.held[2]),
        };
        ready.len() + held.len()
    }

    /// The factor [`next_threshold`] grows a pool's live set by.
    const fn growth(&self) -> usize {
        if self.host_paced {
            HOST_PACED_GROWTH
        } else {
            GROWTH
        }
    }

    /// Whether marking is under way, which is when a store has to mark the
    /// value it overwrites.
    #[inline(always)]
    #[must_use]
    pub(crate) fn marking(&self) -> bool {
        self.phase == Phase::Mark
    }

    /// The free slots of `pool`, ready and held.
    fn free(&mut self, pool: Swept) -> FreeSlots<'_> {
        let [arrays, maps, strings] = &mut self.held;
        match pool {
            Swept::Arrays => FreeSlots {
                ready: &mut self.free_arrays,
                held: arrays,
            },
            Swept::Maps => FreeSlots {
                ready: &mut self.free_maps,
                held: maps,
            },
            Swept::Strings => FreeSlots {
                ready: &mut self.free_strings,
                held: strings,
            },
        }
    }

    /// Whether a cycle is in progress.
    #[inline(always)]
    #[must_use]
    pub(crate) fn active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Begins a cycle: records each pool's length, holds back and marks every
    /// free slot, and marks what the roots hold. Answers the units it spent,
    /// which it spends whatever the budget, since the roots have to be read
    /// all at once. The caller adds them to the slice it records.
    fn begin(
        &mut self,
        objs: &ObjectPool,
        maps: &MapPool,
        strings: &mut StringPool,
        roots: [&[Data]; 2],
    ) -> u32 {
        debug_assert!(!self.active());
        self.cycles += 1;
        self.mark_units = 0;
        self.debt = 0;
        self.free(Swept::Arrays).hold_all();
        self.free(Swept::Maps).hold_all();
        self.free(Swept::Strings).hold_all();
        self.free_at_begin = [self.held[0].len(), self.held[1].len(), self.held[2].len()];
        let tracer = &mut self.tracer;
        tracer.array_limit = objs.len() as u32;
        tracer.map_limit = maps.len() as u32;
        tracer.array_marks.reset(objs.len());
        tracer.map_marks.reset(maps.len());
        strings.begin_marking();
        // A free slot is garbage already, and marking it keeps the sweep from
        // putting it on the free list a second time.
        for &id in &self.held[0] {
            tracer.array_marks.insert(id as usize);
        }
        for &id in &self.held[1] {
            tracer.map_marks.insert(id as usize);
        }
        for &id in &self.held[2] {
            strings.shade(id as usize);
        }
        let mut read = self.free_at_begin.iter().sum::<usize>();
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

    /// Carries the cycle forward by about `budget` units, and answers the
    /// units spent. Marking stops at the budget; sweeping can pass it by up to
    /// one mark word's worth of freed slots. A cycle that has not begun has
    /// nothing to do. The caller records the slice.
    fn advance(
        &mut self,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        roots: [&[Data]; 2],
        budget: u32,
    ) -> u32 {
        let mut spent = 0u32;
        while spent < budget {
            match self.phase {
                Phase::Idle => break,
                Phase::Mark => {
                    let before = spent;
                    while spent < budget {
                        let Some(g) = self.tracer.work.pop() else {
                            break;
                        };
                        self.tracer.walk(objs, maps, strings, g);
                        spent += 1;
                    }
                    self.mark_units += (spent - before) as usize;
                    if self.tracer.work.is_empty() {
                        if TORTURE {
                            self.verify(objs, maps, strings, roots);
                        }
                        // The next cycle marks about as much as this one did,
                        // and the program allocates from the reserve while it
                        // does.
                        self.reserve = (self.mark_units + SLICE as usize) / ALLOC_DEBT as usize;
                        self.phase = Phase::Sweep(Swept::Arrays);
                        self.sweep_word = 0;
                        self.swept_free = self.free_at_begin[0];
                    }
                }
                Phase::Sweep(pool) => {
                    spent += self.sweep(pool, objs, maps, strings, budget - spent);
                }
            }
        }
        spent
    }

    /// Adds a step of `spent` units to the totals.
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
                &mut self.held[0],
            ),
            Swept::Maps => (
                self.tracer.map_limit as usize,
                &self.tracer.map_marks,
                &mut self.held[1],
            ),
            Swept::Strings => (strings.mark_limit(), strings.marks(), &mut self.held[2]),
        };
        let words = limit.div_ceil(64);
        // The string marks are borrowed from the pool the freed slots are
        // emptied in, so the sweep gathers the slots first and empties them
        // after.
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
        self.swept_free += free.len() - freed_from;
        let done = self.sweep_word >= words;
        match pool {
            Swept::Arrays => {
                for &id in &self.held[0][freed_from..] {
                    let slot = objs.get_mut(id as usize);
                    if slot.capacity() > KEEP_FREED_CAPACITY {
                        *slot = Vec::new();
                    }
                }
                if done {
                    self.array_threshold = next_threshold(limit, self.swept_free, self.growth());
                    self.occupied_after[0] = objs.len() - self.free_len(pool);
                    self.phase = Phase::Sweep(Swept::Maps);
                    self.swept_free = self.free_at_begin[1];
                }
            }
            Swept::Maps => {
                for &id in &self.held[1][freed_from..] {
                    let slot = maps.get_mut(id as usize);
                    if slot.capacity() > KEEP_FREED_CAPACITY {
                        *slot = CandelaMap::default();
                    }
                }
                if done {
                    self.map_threshold = next_threshold(limit, self.swept_free, self.growth());
                    self.occupied_after[1] = maps.len() - self.free_len(pool);
                    self.phase = Phase::Sweep(Swept::Strings);
                    self.swept_free = self.free_at_begin[2];
                }
            }
            Swept::Strings => {
                // A freed slot gives up its text. Interning finds a string by
                // comparing text across the whole pool, so a freed slot that
                // kept its text would be handed out as a live string and then
                // overwritten by the next allocation that reuses the slot.
                for &id in &self.held[2][freed_from..] {
                    strings.release(id as usize);
                }
                if done {
                    self.string_threshold = next_threshold(limit, self.swept_free, self.growth());
                    self.occupied_after[2] = strings.len() - self.free_len(pool);
                    strings.end_marking();
                    self.finish();
                }
            }
        }
        if done {
            self.sweep_word = 0;
        }
        spent
    }

    /// Ends the cycle: every free slot above each pool's reserve becomes ready
    /// for allocations to take without asking the collector.
    fn finish(&mut self) {
        self.phase = Phase::Idle;
        if !TORTURE {
            let reserve = self.reserve;
            self.free(Swept::Arrays).release_above(reserve);
            self.free(Swept::Maps).release_above(reserve);
            self.free(Swept::Strings).release_above(reserve);
        }
    }

    /// What an allocation does when its pool has no ready slot. With a cycle
    /// running it adds ALLOC_DEBT units to what the program owes, and a step
    /// pays the debt off once it reaches SLICE. With none running, a pool at
    /// its threshold begins one; a pool below it makes its reserve ready, since
    /// it has room to grow before it needs a cycle.
    #[inline(never)]
    fn on_empty(
        &mut self,
        pool: Swept,
        objs: &mut ObjectPool,
        maps: &mut MapPool,
        strings: &mut StringPool,
        roots: [&[Data]; 2],
    ) {
        if TORTURE {
            let mut spent = 0;
            if !self.active() {
                spent = self.begin(objs, maps, strings, roots);
            }
            spent += self.advance(objs, maps, strings, roots, 1);
            self.record(spent);
            return;
        }
        if self.active() {
            self.debt += ALLOC_DEBT;
            if self.debt >= SLICE {
                let budget = self.debt;
                self.debt = 0;
                let spent = self.advance(objs, maps, strings, roots, budget);
                self.record(spent);
            }
            return;
        }
        let (len, threshold) = match pool {
            Swept::Arrays => (objs.len(), self.array_threshold),
            Swept::Maps => (maps.len(), self.map_threshold),
            Swept::Strings => (strings.len(), self.string_threshold),
        };
        if len >= threshold as usize {
            let spent = self.begin(objs, maps, strings, roots);
            self.record(spent);
        } else {
            self.free(pool).release_above(0);
        }
    }

    /// Checks a finished mark phase against a fresh trace from `roots`: every
    /// object the trace reaches that the cycle is responsible for has to be
    /// marked, or the sweep would free something the program can still read.
    #[cold]
    #[inline(never)]
    fn verify(&self, objs: &ObjectPool, maps: &MapPool, strings: &StringPool, roots: [&[Data]; 2]) {
        let t = &self.tracer;
        let mut seen_arrays = vec![false; objs.len()];
        let mut seen_maps = vec![false; maps.len()];
        let mut stack: Vec<Data> = roots.iter().flat_map(|r| r.iter().copied()).collect();
        while let Some(d) = stack.pop() {
            if d.is_map() {
                let id = d.as_map();
                if std::mem::replace(&mut seen_maps[id], true) {
                    continue;
                }
                assert!(
                    id >= t.map_limit as usize || t.map_marks.contains(id),
                    "gc-torture: map {id} is reachable but unmarked at the end of marking"
                );
                for (k, v) in &maps[id] {
                    stack.push(*k);
                    stack.push(*v);
                }
            } else if d.is_array() || d.is_struct() || d.is_enum() {
                let id = d.as_array();
                if std::mem::replace(&mut seen_arrays[id], true) {
                    continue;
                }
                assert!(
                    id >= t.array_limit as usize || t.array_marks.contains(id),
                    "gc-torture: object {id} is reachable but unmarked at the end of marking"
                );
                stack.extend_from_slice(&objs[id]);
            } else if d.is_large_str() {
                let id = d.get_str_pool_id();
                assert!(
                    id >= strings.mark_limit() || strings.marks().contains(id),
                    "gc-torture: string {id} is reachable but unmarked at the end of marking"
                );
            }
        }
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
    alloc_array_slow(objs, maps, strings, registers, recursion_stack, gc)
}

/// [`alloc_array`] when no slot is ready.
#[inline(never)]
fn alloc_array_slow(
    objs: &mut ObjectPool,
    maps: &mut MapPool,
    strings: &mut StringPool,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcState,
) -> u32 {
    let roots = [&registers.0[..], &recursion_stack.0[..]];
    gc.on_empty(Swept::Arrays, objs, maps, strings, roots);
    if let Some(id) = gc.free(Swept::Arrays).take() {
        objs[id as usize].clear();
        return id;
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
    let roots = [&registers.0[..], &recursion_stack.0[..]];
    gc.on_empty(Swept::Maps, objs, maps, strings, roots);
    if let Some(id) = gc.free(Swept::Maps).take() {
        maps[id as usize].clear();
        return id;
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
    let slot = match gc.free_strings.pop() {
        Some(id) => Some(id),
        None => {
            let roots = [&registers.0[..], &recursion_stack.0[..]];
            gc.on_empty(Swept::Strings, objs, maps, strings, roots);
            gc.free(Swept::Strings).take()
        }
    };
    if let Some(id) = slot {
        s.move_to_slot(strings.get_mut(id as usize));
        u64::from(id)
    } else {
        let id = strings.len() as u64;
        s.push_to_pool(strings);
        id
    }
}

/// The pools and collector a value built outside the instruction stream
/// allocates through, with the roots a collection it starts reads.
///
/// Every allocation here can start or carry on a collection, so whatever the
/// builder holds has to be reachable from `registers` or `recursion_stack`
/// before it allocates again. A nested value is built top-down to keep it so:
/// the outermost container sits on `recursion_stack` while it is filled, and
/// each entry goes into its parent before anything inside it is allocated.
pub(crate) struct Heap<'a> {
    pub objs: &'a mut ObjectPool,
    pub maps: &'a mut MapPool,
    pub strings: &'a mut StringPool,
    pub gc: &'a mut GcState,
    pub registers: &'a RegisterFile,
    pub recursion_stack: &'a mut RegisterFile,
}

impl Heap<'_> {
    /// A new empty list.
    pub fn array(&mut self) -> Data {
        Data::array(alloc_array(
            self.objs,
            self.maps,
            self.strings,
            self.registers,
            self.recursion_stack,
            self.gc,
        ))
    }

    /// A new empty map.
    pub fn map(&mut self) -> Data {
        Data::map(alloc_map(
            self.objs,
            self.maps,
            self.strings,
            self.registers,
            self.recursion_stack,
            self.gc,
        ))
    }

    /// `s` as a value, sharing the pool slot of an equal string, since a map
    /// finds a string key by its slot.
    pub fn string(&mut self, s: &str) -> Data {
        Data::interned(s, self.strings).unwrap_or_else(|| {
            Data::string(
                s,
                self.objs,
                self.maps,
                self.strings,
                self.registers,
                self.recursion_stack,
                self.gc,
            )
        })
    }
}

/// The deletion barrier: marks `old`, a value a store is about to overwrite or
/// remove from the heap while marking runs. Out of line, so a store pays one
/// test of [`GcState::marking`] when no cycle is marking.
#[cold]
#[inline(never)]
pub fn shade_overwritten(gc: &mut GcState, strings: &mut StringPool, old: Data) {
    gc.tracer.shade(strings, old);
}

/// Walks list `id` again from its first entry, if marking has already reached
/// it. Called after its entries moved in place.
#[cold]
#[inline(never)]
pub fn rescan_list(gc: &mut GcState, id: usize) {
    let t = &mut gc.tracer;
    if id < t.array_limit as usize && t.array_marks.contains(id) {
        t.work.push(Grey {
            id: id as u32,
            from: 0,
            map: false,
        });
    }
}

/// Walks map `id` again from its first entry, if marking has already reached
/// it. Called after its entries moved in place.
#[cold]
#[inline(never)]
pub fn rescan_map(gc: &mut GcState, id: usize) {
    let t = &mut gc.tracer;
    if id < t.map_limit as usize && t.map_marks.contains(id) {
        t.work.push(Grey {
            id: id as u32,
            from: 0,
            map: true,
        });
    }
}
