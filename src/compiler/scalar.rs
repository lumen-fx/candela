//! Scalar replacement, a release-profile pass: a struct that is built, read
//! and passed around only between registers never needs its pooled object, so
//! each of its fields lives in a register of its own instead.
//!
//! A struct is a pooled object and a register holds a reference to it, so a
//! struct built in a loop costs an allocation per turn and work for the
//! collector. The pass finds the structs whose references never leave the
//! registers: every register one can reach through moves is written only by a
//! struct construction or a move from another such register, and read only
//! for a field, for a field write, for `print`, or by a move to another such
//! register. Those registers become one register per field. Anything else a
//! program does with a reference (returning it, storing it in a list or a
//! map, passing it to a call, comparing it) keeps the struct pooled exactly
//! as before. `type()` names the type the compiler inferred and never reads
//! the value, so it keeps nothing pooled.
//!
//! A struct stored in a field of another is replaced too once the outer one
//! is: the field is then a register, and a move into or out of it is one more
//! move of the inner struct. The pass runs again until it finds nothing more.
//!
//! A field write is visible through every register sharing the reference,
//! which separate field registers cannot reproduce by themselves. A write
//! while the struct is still being built is safe, since no other register
//! holds it yet. A later one is safe when no other register of the struct's
//! web is still read afterwards: the copies the other registers hold are then
//! never looked at again. A struct written any other way is left alone.
//!
//! `print` shows the fields, so a struct kept in registers is built back into
//! a pooled object right where it is printed, and only there.
//!
//! The rewrite changes how many instructions the program has, so the pass
//! then moves every jump, call target and function location to match.

use crate::compiler::flow::Program;
use crate::compiler::flow::branch_targets;
use crate::compiler::flow::ends_straight_run;
use crate::compiler::flow::is_call;
use crate::compiler::flow::relocate;
use crate::compiler::flow::successors;
use crate::data::Data;
use crate::data::NULL;
use crate::instr::Instr;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// Replaces every struct the module documentation describes with one register
/// per field, and answers whether it found any.
pub fn replace_scalars(program: &mut Program<'_>) -> bool {
    let webs = find_webs(program);
    if webs.is_empty() {
        return false;
    }
    // Every register has to fit an operand; a program the rewrite would push
    // past that keeps its pooled structs. A web takes a register per field
    // for each of its registers, one per print, and at most one per field of
    // each construction for the template's values.
    let mut field_count_of: FxHashMap<u16, usize> = FxHashMap::default();
    let mut needed = 0;
    for web in &webs {
        needed += web.members.len() * web.field_count;
        for &reg in &web.members {
            field_count_of.insert(reg, web.field_count);
        }
    }
    for instr in program.instructions.iter() {
        match *instr {
            Instr::Print(reg) if field_count_of.contains_key(&reg) => needed += 1,
            Instr::CloneStruct(_, reg) => needed += field_count_of.get(&reg).copied().unwrap_or(0),
            _ => {}
        }
    }
    if program.registers.len() + needed > usize::from(u16::MAX) {
        return false;
    }
    let base = program.registers.len();
    let mut fields: FxHashMap<u16, Box<[u16]>> = FxHashMap::default();
    let mut templates: FxHashMap<u16, u16> = FxHashMap::default();
    for web in &webs {
        for &reg in &web.members {
            templates.insert(reg, web.template);
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
    let mut written: FxHashSet<u16> = FxHashSet::default();
    for instr in &old {
        instr.for_each_write_reg(|reg| {
            written.insert(reg);
        });
    }
    let mut map: Vec<usize> = Vec::with_capacity(old.len() + 1);
    let mut new: Vec<Instr> = Vec::with_capacity(old.len());
    let mut replaced: Vec<bool> = Vec::with_capacity(old.len());
    for (pos, &instr) in old.iter().enumerate() {
        map.push(new.len());
        let rewrites = match instr {
            Instr::CloneStruct(_, reg)
            | Instr::SetFieldStruct(reg, _, _)
            | Instr::AddFieldFloat(reg, _, _)
            | Instr::GetFieldStruct(reg, _, _)
            | Instr::Print(reg)
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
                    // The count checked above leaves room for these.
                    if let Some(src) = program.constant_register(value, &written) {
                        new.push(Instr::Mov(src, field));
                    }
                }
            }
            Instr::SetFieldStruct(reg, value, idx) if fields.contains_key(&reg) => {
                new.push(Instr::Mov(value, fields[&reg][idx as usize]));
            }
            Instr::AddFieldFloat(reg, value, idx) if fields.contains_key(&reg) => {
                let field = fields[&reg][idx as usize];
                new.push(Instr::AddFloat(field, value, field));
            }
            Instr::GetFieldStruct(reg, idx, out) if fields.contains_key(&reg) => {
                new.push(Instr::Mov(fields[&reg][idx as usize], out));
            }
            // A pooled copy of the struct to print, built from its fields.
            Instr::Print(reg) if fields.contains_key(&reg) => {
                let template = templates[&reg];
                program.registers.push(program.registers[template as usize]);
                let built = (program.registers.len() - 1) as u16;
                new.push(Instr::CloneStruct(template, built));
                for (i, &field) in fields[&reg].iter().enumerate() {
                    new.push(Instr::SetFieldStruct(built, field, i as u16));
                }
                new.push(Instr::Print(built));
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
        program.registers.truncate(base);
        program
            .const_registers
            .retain(|_, reg| usize::from(*reg) < base);
        return false;
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
    true
}

/// A set of registers that share the references to structs of one type, how
/// many fields that type has, and a register holding a struct of the type to
/// build a pooled copy from.
struct Web {
    members: Vec<u16>,
    field_count: usize,
    template: u16,
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

/// The webs of registers that can be replaced.
fn find_webs(program: &Program<'_>) -> Vec<Web> {
    let instructions: &[Instr] = program.instructions;
    // Registers a call site, the VM or a host reads or writes without an
    // instruction naming it here, and the ones a call saves and puts back.
    let excluded = program.outside_registers();
    let saved: FxHashSet<u16> = program
        .callsite_registers
        .iter()
        .flat_map(|list| list.iter().copied())
        .collect();

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

    // Every web is judged in one walk over the program: an instruction is
    // looked at for each web whose registers it names.
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut web_of: FxHashMap<u16, usize> = FxHashMap::default();
    for root in seed_roots {
        let members = components[&root].clone();
        let ok = !members.iter().any(|reg| excluded.contains(reg));
        for &reg in &members {
            web_of.insert(reg, candidates.len());
        }
        candidates.push(Candidate {
            members,
            ok,
            type_id: None,
            field_count: 0,
            template: None,
            worked_on: false,
            late: Vec::new(),
        });
    }
    let lands = branch_targets(instructions);
    let mut touched: Vec<usize> = Vec::new();
    for (pos, &instr) in instructions.iter().enumerate() {
        touched.clear();
        let mut note = |reg: u16| {
            if let Some(&web) = web_of.get(&reg)
                && !touched.contains(&web)
            {
                touched.push(web);
            }
        };
        instr.for_each_read_reg(&mut note);
        instr.for_each_write_reg(&mut note);
        for &web in &touched {
            if candidates[web].ok {
                let in_web = |reg: u16| web_of.get(&reg) == Some(&web);
                let fits = judge(program, &mut candidates[web], instr, pos, &in_web, &lands);
                candidates[web].ok = fits;
            }
        }
    }
    for candidate in &mut candidates {
        if candidate.ok {
            for i in 0..candidate.members.len() {
                let initial = program.registers[candidate.members[i] as usize];
                if !initial.is_null() && !candidate.same_type(program, initial) {
                    candidate.ok = false;
                    break;
                }
            }
        }
        candidate.ok &= candidate.worked_on && candidate.template.is_some();
    }
    writes_unshared(program, &saved, &mut candidates);

    candidates
        .into_iter()
        .filter(|candidate| candidate.ok)
        .map(|candidate| Web {
            members: candidate.members,
            field_count: candidate.field_count,
            template: candidate.template.unwrap_or_default(),
        })
        .collect()
}

/// A web of registers being judged, and what the walk over the program has
/// found about it so far.
struct Candidate {
    members: Vec<u16>,
    /// Whether every use seen so far is one the pass can replace.
    ok: bool,
    type_id: Option<u16>,
    field_count: usize,
    template: Option<u16>,
    /// Whether the struct is used for anything but being built and printed:
    /// one that is only printed gains nothing from its fields in registers,
    /// and the pooled copy a print builds is such a struct.
    worked_on: bool,
    /// Field writes made after the struct was built, as `(position, register
    /// written through)`.
    late: Vec<(usize, u16)>,
}

impl Candidate {
    /// Whether `value` is a struct of the one type every register of the web
    /// holds, which the first struct seen decides.
    fn same_type(&mut self, program: &Program<'_>, value: Data) -> bool {
        if !value.is_struct() {
            return false;
        }
        let count = program.objs[value.as_struct()].len();
        match self.type_id {
            None => {
                self.type_id = Some(value.struct_type_id());
                self.field_count = count;
                true
            }
            Some(id) => id == value.struct_type_id() && count == self.field_count,
        }
    }
}

/// Whether `instr`, at `pos` and naming a register of the web `in_web`
/// recognises, is a use the pass can replace, noting what it tells about the
/// web on the way.
fn judge(
    program: &Program<'_>,
    web: &mut Candidate,
    instr: Instr,
    pos: usize,
    in_web: &impl Fn(u16) -> bool,
    lands: &[bool],
) -> bool {
    let instructions: &[Instr] = program.instructions;
    match instr {
        Instr::CloneStruct(from, reg) if in_web(reg) => {
            web.template = Some(from);
            !in_web(from) && web.same_type(program, program.registers[from as usize])
        }
        Instr::Mov(a, b) => {
            web.worked_on = true;
            in_web(a) && in_web(b)
        }
        Instr::GetFieldStruct(reg, _, out) if in_web(reg) => {
            web.worked_on = true;
            !in_web(out)
        }
        Instr::SetFieldStruct(reg, value, _) if in_web(reg) => {
            if !built_here(instructions, lands, reg, pos) {
                web.late.push((pos, reg));
                web.worked_on = true;
            }
            !in_web(value)
        }
        Instr::AddFieldFloat(reg, value, _) if in_web(reg) => {
            web.late.push((pos, reg));
            web.worked_on = true;
            !in_web(value)
        }
        Instr::Print(reg) if in_web(reg) => true,
        _ => false,
    }
}

/// Keeps a web whose fields are written after it was built only when no
/// register of the web but the one written through is read after any of
/// those writes, so the copies of the fields the others hold are never looked
/// at again.
///
/// Worked out as the registers of those webs live after each instruction, all
/// webs at once. A throw can reach a catch from anywhere, so a register a
/// catch reads counts as live everywhere. A recursive call saves registers
/// and puts them back, which liveness here does not follow, so a web one of
/// whose registers a call saves is not kept.
fn writes_unshared(program: &Program<'_>, saved: &FxHashSet<u16>, candidates: &mut [Candidate]) {
    let mut bit_of: FxHashMap<u16, usize> = FxHashMap::default();
    for candidate in candidates.iter_mut() {
        if !candidate.ok || candidate.late.is_empty() {
            continue;
        }
        if candidate.members.iter().any(|reg| saved.contains(reg)) {
            candidate.ok = false;
            continue;
        }
        for &reg in &candidate.members {
            let bit = bit_of.len();
            bit_of.insert(reg, bit);
        }
    }
    if bit_of.is_empty() {
        return;
    }
    let words = bit_of.len().div_ceil(64);
    let instructions: &[Instr] = program.instructions;
    let len = instructions.len();
    let set = |sets: &mut [u64], at: usize, reg: u16| {
        if let Some(&bit) = bit_of.get(&reg) {
            sets[at * words + bit / 64] |= 1 << (bit % 64);
        }
    };
    let mut reads = vec![0_u64; len * words];
    let mut writes = vec![0_u64; len * words];
    for (pos, instr) in instructions.iter().enumerate() {
        instr.for_each_read_reg(|reg| set(&mut reads, pos, reg));
        instr.for_each_write_reg(|reg| set(&mut writes, pos, reg));
    }
    let mut live_in = vec![0_u64; (len + 1) * words];
    let mut live_out = vec![0_u64; len * words];
    let row = |pos: usize| pos * words..(pos + 1) * words;
    let mut out = vec![0_u64; words];
    let mut changed = true;
    while changed {
        changed = false;
        for pos in (0..len).rev() {
            out.fill(0);
            for next in successors(pos, instructions[pos]).into_iter().flatten() {
                if next <= len {
                    for (word, &live) in out.iter_mut().zip(&live_in[row(next)]) {
                        *word |= live;
                    }
                }
            }
            for (w, &word) in out.iter().enumerate() {
                let at = pos * words + w;
                let into = reads[at] | (word & !writes[at]);
                if word != live_out[at] || into != live_in[at] {
                    live_out[at] = word;
                    live_in[at] = into;
                    changed = true;
                }
            }
        }
    }
    let mut caught = vec![0_u64; words];
    for (pos, instr) in instructions.iter().enumerate() {
        if let Instr::StartErrorCatch(size, _) = *instr {
            let at = pos + size as usize;
            if at <= len {
                for (word, &live) in caught.iter_mut().zip(&live_in[row(at)]) {
                    *word |= live;
                }
            }
        }
    }
    for candidate in candidates.iter_mut() {
        if !candidate.ok || candidate.late.is_empty() {
            continue;
        }
        let shared = candidate.late.iter().any(|&(pos, written)| {
            candidate.members.iter().any(|&reg| {
                let bit = bit_of[&reg];
                let mask = 1 << (bit % 64);
                reg != written && (live_out[pos * words + bit / 64] | caught[bit / 64]) & mask != 0
            })
        });
        if shared {
            candidate.ok = false;
        }
    }
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
