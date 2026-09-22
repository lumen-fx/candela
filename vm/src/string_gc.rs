// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::array_gc::mark;
use crate::array_gc::reset_marks;
use crate::vm::GcScratch;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::RegisterFile;
use crate::vm::StringPool;

/// Frees every pooled string no live value can still reach, rebuilding the
/// string free list. It runs only once that list is empty, so every slot the
/// trace leaves unmarked is garbage.
///
/// A string is reachable straight from a register or through any collection a
/// register holds, so the walk crosses both object pools: a list can hold a
/// map, a map can hold a list, and a document parsed from json holds both at
/// arbitrary depth. The object marks keep that walk from revisiting a shared
/// object or looping on a cyclic one.
pub fn string_gc(
    array_pool: &ObjectPool,
    map_pool: &MapPool,
    string_pool: &StringPool,
    free_strings: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcScratch,
) {
    reset_marks(gc, array_pool.len(), map_pool.len());
    gc.string_live.reset(string_pool.len());
    mark::<true>(registers, recursion_stack, array_pool, map_pool, gc);
    free_strings.clear();
    gc.string_live
        .push_unmarked(string_pool.len(), free_strings);
}
