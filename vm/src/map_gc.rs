use crate::array_gc::reset_marks;
use crate::array_gc::track;
use crate::vm::GcScratch;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::RegisterFile;
use std::collections::HashMap;

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
        id
    } else {
        if map_pool.len() >= (*gc_map_threshold as usize) {
            *gc_map_threshold *= 2;
            map_gc(
                map_pool,
                obj_pool,
                free_maps,
                registers,
                recursion_stack,
                gc,
            );
        }
        if let Some(id) = free_maps.pop() {
            map_pool[id as usize].clear();
            id
        } else {
            let id = map_pool.len() as u32;
            map_pool.push(HashMap::default());
            id
        }
    }
}

pub fn map_gc(
    map_pool: &MapPool,
    obj_pool: &ObjectPool,
    free_maps: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcScratch,
) {
    reset_marks(gc, obj_pool.len(), map_pool.len());
    for data in registers.0.iter().chain(recursion_stack.0.iter()) {
        if data.is_map() {
            track_maps(data.as_map(), map_pool, obj_pool, gc);
        } else if data.is_array() || data.is_struct() || data.is_enum() {
            track(*data, obj_pool, map_pool, gc);
        }
    }

    for &id in free_maps.iter() {
        unsafe {
            *gc.map_live.get_unchecked_mut(id as usize) = true;
        }
    }

    for (i, map_alive) in gc.map_live.iter().enumerate() {
        if !map_alive {
            free_maps.push(i as u32);
        }
    }
}

pub fn track_maps(idx: usize, map_pool: &MapPool, obj_pool: &ObjectPool, gc: &mut GcScratch) {
    let is_live = unsafe { gc.map_live.get_unchecked_mut(idx) };
    if *is_live {
        return;
    }
    *is_live = true;
    // Each entry is tested on its own, for the same reason the array trace
    // tests each element: a map typed `any` holds whatever the document held,
    // so one entry cannot stand in for the rest.
    for (k, v) in &map_pool[idx] {
        for d in [k, v] {
            if d.is_array() || d.is_struct() || d.is_enum() || d.is_map() {
                track(*d, obj_pool, map_pool, gc);
            }
        }
    }
}
