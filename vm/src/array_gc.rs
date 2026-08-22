use crate::data::Data;
use crate::map_gc::track_maps;
use crate::vm::{GcScratch, MapPool, ObjectPool, RegisterFile};

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
        id
    } else {
        if obj_pool.len() >= (*gc_array_threshold as usize) {
            *gc_array_threshold *= 2;
            array_gc(
                obj_pool,
                map_pool,
                free_arrays,
                registers,
                recursion_stack,
                gc,
            );
        }
        if let Some(id) = free_arrays.pop() {
            obj_pool[id as usize].clear();
            id
        } else {
            let id = obj_pool.len() as u32;
            obj_pool.push(Vec::new());
            id
        }
    }
}

fn array_gc(
    obj_pool: &ObjectPool,
    map_pool: &MapPool,
    free_arrays: &mut Vec<u32>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcScratch,
) {
    reset_marks(gc, obj_pool.len(), map_pool.len());

    // Find all used arrays
    for data in registers.0.iter().chain(recursion_stack.0.iter()) {
        if data.is_array() || data.is_struct() || data.is_enum() {
            track(*data, obj_pool, map_pool, gc);
        }
    }

    // Mark slots that are already free as live
    for &id in free_arrays.iter() {
        unsafe {
            *gc.array_live.get_unchecked_mut(id as usize) = true;
        }
    }

    // Mark as free any array that isn't referenced by a register
    for (i, array_alive) in gc.array_live.iter().enumerate() {
        if !array_alive {
            free_arrays.push(i as u32);
        }
    }
}

/// Clears the object marks and gives each pool a slot per entry.
///
/// Both pools are prepared whichever one is about to be swept: the trace
/// crosses between them, so it writes a mark for every array and every map it
/// reaches. Clearing matters as much as sizing, since a mark left over from an
/// earlier collection reads as "already visited" and stops the trace short of
/// objects that are still live.
pub fn reset_marks(gc: &mut GcScratch, arrays: usize, maps: usize) {
    gc.array_live.clear();
    gc.array_live.resize(arrays, false);
    gc.map_live.clear();
    gc.map_live.resize(maps, false);
}

pub fn track(root: Data, obj_pool: &ObjectPool, map_pool: &MapPool, gc: &mut GcScratch) {
    gc.work.push(root);
    while let Some(d) = gc.work.pop() {
        if d.is_map() {
            track_maps(d.as_map(), map_pool, obj_pool, gc);
            continue;
        }
        let is_live = unsafe { gc.array_live.get_unchecked_mut(d.as_array()) };
        if *is_live {
            continue;
        }
        *is_live = true;
        // Every element is tested on its own. A list typed `any`, which is what
        // a parsed json document is made of, mixes scalars and objects freely,
        // so the first element says nothing about the rest: reading it as a
        // verdict for the whole list either walks past a live object or hands
        // an int's payload to `as_array` as if it were a pool index.
        for e in &obj_pool[d.as_array()] {
            if e.is_array() || e.is_struct() || e.is_enum() || e.is_map() {
                gc.work.push(*e);
            }
        }
    }
}
