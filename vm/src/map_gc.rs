// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::array_gc::mark;
use crate::array_gc::next_threshold;
use crate::array_gc::reset_marks;
use crate::vm::CandelaMap;
use crate::vm::GcScratch;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::RegisterFile;

/// Allocates a new map in the map pool. If reusing a map, it clears it.
pub fn alloc_map(
    map_pool: &mut MapPool,
    obj_pool: &ObjectPool,
    free_maps: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc_map_threshold: &mut u32,
    gc: &mut GcScratch,
) -> u32 {
    if let Some(id) = free_maps.pop() {
        map_pool[id as usize].clear();
        return id;
    }
    if map_pool.len() >= (*gc_map_threshold as usize) {
        map_gc(
            map_pool,
            obj_pool,
            free_maps,
            registers,
            recursion_stack,
            gc,
        );
        *gc_map_threshold = next_threshold(map_pool.len(), free_maps.len());
        if let Some(id) = free_maps.pop() {
            map_pool[id as usize].clear();
            return id;
        }
    }
    let id = map_pool.len() as u32;
    map_pool.push(CandelaMap::default());
    id
}

/// Rebuilds the map free list from a fresh trace. It runs only once the free
/// list is empty, so every slot the trace leaves unmarked is garbage.
pub fn map_gc(
    map_pool: &MapPool,
    obj_pool: &ObjectPool,
    free_maps: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcScratch,
) {
    reset_marks(gc, obj_pool.len(), map_pool.len());
    mark::<false>(registers, recursion_stack, obj_pool, map_pool, gc);
    free_maps.clear();
    gc.map_live.push_unmarked(map_pool.len(), free_maps);
}
