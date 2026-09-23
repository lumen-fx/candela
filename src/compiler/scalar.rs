//! Scalar replacement, a release-profile pass: a struct that is built, read
//! and passed around only between registers never needs its pooled object, so
//! each of its fields lives in a register of its own instead.
//!
//! A struct is a pooled object and a register holds a reference to it, so a
//! struct built in a loop costs an allocation per turn and work for the
//! collector. The pass finds the structs whose references never leave the
//! registers: every register one can reach through moves is written only by a
//! struct construction or a move from another such register, and read only
//! for a field, for a field write while the struct is still being built, or
//! by a move to another such register. Those registers become one register
//! per field. Anything else a program does with a reference (returning it,
//! storing it in a list, a map or another struct, passing it to a call,
//! printing it, comparing it) keeps the struct pooled exactly as before.
//!
//! A field write outside the construction would be visible through every
//! register sharing the reference, which separate field registers could not
//! reproduce, so a struct written after it was built is left alone.
//!
//! The rewrite changes how many instructions the program has, so the pass
//! then moves every jump, call target and function location to match.

use crate::compiler::flow::Program;
use crate::compiler::flow::branch_targets;
use crate::compiler::flow::ends_straight_run;
use crate::compiler::flow::is_call;
use crate::compiler::flow::relocate;
use crate::data::Data;
use crate::data::NULL;
use crate::instr::Instr;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// Replaces every struct the module documentation describes with one register
/// per field.
pub fn replace_scalars(program: &mut Program<'_>) {
    let webs = find_webs(program);
    if webs.is_empty() {
        return;
    }
    let mut fields: FxHashMap<u16, Box<[u16]>> = FxHashMap::default();
    for web in &webs {
        for &reg in &web.members {
            let initial = program.registers[reg as usize];
            let regs = (0..web.field_count)
                .map(|i| {
                    let value = if initial.is_struct() {
                        program.objs[initial.as_struct()][i]
                    } else {
                        NULL
                    };
                    program.registers.push(value);
                    (program.registers.len() - 1) as u16
                })
                .collect();
            fields.insert(reg, regs);
        }
    }

    let old = std::mem::take(program.instructions);
    let mut map: Vec<usize> = Vec::with_capacity(old.len() + 1);
    let mut new: Vec<Instr> = Vec::with_capacity(old.len());
    let mut replaced: Vec<bool> = Vec::with_capacity(old.len());
    for (pos, &instr) in old.iter().enumerate() {
        map.push(new.len());
        let rewrites = match instr {
            Instr::CloneStruct(_, reg)
            | Instr::SetFieldStruct(reg, _, _)
            | Instr::GetFieldStruct(reg, _, _)
            | Instr::Mov(reg, _) => fields.contains_key(&reg),
            _ => false,
        };
        replaced.push(rewrites);
        match instr {
            Instr::CloneStruct(template, reg) if fields.contains_key(&reg) => {
                let set_later = construction_writes(&old, pos, reg);
                let template = program.registers[template as usize];
                for (i, &field) in fields[&reg].iter().enumerate() {
                    if set_later.contains(&i) {
                        continue;
                    }
                    let value = program.objs[template.as_struct()][i];
                    let src = constant_register(program, value);
                    new.push(Instr::Mov(src, field));
                }
            }
            Instr::SetFieldStruct(reg, value, idx) if fields.contains_key(&reg) => {
                new.push(Instr::Mov(value, fields[&reg][idx as usize]));
            }
            Instr::GetFieldStruct(reg, idx, out) if fields.contains_key(&reg) => {
                new.push(Instr::Mov(fields[&reg][idx as usize], out));
            }
            Instr::Mov(from, to) if fields.contains_key(&from) => {
                for (&a, &b) in fields[&from].iter().zip(fields[&to].iter()) {
                    new.push(Instr::Mov(a, b));
                }
            }
            other => new.push(other),
        }
    }
    map.push(new.len());
    if new.len() > usize::from(u16::MAX) {
        // An instruction's position has to fit a u16; a program the rewrite
        // would push past that keeps its pooled structs.
        *program.instructions = old;
        program
            .registers
            .truncate(program.registers.len() - fields.values().map(|f| f.len()).sum::<usize>());
        return;
    }

    for list in program.callsite_registers.iter_mut() {
        if list.iter().any(|reg| fields.contains_key(reg)) {
            let mut expanded: Vec<u16> = Vec::with_capacity(list.len());
            for reg in list.iter() {
                match fields.get(reg) {
                    Some(regs) => expanded.extend_from_slice(regs),
                    None => expanded.push(*reg),
                }
            }
            expanded.sort_unstable();
            expanded.dedup();
            *list = expanded;
        }
    }

    relocate(program, &old, &replaced, &mut new, &map);
    *program.instructions = new;
}

/// A set of registers that share the references to structs of one type, and
/// how many fields that type has.
struct Web {
    members: Vec<u16>,
    field_count: usize,
}

/// The fields a construction at `pos` writes before anything reads the struct
/// it builds, which need no value from the template.
fn construction_writes(instructions: &[Instr], pos: usize, reg: u16) -> Vec<usize> {
    let mut written = Vec::new();
    for instr in &instructions[pos + 1..] {
        match *instr {
            Instr::SetFieldStruct(r, value, idx) if r == reg && value != reg => {
                written.push(idx as usize);
            }
            other => {
                let mut touches = false;
                other.for_each_read_reg(|r| touches |= r == reg);
                if touches
                    || other.get_tgt_id() == Some(reg)
                    || (ends_straight_run(other) || is_call(other))
                {
                    break;
                }
            }
        }
    }
    written
}

/// A register holding `value` that nothing writes, made if there is none.
fn constant_register(program: &mut Program<'_>, value: Data) -> u16 {
    if let Some(&reg) = program.const_registers.get(&value) {
        return reg;
    }
    program.registers.push(value);
    let reg = (program.registers.len() - 1) as u16;
    program.const_registers.insert(value, reg);
    reg
}

/// The webs of registers that can be replaced.
fn find_webs(program: &Program<'_>) -> Vec<Web> {
    let instructions: &[Instr] = program.instructions;
    // Registers a call site, the VM or a host reads or writes without an
    // instruction naming it here: parameters, the indirect-call registers,
    // function entries and the null sink.
    let mut excluded: FxHashSet<u16> = FxHashSet::default();
    excluded.insert(0);
    excluded.extend(program.indirect.args.iter().copied());
    excluded.extend(program.indirect.env);
    for function in program.functions.iter() {
        excluded.extend(function.entry_register);
        for fn_impl in &function.impls {
            excluded.extend(fn_impl.args_loc.iter().copied());
            excluded.extend(fn_impl.env_loc);
        }
    }

    let mut parent: FxHashMap<u16, u16> = FxHashMap::default();
    fn find(parent: &mut FxHashMap<u16, u16>, reg: u16) -> u16 {
        let up = *parent.entry(reg).or_insert(reg);
        if up == reg {
            return reg;
        }
        let root = find(parent, up);
        parent.insert(reg, root);
        root
    }
    let mut seeds: Vec<u16> = Vec::new();
    for instr in instructions {
        match *instr {
            Instr::Mov(a, b) => {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent.insert(ra, rb);
                }
            }
            Instr::CloneStruct(_, reg) => seeds.push(reg),
            _ => {}
        }
    }
    let mut components: FxHashMap<u16, Vec<u16>> = FxHashMap::default();
    let regs: Vec<u16> = parent
        .keys()
        .copied()
        .chain(seeds.iter().copied())
        .collect();
    for reg in regs {
        let root = find(&mut parent, reg);
        let members = components.entry(root).or_default();
        if !members.contains(&reg) {
            members.push(reg);
        }
    }
    let seed_roots: FxHashSet<u16> = seeds.iter().map(|&reg| find(&mut parent, reg)).collect();

    let lands = branch_targets(instructions);
    let mut webs = Vec::new();
    for root in seed_roots {
        let members = &components[&root];
        if let Some(field_count) = replaceable(program, members, &excluded, &lands) {
            webs.push(Web {
                members: members.clone(),
                field_count,
            });
        }
    }
    webs
}

/// The field count of the struct type a web holds, when every use of every
/// register in it is one the pass can replace.
fn replaceable(
    program: &Program<'_>,
    members: &[u16],
    excluded: &FxHashSet<u16>,
    lands: &[bool],
) -> Option<usize> {
    let instructions: &[Instr] = program.instructions;
    let in_web = |reg: u16| members.contains(&reg);
    if members.iter().any(|reg| excluded.contains(reg)) {
        return None;
    }
    let mut type_id: Option<u16> = None;
    let mut field_count = 0;
    let mut same_type = |value: Data| -> bool {
        if !value.is_struct() {
            return false;
        }
        let count = program.objs[value.as_struct()].len();
        match type_id {
            None => {
                type_id = Some(value.struct_type_id());
                field_count = count;
                true
            }
            Some(id) => id == value.struct_type_id() && count == field_count,
        }
    };
    for (pos, instr) in instructions.iter().enumerate() {
        match *instr {
            Instr::CloneStruct(template, reg) if in_web(reg) => {
                if !same_type(program.registers[template as usize]) {
                    return None;
                }
            }
            Instr::Mov(a, b) if in_web(a) || in_web(b) => {
                if !(in_web(a) && in_web(b)) {
                    return None;
                }
            }
            Instr::GetFieldStruct(reg, _, out) if in_web(reg) => {
                if in_web(out) {
                    return None;
                }
            }
            Instr::SetFieldStruct(reg, value, _) if in_web(reg) => {
                if in_web(value) || !built_here(instructions, lands, reg, pos) {
                    return None;
                }
            }
            other => {
                let mut touches = false;
                other.for_each_read_reg(|r| touches |= in_web(r));
                if touches || other.get_tgt_id().is_some_and(in_web) {
                    return None;
                }
            }
        }
    }
    for &reg in members {
        let initial = program.registers[reg as usize];
        if !initial.is_null() && !same_type(initial) {
            return None;
        }
    }
    type_id.map(|_| field_count)
}

/// Whether the field write at `pos` to `reg` belongs to the construction that
/// built the struct: walking back from `pos` along one straight run of
/// instructions, the first one that writes `reg` is a construction, and
/// nothing in between reads the struct except other field writes. The struct
/// is then still in the hands of the code building it, and no other register
/// holds it yet.
fn built_here(instructions: &[Instr], lands: &[bool], reg: u16, pos: usize) -> bool {
    if lands[pos] {
        return false;
    }
    let mut at = pos;
    while at > 0 {
        at -= 1;
        let instr = instructions[at];
        if let Instr::CloneStruct(_, r) = instr
            && r == reg
        {
            return true;
        }
        if lands[at]
            || (ends_straight_run(instr) || is_call(instr))
            || instr.get_tgt_id() == Some(reg)
        {
            return false;
        }
        if !matches!(instr, Instr::SetFieldStruct(r, _, _) if r == reg) {
            let mut touches = false;
            instr.for_each_read_reg(|r| touches |= r == reg);
            if touches {
                return false;
            }
        }
    }
    false
}
