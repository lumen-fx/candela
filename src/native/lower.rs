//! Lowers a finished program's bytecode to the [`ir`](super::ir), reading
//! types from the compiler's function table.
//!
//! The bytecode keeps every value in one shared register file. Machine code
//! keeps a function's values in variables of its own instead, so the
//! lowering has to know which registers a function reads on the way in (its
//! parameters), which values a caller still reads after a call its callee
//! could have overwritten, and which reuse of a register holds a value of
//! another type. The first two come from liveness over the whole program;
//! the last from splitting each register into webs of the writes that reach
//! a common read, each web typed by the table.
//!
//! A function qualifies when every instruction of it is one the lowering
//! handles. It calls a function that does not qualify through the
//! interpreter, and the interpreter calls it the same way the other way
//! round, so one that does not qualify costs its callers nothing but the
//! trip.

use super::ir::Block;
use super::ir::BlockId;
use super::ir::Call;
use super::ir::CallVm;
use super::ir::Cmp;
use super::ir::Cond;
use super::ir::FloatOp;
use super::ir::Func;
use super::ir::FuncId;
use super::ir::IntOp;
use super::ir::Kind;
use super::ir::Module;
use super::ir::Op;
use super::ir::Operand;
use super::ir::Term;
use super::ir::UnaryOp;
use super::ir::Var;
use crate::compiler::flow::body;
use crate::compiler::flow::successors;
use candela_vm::artifact::FunctionTable;
use candela_vm::artifact::ProgramImage;
use candela_vm::data::Data;
use candela_vm::instr::Instr;
use candela_vm::instr::LibFunc;
use candela_vm::rt::DataType;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

type RegSet = FxHashSet<u16>;

/// Lowers every function of `image` that qualifies. `table` is the function
/// table the compiler wrote for it.
#[must_use]
pub fn lower(image: &ProgramImage, table: &FunctionTable) -> Module {
    let analysis = Analysis::new(image, table);
    analysis.module()
}

/// How one call instruction calls.
#[derive(Clone, Debug)]
enum CallKind {
    /// `CallFunc`: the callee keeps clear of every register the caller still
    /// reads.
    Direct { callee: usize },
    /// `CallFuncRecursive`, whose `SaveFrame` at `save` keeps `saved`.
    Recursive {
        callee: usize,
        save: usize,
        saved: Vec<u16>,
    },
    /// `CallIndirect`, through a function value, whose `SaveFrame` keeps
    /// `saved`.
    Indirect { saved: Vec<u16> },
}

#[derive(Clone, Debug)]
struct CallSite {
    at: usize,
    kind: CallKind,
    /// The register the result lands in.
    ret: u16,
}

/// A stretch of code with one entry: a compiled function, or a trampoline a
/// host enters through.
struct Unit {
    entry: usize,
    body: Vec<usize>,
    calls: Vec<CallSite>,
    /// The registers its own instructions write.
    writes: RegSet,
    /// Its entry in the function table, when it has one.
    table: Option<usize>,
    /// Whether every call in it could be matched with its frame.
    analyzable: bool,
    /// The registers it reads before writing them: its parameters.
    live_in: RegSet,
    /// The registers read after each instruction, before being written.
    live_out: FxHashMap<usize, RegSet>,
    /// The registers a catch in it reads, which any call could jump to.
    catch_live: RegSet,
}

struct Analysis<'a> {
    image: &'a ProgramImage,
    instrs: &'a [Instr],
    table: &'a FunctionTable,
    /// The registers no instruction writes.
    constant: Vec<bool>,
    units: Vec<Unit>,
    unit_at: FxHashMap<usize, usize>,
    /// The units a call through a function value can reach.
    indirect: Vec<usize>,
    /// The registers each unit writes, its callees' included.
    all_writes: Vec<RegSet>,
    /// Registers some caller reads after a call that could have written
    /// them. Native code keeps them in the register file as well as in its
    /// variables.
    observed: RegSet,
    /// The type of each value an instruction writes.
    def_types: FxHashMap<(u32, u16), &'a DataType>,
}

impl<'a> Analysis<'a> {
    fn new(image: &'a ProgramImage, table: &'a FunctionTable) -> Self {
        let instrs = &image.instructions[..];
        let mut constant = vec![true; image.registers.len()];
        let mut written = |reg: u16| {
            if let Some(slot) = constant.get_mut(reg as usize) {
                *slot = false;
            }
        };
        for instr in instrs {
            instr.for_each_write_reg(&mut written);
        }
        for export in &image.exports {
            for reg in &export.arg_registers {
                written(*reg);
            }
        }
        // A caller writes a function's parameters, even one every call site
        // inlined.
        for f in &table.functions {
            for reg in f.params.iter().chain(&f.env) {
                written(*reg);
            }
        }
        let mut def_types = FxHashMap::default();
        for f in &table.functions {
            for d in &f.defs {
                def_types.insert((d.at, d.reg), &d.ty);
            }
        }
        let mut analysis = Self {
            image,
            instrs,
            table,
            constant,
            units: Vec::new(),
            unit_at: FxHashMap::default(),
            indirect: Vec::new(),
            all_writes: Vec::new(),
            observed: RegSet::default(),
            def_types,
        };
        for (k, f) in table.functions.iter().enumerate() {
            analysis.add_unit(f.entry as usize, Some(k));
        }
        for export in &image.exports {
            analysis.add_unit(export.entry as usize, None);
        }
        // A call to code the table does not list still gets analysed, so
        // the registers it writes are known.
        let mut next = 0;
        while next < analysis.units.len() {
            let targets: Vec<usize> = analysis.units[next]
                .calls
                .iter()
                .filter_map(|c| match c.kind {
                    CallKind::Direct { callee } | CallKind::Recursive { callee, .. } => {
                        Some(callee)
                    }
                    CallKind::Indirect { .. } => None,
                })
                .collect();
            for target in targets {
                analysis.add_unit(target, None);
            }
            next += 1;
        }
        analysis.indirect = table
            .functions
            .iter()
            .filter(|f| f.indirect)
            .filter_map(|f| analysis.unit_at.get(&(f.entry as usize)).copied())
            .collect();
        analysis.writes_through_calls();
        analysis.liveness();
        analysis.find_observed();
        analysis
    }

    /// Adds the unit starting at `entry`, unless there is one.
    fn add_unit(&mut self, entry: usize, table: Option<usize>) {
        if entry >= self.instrs.len() {
            return;
        }
        if let Some(&u) = self.unit_at.get(&entry) {
            if self.units[u].table.is_none() {
                self.units[u].table = table;
            }
            return;
        }
        let body = body(self.instrs, entry);
        let mut calls = Vec::new();
        let mut writes = RegSet::default();
        let mut analyzable = true;
        for (k, &pos) in body.iter().enumerate() {
            let instr = self.instrs[pos];
            if !matches!(instr, Instr::SaveFrame(..)) {
                instr.for_each_write_reg(|reg| {
                    writes.insert(reg);
                });
            }
            let frame = || {
                body[..k].iter().rev().find_map(|&j| match self.instrs[j] {
                    Instr::SaveFrame(offset, ret, callsite) if j + offset as usize == pos => {
                        Some((j, ret, callsite))
                    }
                    _ => None,
                })
            };
            match instr {
                Instr::CallFunc(loc, ret) => calls.push(CallSite {
                    at: pos,
                    kind: CallKind::Direct {
                        callee: loc as usize,
                    },
                    ret,
                }),
                Instr::CallFuncRecursive(loc, ret) => {
                    let kind = if let Some((save, _, callsite)) = frame() {
                        CallKind::Recursive {
                            callee: loc as usize,
                            save,
                            saved: self.saved(callsite),
                        }
                    } else {
                        analyzable = false;
                        CallKind::Direct {
                            callee: loc as usize,
                        }
                    };
                    calls.push(CallSite { at: pos, kind, ret });
                }
                Instr::CallIndirect(_) => match frame() {
                    Some((_, ret, callsite)) => {
                        writes.insert(ret);
                        calls.push(CallSite {
                            at: pos,
                            kind: CallKind::Indirect {
                                saved: self.saved(callsite),
                            },
                            ret,
                        });
                    }
                    None => analyzable = false,
                },
                _ => {}
            }
        }
        self.unit_at.insert(entry, self.units.len());
        self.units.push(Unit {
            entry,
            body,
            calls,
            writes,
            table,
            analyzable,
            live_in: RegSet::default(),
            live_out: FxHashMap::default(),
            catch_live: RegSet::default(),
        });
    }

    fn saved(&self, callsite: u16) -> Vec<u16> {
        self.image
            .callsite_registers
            .get(callsite as usize)
            .cloned()
            .unwrap_or_default()
    }

    fn is_constant(&self, reg: u16) -> bool {
        self.constant.get(reg as usize).copied().unwrap_or(false)
    }

    /// The units a call can reach.
    fn callees(&self, kind: &CallKind) -> Vec<usize> {
        match kind {
            CallKind::Direct { callee } | CallKind::Recursive { callee, .. } => {
                self.unit_at.get(callee).copied().into_iter().collect()
            }
            CallKind::Indirect { .. } => self.indirect.clone(),
        }
    }

    /// Whether the unit starting at `entry` hands back a value.
    fn returns_value(&self, entry: usize) -> bool {
        self.unit_at
            .get(&entry)
            .and_then(|u| self.units[*u].table)
            .is_none_or(|k| self.table.functions[k].returns != DataType::Null)
    }

    fn writes_through_calls(&mut self) {
        self.all_writes = self.units.iter().map(|u| u.writes.clone()).collect();
        loop {
            let mut changed = false;
            for u in 0..self.units.len() {
                let callees: Vec<usize> = self.units[u]
                    .calls
                    .iter()
                    .flat_map(|c| self.callees(&c.kind))
                    .collect();
                for callee in callees {
                    if callee == u {
                        continue;
                    }
                    let theirs: Vec<u16> = self.all_writes[callee].iter().copied().collect();
                    for reg in theirs {
                        changed |= self.all_writes[u].insert(reg);
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// What instruction `pos` of a unit reads and writes, for liveness: a
    /// call reads what its callee reads on the way in.
    fn uses_defs(&self, pos: usize, call: Option<&CallSite>) -> (Vec<u16>, Vec<u16>) {
        let instr = self.instrs[pos];
        let mut uses = Vec::new();
        let mut defs = Vec::new();
        if let Some(call) = call {
            for callee in self.callees(&call.kind) {
                uses.extend(self.units[callee].live_in.iter().copied());
            }
            match &call.kind {
                CallKind::Direct { callee } => {
                    if self.returns_value(*callee) {
                        defs.push(call.ret);
                    }
                }
                CallKind::Recursive { callee, saved, .. } => {
                    if self.returns_value(*callee) {
                        defs.push(call.ret);
                    }
                    defs.extend(saved.iter().copied());
                }
                CallKind::Indirect { saved, .. } => {
                    instr.for_each_read_reg(|reg| uses.push(reg));
                    defs.push(call.ret);
                    defs.extend(saved.iter().copied());
                }
            }
        } else if let Instr::SaveFrame(_, _, callsite) = instr {
            // It keeps these, and the call's return puts them back.
            uses.extend(self.saved(callsite));
        } else {
            instr.for_each_read_reg(|reg| uses.push(reg));
            instr.for_each_write_reg(|reg| defs.push(reg));
        }
        uses.retain(|reg| !self.is_constant(*reg));
        (uses, defs)
    }

    /// Liveness in every unit, repeated until what each unit reads on entry
    /// settles, since a call reads what its callee does.
    fn liveness(&mut self) {
        loop {
            let mut changed = false;
            for u in 0..self.units.len() {
                let (live_in, live_out, catch_live) = self.unit_liveness(u);
                if live_in != self.units[u].live_in {
                    changed = true;
                }
                let unit = &mut self.units[u];
                unit.live_in = live_in;
                unit.live_out = live_out;
                unit.catch_live = catch_live;
            }
            if !changed {
                break;
            }
        }
    }

    fn unit_liveness(&self, u: usize) -> (RegSet, FxHashMap<usize, RegSet>, RegSet) {
        let unit = &self.units[u];
        let call_at: FxHashMap<usize, &CallSite> = unit.calls.iter().map(|c| (c.at, c)).collect();
        let effects: Vec<(Vec<u16>, Vec<u16>)> = unit
            .body
            .iter()
            .map(|&pos| self.uses_defs(pos, call_at.get(&pos).copied()))
            .collect();
        let index: FxHashMap<usize, usize> = unit
            .body
            .iter()
            .enumerate()
            .map(|(k, &pos)| (pos, k))
            .collect();
        let mut live_in: Vec<RegSet> = vec![RegSet::default(); unit.body.len()];
        let mut live_out: Vec<RegSet> = vec![RegSet::default(); unit.body.len()];
        loop {
            let mut changed = false;
            for k in (0..unit.body.len()).rev() {
                let pos = unit.body[k];
                let mut out = RegSet::default();
                for next in successors(pos, self.instrs[pos]).into_iter().flatten() {
                    if let Some(&j) = index.get(&next) {
                        out.extend(live_in[j].iter().copied());
                    }
                }
                let (uses, defs) = &effects[k];
                let mut inn: RegSet = out.iter().copied().filter(|r| !defs.contains(r)).collect();
                inn.extend(uses.iter().copied());
                if inn != live_in[k] {
                    live_in[k] = inn;
                    changed = true;
                }
                live_out[k] = out;
            }
            if !changed {
                break;
            }
        }
        let mut catch_live = RegSet::default();
        for &pos in &unit.body {
            if let Instr::StartErrorCatch(size, _) = self.instrs[pos]
                && let Some(&j) = index.get(&(pos + size as usize))
            {
                catch_live.extend(live_in[j].iter().copied());
            }
        }
        let entry_in = index
            .get(&unit.entry)
            .map(|&k| live_in[k].clone())
            .unwrap_or_default();
        let out_map = unit
            .body
            .iter()
            .enumerate()
            .map(|(k, &pos)| (pos, std::mem::take(&mut live_out[k])))
            .collect();
        (entry_in, out_map, catch_live)
    }

    /// The registers a call may overwrite that its caller has not kept:
    /// everything its callees write, but the result and what `SaveFrame`
    /// puts back.
    fn clobbered(&self, call: &CallSite) -> RegSet {
        let mut regs = RegSet::default();
        for callee in self.callees(&call.kind) {
            regs.extend(self.all_writes[callee].iter().copied());
        }
        regs.remove(&call.ret);
        if let CallKind::Recursive { saved, .. } | CallKind::Indirect { saved, .. } = &call.kind {
            for reg in saved {
                regs.remove(reg);
            }
        }
        regs
    }

    /// The registers some caller reads after a call that may have written
    /// them.
    fn read_after(&self, unit: &Unit, call: &CallSite) -> RegSet {
        let clobbered = self.clobbered(call);
        let empty = RegSet::default();
        let live = unit.live_out.get(&call.at).unwrap_or(&empty);
        clobbered
            .into_iter()
            .filter(|r| live.contains(r) || unit.catch_live.contains(r))
            .collect()
    }

    fn find_observed(&mut self) {
        let mut observed = RegSet::default();
        for unit in &self.units {
            for call in &unit.calls {
                observed.extend(self.read_after(unit, call));
            }
        }
        self.observed = observed;
    }

    /// Every function that qualifies, lowered.
    fn module(&self) -> Module {
        let mut native: Vec<usize> = (0..self.units.len())
            .filter(|&u| self.units[u].table.is_some() && self.units[u].analyzable)
            .filter(|&u| self.qualifies(u))
            .collect();
        // `main` runs once. As machine code it only pays off with a loop of
        // its own or a native function to call; otherwise the calls it makes
        // into the interpreter run there nested, which is slower than the
        // interpreter running `main` itself.
        let worth = |u: usize, native: &[usize]| {
            let unit = &self.units[u];
            unit.entry != 0
                || unit.body.iter().any(|&pos| {
                    matches!(
                        self.instrs[pos],
                        Instr::JmpBack(_) | Instr::InfIntJmpBack(..) | Instr::StepIntJmpBack(..)
                    )
                })
                || unit.calls.iter().any(|call| {
                    self.callees(&call.kind)
                        .iter()
                        .any(|callee| native.contains(callee))
                })
        };
        let candidates = native.clone();
        native.retain(|&u| worth(u, &candidates));
        // Lowering a function can still refuse it, which turns its native
        // callers' calls into calls through the interpreter, so repeat until
        // the set settles.
        loop {
            let ids: FxHashMap<usize, FuncId> = native
                .iter()
                .enumerate()
                .map(|(k, &u)| (u, k as FuncId))
                .collect();
            let mut funcs = Vec::with_capacity(native.len());
            let mut refused = Vec::new();
            for &u in &native {
                match Lowering::new(self, u, &ids).run() {
                    Some(f) => funcs.push(f),
                    None => refused.push(u),
                }
            }
            if refused.is_empty() {
                let main = native
                    .iter()
                    .position(|&u| self.units[u].entry == 0)
                    .map(|k| k as FuncId);
                let mut sites = Vec::new();
                for unit in &self.units {
                    for call in &unit.calls {
                        if let CallKind::Direct { callee } | CallKind::Recursive { callee, .. } =
                            call.kind
                            && let Some(u) = self.unit_at.get(&callee)
                            && let Some(&id) = ids.get(u)
                        {
                            sites.push((call.at as u32, id));
                        }
                    }
                }
                sites.sort_unstable();
                sites.dedup_by_key(|(at, _)| *at);
                let mut module = Module { funcs, main, sites };
                super::inline::inline_early_returns(&mut module);
                for func in &mut module.funcs {
                    root_calls(func);
                }
                return module;
            }
            native.retain(|u| !refused.contains(u));
        }
    }

    /// Whether every instruction of unit `u` is one the lowering handles.
    fn qualifies(&self, u: usize) -> bool {
        let unit = &self.units[u];
        let is_main = unit.entry == 0;
        let mut returns = false;
        let mut void = false;
        for &pos in &unit.body {
            let instr = self.instrs[pos];
            if !supported(instr, is_main) {
                return false;
            }
            match instr {
                Instr::Return(_) | Instr::RecursiveReturn(_) => returns = true,
                Instr::VoidReturn => void = true,
                Instr::CallFunc(loc, _) | Instr::CallFuncRecursive(loc, _)
                    if !self.unit_at.contains_key(&(loc as usize)) =>
                {
                    return false;
                }
                _ => {}
            }
        }
        !(returns && void) && !(is_main && (returns || void))
    }
}

/// Whether the lowering handles `instr`.
const fn supported(instr: Instr, is_main: bool) -> bool {
    match instr {
        Instr::Print(_)
        | Instr::Jmp(_)
        | Instr::JmpBack(_)
        | Instr::IsFalseJmp(..)
        | Instr::IsTrueJmp(..)
        | Instr::SupEqFloatJmp(..)
        | Instr::SupEqIntJmp(..)
        | Instr::SupFloatJmp(..)
        | Instr::SupIntJmp(..)
        | Instr::InfEqFloatJmp(..)
        | Instr::InfEqIntJmp(..)
        | Instr::InfFloatJmp(..)
        | Instr::InfIntJmp(..)
        | Instr::InfIntJmpBack(..)
        | Instr::StepIntJmpBack(..)
        | Instr::NotEqJmp(..)
        | Instr::EqJmp(..)
        | Instr::Mov(..)
        | Instr::MovChain(..)
        | Instr::SetInt(..)
        | Instr::SetBool(..)
        | Instr::AddFloat(..)
        | Instr::AddInt(..)
        | Instr::AddIntImm(..)
        | Instr::MulFloat(..)
        | Instr::MulInt(..)
        | Instr::SubFloat(..)
        | Instr::SubInt(..)
        | Instr::DivFloat(..)
        | Instr::BitAndInt(..)
        | Instr::BitOrInt(..)
        | Instr::BitXorInt(..)
        | Instr::BitNotInt(..)
        | Instr::ShlInt(..)
        | Instr::ShrInt(..)
        | Instr::IncInt(_)
        | Instr::DecInt(_)
        | Instr::IncIntTo(..)
        | Instr::DecIntTo(..)
        | Instr::Eq(..)
        | Instr::NotEq(..)
        | Instr::SupFloat(..)
        | Instr::SupInt(..)
        | Instr::SupEqFloat(..)
        | Instr::SupEqInt(..)
        | Instr::InfFloat(..)
        | Instr::InfInt(..)
        | Instr::InfEqFloat(..)
        | Instr::InfEqInt(..)
        | Instr::BoolAnd(..)
        | Instr::BoolOr(..)
        | Instr::NegBool(..)
        | Instr::NegFloat(..)
        | Instr::NegInt(..)
        | Instr::CallFunc(..)
        | Instr::CallFuncRecursive(..)
        | Instr::VoidReturn
        | Instr::Return(_)
        | Instr::RecursiveReturn(_)
        | Instr::SaveFrame(..)
        | Instr::StoreFuncArg(_)
        | Instr::CallDynamicLibFunc(..)
        | Instr::CallLibFunc(
            LibFunc::SqrtFloat | LibFunc::Float | LibFunc::Int | LibFunc::Abs | LibFunc::Len,
            ..,
        )
        | Instr::ObjElemMov(..)
        | Instr::SetElementObj(..)
        | Instr::SetFieldStruct(..)
        | Instr::AddFieldFloat(..)
        | Instr::GetIndexArray(..)
        | Instr::GetFieldStruct(..)
        | Instr::LoadCell(..)
        | Instr::StoreCell(..) => true,
        Instr::Halt(0) => is_main,
        _ => false,
    }
}

/// The machine kind a value of type `ty` is held as.
const fn kind_of(ty: &DataType) -> Kind {
    match ty {
        DataType::Int => Kind::Int,
        DataType::Float => Kind::Float,
        DataType::Bool => Kind::Bool,
        _ => Kind::Value,
    }
}

/// Where lowering an instruction leaves its block.
enum Flow {
    /// The block goes on.
    On,
    /// The instruction ended the block.
    Ends(Term),
}

/// Where a write happens: at an instruction, or on entry for a register the
/// function reads before writing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Site {
    Entry,
    At(usize),
    /// A register read back from the register file after the call at this
    /// instruction.
    Reload(usize),
}

#[derive(Clone, Copy, Debug)]
struct Def {
    site: Site,
    reg: u16,
}

/// Union-find over a function's writes.
struct Webs(Vec<u32>);

impl Webs {
    fn find(&mut self, x: u32) -> u32 {
        let mut root = x;
        while self.0[root as usize] != root {
            root = self.0[root as usize];
        }
        let mut x = x;
        while self.0[x as usize] != root {
            let next = self.0[x as usize];
            self.0[x as usize] = root;
            x = next;
        }
        root
    }
    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a as usize] = b;
        }
    }
}

/// The lowering of one function.
struct Lowering<'a, 'b> {
    an: &'b Analysis<'a>,
    unit: &'b Unit,
    /// Every function that qualifies, by unit.
    ids: &'b FxHashMap<usize, FuncId>,
    defs: Vec<Def>,
    def_at: FxHashMap<(Site, u16), u32>,
    /// The writes that reach each read, by instruction and register.
    reaching: FxHashMap<(usize, u16), Vec<u32>>,
    /// Each write's variable.
    var_of_def: Vec<Var>,
    vars: Vec<Kind>,
    /// Each variable's type, when every write of it agrees on one.
    var_types: Vec<DataType>,
    /// The calls of the function, by instruction.
    call_at: FxHashMap<usize, &'b CallSite>,
    /// The variable each `SaveFrame` keeps a register in until its call
    /// returns, by the `SaveFrame` and the register.
    shadows: FxHashMap<(usize, u16), Var>,
}

impl<'a, 'b> Lowering<'a, 'b> {
    fn new(an: &'b Analysis<'a>, u: usize, ids: &'b FxHashMap<usize, FuncId>) -> Self {
        let unit = &an.units[u];
        let call_at = unit.calls.iter().map(|c| (c.at, c)).collect();
        Self {
            an,
            unit,
            ids,
            defs: Vec::new(),
            def_at: FxHashMap::default(),
            reaching: FxHashMap::default(),
            var_of_def: Vec::new(),
            vars: Vec::new(),
            var_types: Vec::new(),
            call_at,
            shadows: FxHashMap::default(),
        }
    }

    const fn instr(&self, pos: usize) -> Instr {
        self.an.instrs[pos]
    }

    /// The native function a call reaches, if its callee qualified.
    fn native_callee(&self, call: &CallSite) -> Option<FuncId> {
        match call.kind {
            CallKind::Direct { callee } | CallKind::Recursive { callee, .. } => self
                .an
                .unit_at
                .get(&callee)
                .and_then(|u| self.ids.get(u))
                .copied(),
            CallKind::Indirect { .. } => None,
        }
    }

    fn callee_unit(&self, call: &CallSite) -> Option<&'b Unit> {
        match call.kind {
            CallKind::Direct { callee } | CallKind::Recursive { callee, .. } => {
                self.an.unit_at.get(&callee).map(|&u| &self.an.units[u])
            }
            CallKind::Indirect { .. } => None,
        }
    }

    /// Whether the call's result comes back into its register.
    fn call_returns(&self, call: &CallSite) -> bool {
        match call.kind {
            CallKind::Direct { callee } | CallKind::Recursive { callee, .. } => {
                self.an.returns_value(callee)
            }
            CallKind::Indirect { .. } => true,
        }
    }

    /// The registers read back from the register file after the call.
    fn reloads(&self, call: &CallSite) -> Vec<u16> {
        let mut regs: Vec<u16> = self.an.read_after(self.unit, call).into_iter().collect();
        regs.sort_unstable();
        regs
    }

    /// The writes instruction `pos` makes, as native code sees them: a call
    /// writes what its `SaveFrame` kept, then its result, then the registers
    /// read back after it; `SaveFrame` itself writes nothing.
    fn writes(&self, pos: usize) -> Vec<(Site, u16)> {
        let mut out = Vec::new();
        if let Some(call) = self.call_at.get(&pos) {
            for reg in self.restored(call) {
                out.push((Site::At(pos), reg));
            }
            if self.call_returns(call) {
                out.push((Site::At(pos), call.ret));
            }
            for reg in self.reloads(call) {
                out.push((Site::Reload(pos), reg));
            }
            return out;
        }
        let instr = self.instr(pos);
        if matches!(instr, Instr::SaveFrame(..)) {
            return out;
        }
        instr.for_each_write_reg(|reg| out.push((Site::At(pos), reg)));
        out
    }

    /// The registers a recursive call puts back as its `SaveFrame` kept
    /// them: all it kept but its result.
    fn restored(&self, call: &CallSite) -> Vec<u16> {
        match &call.kind {
            CallKind::Recursive { saved, .. } => saved
                .iter()
                .copied()
                .filter(|r| *r != call.ret && !self.an.is_constant(*r))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The registers instruction `pos` reads, as native code sees them.
    fn reads(&self, pos: usize) -> Vec<u16> {
        let mut out = Vec::new();
        if let Instr::SaveFrame(_, _, callsite) = self.instr(pos) {
            out.extend(self.an.saved(callsite));
        } else if let Some(call) = self.call_at.get(&pos) {
            if let Some(callee) = self.callee_unit(call) {
                let mut regs: Vec<u16> = callee.live_in.iter().copied().collect();
                regs.sort_unstable();
                out.extend(regs);
            }
            // A call into the interpreter leaves the result register as it
            // was when the callee returns nothing, so the value in it goes
            // into the register file first.
            if self.native_callee(call).is_none() {
                out.push(call.ret);
            }
        } else {
            self.instr(pos).for_each_read_reg(|reg| out.push(reg));
        }
        out.retain(|reg| !self.an.is_constant(*reg));
        out
    }

    fn add_def(&mut self, site: Site, reg: u16) -> u32 {
        if let Some(&d) = self.def_at.get(&(site, reg)) {
            return d;
        }
        let d = self.defs.len() as u32;
        self.defs.push(Def { site, reg });
        self.def_at.insert((site, reg), d);
        d
    }

    /// Finds, for every read in the body, the writes that reach it, and
    /// joins the writes reaching a common read into one variable.
    fn build_webs(&mut self) {
        let body = self.unit.body.clone();
        let mut entry_state: FxHashMap<u16, Vec<u32>> = FxHashMap::default();
        let mut live_in: Vec<u16> = self.unit.live_in.iter().copied().collect();
        live_in.sort_unstable();
        for reg in live_in {
            let d = self.add_def(Site::Entry, reg);
            entry_state.insert(reg, vec![d]);
        }
        let writes: Vec<Vec<u32>> = body
            .iter()
            .map(|&pos| {
                self.writes(pos)
                    .into_iter()
                    .map(|(site, reg)| self.add_def(site, reg))
                    .collect()
            })
            .collect();
        let index: FxHashMap<usize, usize> =
            body.iter().enumerate().map(|(k, &p)| (p, k)).collect();

        // Forward, at each instruction, the writes reaching it.
        let mut state_in: Vec<Option<FxHashMap<u16, Vec<u32>>>> = vec![None; body.len()];
        let Some(&first) = index.get(&self.unit.entry) else {
            return;
        };
        state_in[first] = Some(entry_state);
        let mut work = vec![first];
        while let Some(k) = work.pop() {
            let mut state = state_in[k].clone().unwrap_or_default();
            for &d in &writes[k] {
                let reg = self.defs[d as usize].reg;
                // The result of a call and what it reads back are written
                // after its reads, so they replace what reached them.
                state.insert(reg, vec![d]);
            }
            let pos = body[k];
            for next in successors(pos, self.instr(pos)).into_iter().flatten() {
                let Some(&j) = index.get(&next) else { continue };
                let changed = match &mut state_in[j] {
                    None => {
                        state_in[j] = Some(state.clone());
                        true
                    }
                    Some(existing) => {
                        let mut changed = false;
                        for (reg, ds) in &state {
                            let slot = existing.entry(*reg).or_default();
                            for d in ds {
                                if !slot.contains(d) {
                                    slot.push(*d);
                                    changed = true;
                                }
                            }
                        }
                        changed
                    }
                };
                if changed {
                    work.push(j);
                }
            }
        }

        let mut webs = Webs((0..self.defs.len() as u32).collect());
        for (k, &pos) in body.iter().enumerate() {
            let Some(state) = &state_in[k] else { continue };
            let mut regs = self.reads(pos);
            // A `MovChain` reads the register it refills before refilling it,
            // and the in-place steps read what they write.
            regs.sort_unstable();
            regs.dedup();
            for reg in regs {
                if let Some(ds) = state.get(&reg) {
                    for w in ds.windows(2) {
                        webs.union(w[0], w[1]);
                    }
                    self.reaching.insert((pos, reg), ds.clone());
                }
            }
        }

        // A variable per web, typed by the writes in it.
        let mut var_of_root: FxHashMap<u32, Var> = FxHashMap::default();
        let mut types: Vec<Option<DataType>> = Vec::new();
        self.var_of_def = Vec::with_capacity(self.defs.len());
        for d in 0..self.defs.len() as u32 {
            let root = webs.find(d);
            let var = *var_of_root.entry(root).or_insert_with(|| {
                types.push(None);
                (types.len() - 1) as Var
            });
            self.var_of_def.push(var);
            let ty = self.def_type(self.defs[d as usize]);
            let slot = &mut types[var as usize];
            *slot = match slot.take() {
                None => Some(ty),
                Some(old) if old == ty => Some(old),
                Some(old) if kind_of(&old) == kind_of(&ty) && kind_of(&ty) != Kind::Value => {
                    Some(old)
                }
                Some(_) => Some(DataType::Unknown),
            };
        }
        self.var_types = types
            .into_iter()
            .map(|ty| ty.unwrap_or(DataType::Unknown))
            .collect();
        self.vars = self.var_types.iter().map(kind_of).collect();
    }

    /// The type of what write `def` writes.
    fn def_type(&self, def: Def) -> DataType {
        match def.site {
            // `main` starts on the register file the compiler laid out, so a
            // register it reads first holds the literal placed there.
            Site::Entry if self.unit.entry == 0 => {
                let (boxed, int) = self.an.image.registers[def.reg as usize];
                let value = Data::from_words(boxed, int);
                if value.is_int() {
                    DataType::Int
                } else if value.is_float() {
                    DataType::Float
                } else if value.is_bool() {
                    DataType::Bool
                } else {
                    DataType::Unknown
                }
            }
            Site::Entry => self
                .unit
                .table
                .map(|k| &self.an.table.functions[k])
                .and_then(|f| {
                    f.params
                        .iter()
                        .position(|p| *p == def.reg)
                        .map(|i| f.param_types[i].clone())
                })
                .unwrap_or(DataType::Unknown),
            Site::At(pos) => self
                .an
                .def_types
                .get(&(pos as u32, def.reg))
                .map_or(DataType::Unknown, |ty| (*ty).clone()),
            Site::Reload(_) => DataType::Unknown,
        }
    }

    /// The variable the write of `reg` at `site` lands in.
    fn var_at(&self, site: Site, reg: u16) -> Var {
        self.var_of_def[self.def_at[&(site, reg)] as usize]
    }

    /// What instruction `pos` reads for register `reg`: its variable, or the
    /// constant it holds.
    fn operand(&self, pos: usize, reg: u16) -> Option<Operand> {
        if self.an.is_constant(reg) {
            let (boxed, int) = self.an.image.registers.get(reg as usize).copied()?;
            let value = Data::from_words(boxed, int);
            return Some(if value.is_int() {
                Operand::Int(int)
            } else if value.is_float() {
                Operand::Float(boxed)
            } else if value.is_bool() {
                Operand::Bool(value.as_bool())
            } else {
                Operand::Value(boxed, int)
            });
        }
        let ds = self.reaching.get(&(pos, reg))?;
        Some(Operand::Var(self.var_of_def[ds[0] as usize]))
    }

    /// The kind an operand is held as.
    fn kind(&self, operand: Operand) -> Kind {
        match operand {
            Operand::Var(v) => self.vars[v as usize],
            Operand::Int(_) => Kind::Int,
            Operand::Float(_) => Kind::Float,
            Operand::Bool(_) => Kind::Bool,
            Operand::Value(..) => Kind::Value,
        }
    }

    /// The type an operand holds: as far as the table says for a variable,
    /// and what a constant's tag says for a constant.
    fn type_of(&self, operand: Operand) -> DataType {
        match operand {
            Operand::Var(v) => self.var_types[v as usize].clone(),
            Operand::Int(_) => DataType::Int,
            Operand::Float(_) => DataType::Float,
            Operand::Bool(_) => DataType::Bool,
            Operand::Value(boxed, int) => {
                let value = Data::from_words(boxed, int);
                if value.is_string() {
                    DataType::String
                } else if value.is_function() {
                    DataType::Unknown
                } else if value.is_array() {
                    DataType::Array(None)
                } else if value.is_struct() {
                    DataType::Struct(value.struct_type_id())
                } else {
                    DataType::Unknown
                }
            }
        }
    }

    /// `operand`, when it can be read as `kind`: held as it, or as a whole
    /// value to take it out of.
    fn read(&self, pos: usize, reg: u16, kind: Kind) -> Option<Operand> {
        let operand = self.operand(pos, reg)?;
        let have = self.kind(operand);
        (have == kind || have == Kind::Value || kind == Kind::Value).then_some(operand)
    }

    fn any(&self, pos: usize, reg: u16) -> Option<Operand> {
        self.read(pos, reg, Kind::Value)
    }

    /// The variable instruction `pos` writes `reg` into.
    fn dst(&self, pos: usize, reg: u16) -> Var {
        self.var_at(Site::At(pos), reg)
    }

    fn run(mut self) -> Option<Func> {
        self.build_webs();
        // A variable for each register a `SaveFrame` keeps, holding what it
        // held there until the call puts it back.
        let calls: Vec<&CallSite> = self.unit.calls.iter().collect();
        for call in calls {
            if let CallKind::Recursive { save, .. } = call.kind {
                for reg in self.restored(call) {
                    let operand = self.operand(save, reg)?;
                    let kind = self.kind(operand);
                    let ty = self.type_of(operand);
                    self.vars.push(kind);
                    self.var_types.push(ty);
                    self.shadows
                        .insert((save, reg), (self.vars.len() - 1) as Var);
                }
            }
        }
        let body = self.unit.body.clone();
        let index: FxHashMap<usize, usize> =
            body.iter().enumerate().map(|(k, &p)| (p, k)).collect();

        // Blocks start at the entry, at every branch target, and after every
        // instruction that branches or ends the function.
        let mut leader = vec![false; body.len()];
        leader[*index.get(&self.unit.entry)?] = true;
        for (k, &pos) in body.iter().enumerate() {
            let instr = self.instr(pos);
            let [next, target] = successors(pos, instr);
            let branches = target.is_some() || next != Some(pos + 1);
            if let Some(t) = target.and_then(|t| index.get(&t)) {
                leader[*t] = true;
            }
            if let Some(n) = next.and_then(|n| index.get(&n))
                && (branches || *n != k + 1)
            {
                leader[*n] = true;
            }
            if branches && let Some(after) = index.get(&(pos + 1)) {
                leader[*after] = true;
            }
            if k + 1 < body.len() && body[k + 1] != pos + 1 {
                leader[k + 1] = true;
            }
        }
        // The entry block first, then the rest in bytecode order.
        let entry_k = index[&self.unit.entry];
        let mut order: Vec<usize> = vec![entry_k];
        order.extend((0..body.len()).filter(|&k| leader[k] && k != entry_k));
        let block_of: FxHashMap<usize, BlockId> = order
            .iter()
            .enumerate()
            .map(|(b, &k)| (body[k], b as BlockId))
            .collect();

        let table = self.unit.table.map(|k| &self.an.table.functions[k]);
        let is_main = self.unit.entry == 0;
        let ret_kind = if is_main {
            None
        } else if body
            .iter()
            .any(|&p| matches!(self.instr(p), Instr::Return(_) | Instr::RecursiveReturn(_)))
        {
            Some(table.map_or(Kind::Value, |f| kind_of(&f.returns)))
        } else {
            None
        };

        let mut blocks = Vec::with_capacity(order.len());
        for &start in &order {
            let mut ops = Vec::new();
            let mut k = start;
            let mut pending: Vec<Operand> = Vec::new();
            let term = loop {
                let pos = body[k];
                if let Flow::Ends(term) =
                    self.lower_instr(pos, &mut ops, &mut pending, ret_kind, &block_of)?
                {
                    break term;
                }
                if k + 1 >= body.len() || leader[k + 1] || body[k + 1] != pos + 1 {
                    break Term::Jump(*block_of.get(&(pos + 1))?);
                }
                k += 1;
            };
            // `StoreFuncArg` feeds the call that follows it in the same
            // block; one left over is refused.
            if !pending.is_empty() {
                return None;
            }
            blocks.push(Block {
                ops,
                term,
                cold: false,
            });
        }

        let mut params: Vec<(u16, Var)> = Vec::new();
        let mut live_in: Vec<u16> = self.unit.live_in.iter().copied().collect();
        live_in.sort_unstable();
        for reg in live_in {
            params.push((reg, self.var_at(Site::Entry, reg)));
        }
        let func = Func {
            loc: self.unit.entry as u32,
            params,
            ret: ret_kind,
            vars: self.vars.clone(),
            blocks,
            reenters: false,
            is_main,
        };
        Some(func)
    }

    /// Lowers the instruction at `pos` into `ops`, and answers whether it
    /// ends the block. `pending` collects `StoreFuncArg` operands for the call
    /// they feed. `None` refuses the function.
    #[allow(clippy::too_many_lines)]
    fn lower_instr(
        &self,
        pos: usize,
        ops: &mut Vec<Op>,
        pending: &mut Vec<Operand>,
        ret_kind: Option<Kind>,
        block_of: &FxHashMap<usize, BlockId>,
    ) -> Option<Flow> {
        use Kind::{Bool, Float, Int, Value};
        let instr = self.instr(pos);
        let int = |reg| self.read(pos, reg, Int);
        let float = |reg| self.read(pos, reg, Float);
        let boolean = |reg| self.read(pos, reg, Bool);
        let block = |target: usize| block_of.get(&target).copied();
        let next = || block(pos + 1);
        let jump = |size: u16| block(pos + size as usize);
        let back = |size: u16| pos.checked_sub(size as usize).and_then(block);
        let branch = |cond: Cond, then: Option<BlockId>, other: Option<BlockId>| {
            Some(Flow::Ends(Term::Branch {
                cond,
                then: then?,
                other: other?,
            }))
        };
        macro_rules! int_op {
            ($op:expr, $a:expr, $b:expr, $d:expr) => {{
                ops.push(Op::Int {
                    op: $op,
                    dst: self.dst(pos, $d),
                    a: int($a)?,
                    b: int($b)?,
                });
            }};
        }
        macro_rules! float_op {
            ($op:expr, $a:expr, $b:expr, $d:expr) => {{
                ops.push(Op::Float {
                    op: $op,
                    dst: self.dst(pos, $d),
                    a: float($a)?,
                    b: float($b)?,
                });
            }};
        }
        macro_rules! compare {
            ($cmp:expr, $k:expr, $a:expr, $b:expr, $d:expr) => {{
                ops.push(Op::Compare {
                    cmp: $cmp,
                    dst: self.dst(pos, $d),
                    a: self.read(pos, $a, $k)?,
                    b: self.read(pos, $b, $k)?,
                });
            }};
        }
        macro_rules! cmp_jump {
            ($cmp:expr, $k:expr, $a:expr, $b:expr, $target:expr) => {{
                let cond = Cond::Compare($cmp, self.read(pos, $a, $k)?, self.read(pos, $b, $k)?);
                return branch(cond, $target, next());
            }};
        }
        match instr {
            Instr::Jmp(size) => return Some(Flow::Ends(Term::Jump(jump(size)?))),
            Instr::JmpBack(size) => return Some(Flow::Ends(Term::Jump(back(size)?))),
            Instr::IsFalseJmp(c, size) => {
                let c = boolean(c)?;
                return branch(Cond::False(c), jump(size), next());
            }
            Instr::IsTrueJmp(c, size) => {
                let c = boolean(c)?;
                return branch(Cond::True(c), jump(size), next());
            }
            Instr::SupEqIntJmp(a, b, n) => cmp_jump!(Cmp::IntGe, Int, a, b, jump(n)),
            Instr::SupIntJmp(a, b, n) => cmp_jump!(Cmp::IntGt, Int, a, b, jump(n)),
            Instr::InfEqIntJmp(a, b, n) => cmp_jump!(Cmp::IntLe, Int, a, b, jump(n)),
            Instr::InfIntJmp(a, b, n) => cmp_jump!(Cmp::IntLt, Int, a, b, jump(n)),
            Instr::InfIntJmpBack(a, b, n) => cmp_jump!(Cmp::IntLt, Int, a, b, back(n)),
            Instr::SupEqFloatJmp(a, b, n) => cmp_jump!(Cmp::FloatGe, Float, a, b, jump(n)),
            Instr::SupFloatJmp(a, b, n) => cmp_jump!(Cmp::FloatGt, Float, a, b, jump(n)),
            Instr::InfEqFloatJmp(a, b, n) => cmp_jump!(Cmp::FloatLe, Float, a, b, jump(n)),
            Instr::InfFloatJmp(a, b, n) => cmp_jump!(Cmp::FloatLt, Float, a, b, jump(n)),
            Instr::EqJmp(a, b, n) => cmp_jump!(Cmp::ValueEq, Value, a, b, jump(n)),
            Instr::NotEqJmp(a, b, n) => cmp_jump!(Cmp::ValueNe, Value, a, b, jump(n)),
            Instr::StepIntJmpBack(counter, end, n) => {
                let dst = self.dst(pos, counter);
                ops.push(Op::Int {
                    op: IntOp::Add,
                    dst,
                    a: int(counter)?,
                    b: Operand::Int(1),
                });
                if self.an.observed.contains(&counter) {
                    ops.push(Op::Store {
                        reg: counter,
                        src: Operand::Var(dst),
                    });
                }
                let cond = Cond::Compare(Cmp::IntLt, Operand::Var(dst), int(end)?);
                return branch(cond, back(n), next());
            }
            Instr::Mov(src, dest) => ops.push(Op::Copy {
                dst: self.dst(pos, dest),
                src: self.any(pos, src)?,
            }),
            Instr::MovChain(dest, mid, src) => {
                let first = self.dst(pos, dest);
                ops.push(Op::Copy {
                    dst: first,
                    src: self.any(pos, mid)?,
                });
                // The second move runs after the first, so it reads what the
                // first wrote when it reads the same register.
                let src = if src == dest {
                    Operand::Var(first)
                } else {
                    self.any(pos, src)?
                };
                ops.push(Op::Copy {
                    dst: self.dst(pos, mid),
                    src,
                });
            }
            Instr::SetInt(dest, n) => ops.push(Op::Copy {
                dst: self.dst(pos, dest),
                src: Operand::Int(i64::from(n)),
            }),
            Instr::SetBool(b, dest) => ops.push(Op::Copy {
                dst: self.dst(pos, dest),
                src: Operand::Bool(b),
            }),
            Instr::AddInt(a, b, d) => int_op!(IntOp::Add, a, b, d),
            Instr::SubInt(a, b, d) => int_op!(IntOp::Sub, a, b, d),
            Instr::MulInt(a, b, d) => int_op!(IntOp::Mul, a, b, d),
            Instr::BitAndInt(a, b, d) => int_op!(IntOp::And, a, b, d),
            Instr::BitOrInt(a, b, d) => int_op!(IntOp::Or, a, b, d),
            Instr::BitXorInt(a, b, d) => int_op!(IntOp::Xor, a, b, d),
            Instr::AddIntImm(a, imm, d) => ops.push(Op::Int {
                op: IntOp::Add,
                dst: self.dst(pos, d),
                a: int(a)?,
                b: Operand::Int(i64::from(imm)),
            }),
            Instr::IncInt(r) | Instr::DecInt(r) | Instr::IncIntTo(r, _) | Instr::DecIntTo(r, _) => {
                let d = match instr {
                    Instr::IncIntTo(_, d) | Instr::DecIntTo(_, d) => d,
                    _ => r,
                };
                let step = if matches!(instr, Instr::IncInt(_) | Instr::IncIntTo(..)) {
                    1
                } else {
                    -1
                };
                ops.push(Op::Int {
                    op: IntOp::Add,
                    dst: self.dst(pos, d),
                    a: int(r)?,
                    b: Operand::Int(step),
                });
            }
            Instr::ShlInt(a, b, d) | Instr::ShrInt(a, b, d) => ops.push(Op::Shift {
                left: matches!(instr, Instr::ShlInt(..)),
                dst: self.dst(pos, d),
                a: int(a)?,
                b: int(b)?,
                at: pos as u32,
            }),
            Instr::NegInt(a, d) => ops.push(Op::Unary {
                op: UnaryOp::IntNeg,
                dst: self.dst(pos, d),
                a: int(a)?,
            }),
            Instr::BitNotInt(a, d) => ops.push(Op::Unary {
                op: UnaryOp::IntNot,
                dst: self.dst(pos, d),
                a: int(a)?,
            }),
            Instr::AddFloat(a, b, d) => float_op!(FloatOp::Add, a, b, d),
            Instr::SubFloat(a, b, d) => float_op!(FloatOp::Sub, a, b, d),
            Instr::MulFloat(a, b, d) => float_op!(FloatOp::Mul, a, b, d),
            Instr::DivFloat(a, b, d) => float_op!(FloatOp::Div, a, b, d),
            Instr::NegFloat(a, d) => ops.push(Op::Unary {
                op: UnaryOp::FloatNeg,
                dst: self.dst(pos, d),
                a: float(a)?,
            }),
            Instr::SupInt(a, b, d) => compare!(Cmp::IntGt, Int, a, b, d),
            Instr::SupEqInt(a, b, d) => compare!(Cmp::IntGe, Int, a, b, d),
            Instr::InfInt(a, b, d) => compare!(Cmp::IntLt, Int, a, b, d),
            Instr::InfEqInt(a, b, d) => compare!(Cmp::IntLe, Int, a, b, d),
            Instr::SupFloat(a, b, d) => compare!(Cmp::FloatGt, Float, a, b, d),
            Instr::SupEqFloat(a, b, d) => compare!(Cmp::FloatGe, Float, a, b, d),
            Instr::InfFloat(a, b, d) => compare!(Cmp::FloatLt, Float, a, b, d),
            Instr::InfEqFloat(a, b, d) => compare!(Cmp::FloatLe, Float, a, b, d),
            Instr::Eq(a, b, d) => compare!(Cmp::ValueEq, Value, a, b, d),
            Instr::NotEq(a, b, d) => compare!(Cmp::ValueNe, Value, a, b, d),
            Instr::BoolAnd(a, b, d) => ops.push(Op::BoolAnd {
                dst: self.dst(pos, d),
                a: boolean(a)?,
                b: boolean(b)?,
            }),
            Instr::BoolOr(a, b, d) => ops.push(Op::BoolOr {
                dst: self.dst(pos, d),
                a: boolean(a)?,
                b: boolean(b)?,
            }),
            Instr::NegBool(a, d) => ops.push(Op::Unary {
                op: UnaryOp::Not,
                dst: self.dst(pos, d),
                a: boolean(a)?,
            }),
            Instr::CallLibFunc(func, a, d) => {
                let arg = self.any(pos, a)?;
                let arg_type = self.type_of(arg);
                let op = match (func, self.kind(arg), &arg_type) {
                    (LibFunc::SqrtFloat, Float | Value, _) => UnaryOp::FloatSqrt,
                    // `float` of anything but an `int` parses a string.
                    (LibFunc::Float, Int, _) | (LibFunc::Float, Value, DataType::Int) => {
                        UnaryOp::IntToFloat
                    }
                    (LibFunc::Int, Float, _) | (LibFunc::Int, Value, DataType::Float) => {
                        UnaryOp::FloatToInt
                    }
                    (LibFunc::Abs, Int, _) => UnaryOp::IntAbs,
                    (LibFunc::Abs, Float, _) => UnaryOp::FloatAbs,
                    (LibFunc::Len, Value, DataType::Array(_)) => {
                        ops.push(Op::ListLen {
                            dst: self.dst(pos, d),
                            list: arg,
                        });
                        return Some(Flow::On);
                    }
                    _ => return None,
                };
                ops.push(Op::Unary {
                    op,
                    dst: self.dst(pos, d),
                    a: arg,
                });
            }
            Instr::GetIndexArray(list, index, d) => ops.push(Op::ListGet {
                dst: self.dst(pos, d),
                list: self.any(pos, list)?,
                index: int(index)?,
                at: pos as u32,
            }),
            Instr::SetElementObj(list, value, index) => {
                let value = self.any(pos, value)?;
                let list = self.any(pos, list)?;
                let barrier = !(scalar(&element(&self.type_of(list))));
                ops.push(Op::ListSet {
                    list,
                    index: int(index)?,
                    value,
                    at: pos as u32,
                    barrier,
                });
            }
            Instr::GetFieldStruct(obj, field, d) => ops.push(Op::FieldGet {
                dst: self.dst(pos, d),
                obj: self.any(pos, obj)?,
                field,
            }),
            Instr::LoadCell(cell, d) => ops.push(Op::FieldGet {
                dst: self.dst(pos, d),
                obj: self.any(pos, cell)?,
                field: 0,
            }),
            Instr::SetFieldStruct(obj, value, field) => {
                let obj = self.any(pos, obj)?;
                let barrier = !scalar(&self.field_type(&self.type_of(obj), field));
                ops.push(Op::FieldSet {
                    obj,
                    field,
                    value: self.any(pos, value)?,
                    barrier,
                });
            }
            Instr::StoreCell(cell, value) => ops.push(Op::FieldSet {
                obj: self.any(pos, cell)?,
                field: 0,
                value: self.any(pos, value)?,
                barrier: true,
            }),
            Instr::AddFieldFloat(obj, value, field) => ops.push(Op::FieldAddFloat {
                obj: self.any(pos, obj)?,
                field,
                value: float(value)?,
            }),
            Instr::ObjElemMov(value, id, index) => ops.push(Op::PoolSet {
                id: u32::from(id),
                index,
                value: self.any(pos, value)?,
                barrier: true,
            }),
            Instr::Print(value) => ops.push(Op::Print {
                value: self.any(pos, value)?,
            }),
            Instr::StoreFuncArg(reg) => pending.push(self.any(pos, reg)?),
            Instr::CallDynamicLibFunc(id, d) => {
                let types = &self.an.image.dyn_lib_fns.get(id as usize)?.types;
                let (ret, params) = types.split_first()?;
                let scalar_kind = |ty: &DataType| match ty {
                    DataType::Int => Some(Int),
                    DataType::Float => Some(Float),
                    _ => None,
                };
                if params.len() != pending.len() {
                    return None;
                }
                let mut args = Vec::with_capacity(params.len());
                for (arg, ty) in pending.drain(..).zip(params) {
                    let k = scalar_kind(ty)?;
                    let have = self.kind(arg);
                    if have != k && have != Value {
                        return None;
                    }
                    args.push((arg, k));
                }
                let ret = match ret {
                    DataType::Null => None,
                    ty => Some((self.dst(pos, d), scalar_kind(ty)?)),
                };
                ops.push(Op::Dylib { id, args, ret });
            }
            Instr::SaveFrame(..) => {
                if let Some(call) = self
                    .unit
                    .calls
                    .iter()
                    .find(|c| matches!(c.kind, CallKind::Recursive { save, .. } if save == pos))
                {
                    for reg in self.restored(call) {
                        ops.push(Op::Copy {
                            dst: self.shadows[&(pos, reg)],
                            src: self.operand(pos, reg)?,
                        });
                    }
                }
            }
            Instr::CallFunc(..) | Instr::CallFuncRecursive(..) => {
                self.lower_call(pos, ops)?;
            }
            Instr::Return(r) | Instr::RecursiveReturn(r) => {
                let kind = ret_kind?;
                let value = self.read(pos, r, kind)?;
                return Some(Flow::Ends(Term::Return(Some(value))));
            }
            Instr::VoidReturn => return Some(Flow::Ends(Term::Return(None))),
            Instr::Halt(0) => return Some(Flow::Ends(Term::Halt)),
            _ => return None,
        }
        // A register some caller reads after a call that could write it is
        // kept in the register file too.
        for (site, reg) in self.writes(pos) {
            if site == Site::At(pos) && self.an.observed.contains(&reg) {
                ops.push(Op::Store {
                    reg,
                    src: Operand::Var(self.var_at(site, reg)),
                });
            }
        }
        Some(Flow::On)
    }

    /// Lowers the call at `pos`: to a native call when its callee
    /// qualified, and into the interpreter when it did not.
    fn lower_call(&self, pos: usize, ops: &mut Vec<Op>) -> Option<()> {
        let call = *self.call_at.get(&pos)?;
        let callee = self.callee_unit(call)?;
        // Where a call past the depth limit is reported: the frame push,
        // which for a recursive call is its `SaveFrame`.
        let at = match call.kind {
            CallKind::Recursive { save, .. } => save,
            _ => pos,
        } as u32;
        let mut live_in: Vec<u16> = callee.live_in.iter().copied().collect();
        live_in.sort_unstable();
        let dst = self
            .call_returns(call)
            .then(|| self.var_at(Site::At(pos), call.ret));
        if let Some(id) = self.native_callee(call) {
            let args = live_in
                .iter()
                .map(|&reg| self.any(pos, reg))
                .collect::<Option<Vec<_>>>()?;
            ops.push(Op::Call(Call {
                callee: id,
                args,
                dst,
                at,
                ret_reg: call.ret,
                roots: Vec::new(),
                has_room: false,
            }));
        } else {
            let mut args = live_in
                .iter()
                .map(|&reg| Some((reg, self.any(pos, reg)?)))
                .collect::<Option<Vec<_>>>()?;
            // The result register keeps its value through a callee that
            // returns nothing, so the interpreter gets that value too.
            if !live_in.contains(&call.ret)
                && !self.an.is_constant(call.ret)
                && let Some(value) = self.operand(pos, call.ret)
            {
                args.push((call.ret, value));
            }
            ops.push(Op::CallVm(CallVm {
                loc: callee.entry as u32,
                args,
                ret_reg: call.ret,
                dst,
                at,
                roots: Vec::new(),
            }));
        }
        // What the `SaveFrame` kept comes back, and what the call may have
        // written that is read after it comes back out of the register file.
        if let CallKind::Recursive { save, .. } = call.kind {
            for reg in self.restored(call) {
                ops.push(Op::Copy {
                    dst: self.var_at(Site::At(pos), reg),
                    src: Operand::Var(self.shadows[&(save, reg)]),
                });
            }
        }
        for reg in self.reloads(call) {
            ops.push(Op::Reload {
                reg,
                dst: self.var_at(Site::Reload(pos), reg),
            });
        }
        Some(())
    }

    /// The declared type of field `field` of a value of type `ty`.
    fn field_type(&self, ty: &DataType, field: u16) -> DataType {
        match ty {
            DataType::Struct(id) => self
                .an
                .image
                .structs
                .get(*id as usize)
                .and_then(|s| s.fields.get(field as usize))
                .map_or(DataType::Unknown, |(_, ty, _)| ty.clone()),
            _ => DataType::Unknown,
        }
    }
}

/// What a list of type `ty` holds.
fn element(ty: &DataType) -> DataType {
    match ty {
        DataType::Array(Some(element)) => (**element).clone(),
        _ => DataType::Unknown,
    }
}

/// Whether a value of type `ty` is never a reference the collector follows.
const fn scalar(ty: &DataType) -> bool {
    matches!(ty, DataType::Int | DataType::Float | DataType::Bool)
}

/// Fills in, for every call of `func`, the variables holding references that
/// are still read after it: a call can end up in the interpreter, which can
/// collect, and the collector only sees what the VM holds.
fn root_calls(func: &mut Func) {
    let n = func.blocks.len();
    let succ: Vec<Vec<usize>> = func
        .blocks
        .iter()
        .map(|b| match &b.term {
            Term::Jump(t) => vec![*t as usize],
            Term::Branch { then, other, .. } => vec![*then as usize, *other as usize],
            Term::Return(_) | Term::Halt => vec![],
        })
        .collect();
    let term_uses = |term: &Term| -> Vec<Var> {
        let mut out = Vec::new();
        let mut operand = |o: &Operand| {
            if let Operand::Var(v) = o {
                out.push(*v);
            }
        };
        match term {
            Term::Branch { cond, .. } => match cond {
                Cond::True(a) | Cond::False(a) => operand(a),
                Cond::Room => {}
                Cond::Compare(_, a, b) => {
                    operand(a);
                    operand(b);
                }
            },
            Term::Return(Some(a)) => operand(a),
            _ => {}
        }
        out
    };
    let mut live_in: Vec<FxHashSet<Var>> = vec![FxHashSet::default(); n];
    loop {
        let mut changed = false;
        for b in (0..n).rev() {
            let mut live: FxHashSet<Var> = FxHashSet::default();
            for s in &succ[b] {
                live.extend(live_in[*s].iter().copied());
            }
            live.extend(term_uses(&func.blocks[b].term));
            for op in func.blocks[b].ops.iter().rev() {
                if let Some(d) = op.def() {
                    live.remove(&d);
                }
                op.for_each_use(|v| {
                    live.insert(v);
                });
            }
            if live != live_in[b] {
                live_in[b] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut reenters = false;
    let vars = func.vars.clone();
    for (block, succ) in func.blocks.iter_mut().zip(&succ) {
        let mut live: FxHashSet<Var> = FxHashSet::default();
        for s in succ {
            live.extend(live_in[*s].iter().copied());
        }
        live.extend(term_uses(&block.term));
        for op in block.ops.iter_mut().rev() {
            if let Some(d) = op.def() {
                live.remove(&d);
            }
            let roots = || {
                let mut roots: Vec<Var> = live
                    .iter()
                    .copied()
                    .filter(|v| vars[*v as usize] == Kind::Value)
                    .collect();
                roots.sort_unstable();
                roots
            };
            match op {
                Op::Call(call) => {
                    reenters = true;
                    call.roots = roots();
                }
                Op::CallVm(call) => {
                    reenters = true;
                    call.roots = roots();
                }
                _ => {}
            }
            op.for_each_use(|v| {
                live.insert(v);
            });
        }
    }
    func.reenters = reenters;
}
