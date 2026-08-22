use crate::array_gc::reset_marks;
use crate::vm::GcScratch;
use crate::vm::MapPool;
use crate::vm::ObjectPool;
use crate::vm::RegisterFile;
use crate::vm::StringPool;

/// Frees every pooled string no live value can still reach.
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
    free_strings: &mut Vec<u16>,
    registers: &RegisterFile,
    recursion_stack: &RegisterFile,
    gc: &mut GcScratch,
) {
    gc.string_live.clear();
    gc.string_live.resize(string_pool.len(), false);
    reset_marks(gc, array_pool.len(), map_pool.len());

    for data in registers.0.iter().chain(recursion_stack.0.iter()) {
        gc.work.push(*data);
        track_strings(array_pool, map_pool, gc);
    }

    for &id in free_strings.iter() {
        gc.string_live[id as usize] = true;
    }

    for (i, s) in gc.string_live.iter().enumerate() {
        if !s {
            free_strings.push(i as u16);
        }
    }
}

/// Marks every pooled string the values on the work stack can reach.
fn track_strings(array_pool: &ObjectPool, map_pool: &MapPool, gc: &mut GcScratch) {
    // Everything is tagged, so any value at all can go on the stack and be
    // sorted out on the way off it. Deciding from one element what a whole
    // collection holds is what used to free strings a parsed document was
    // still holding.
    while let Some(d) = gc.work.pop() {
        if d.is_large_str() {
            gc.string_live[d.get_str_pool_id()] = true;
        } else if d.is_map() {
            let seen = &mut gc.map_live[d.as_map()];
            if *seen {
                continue;
            }
            *seen = true;
            for (k, v) in &map_pool[d.as_map()] {
                gc.work.push(*k);
                gc.work.push(*v);
            }
        } else if d.is_array() || d.is_struct() || d.is_enum() {
            let seen = &mut gc.array_live[d.as_array()];
            if *seen {
                continue;
            }
            *seen = true;
            gc.work.extend(&array_pool[d.as_array()]);
        }
    }
}

#[inline(always)]
pub fn raise_string_gc_threshold(gc_string_threshold: &mut u32, string_pool_len: usize) {
    *gc_string_threshold = string_pool_len.next_power_of_two().min(u32::MAX as usize) as u32;
}
