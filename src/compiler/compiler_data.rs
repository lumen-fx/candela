// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use super::expr::Expr;
use super::expr::Span;
use super::registers::get_tgt_ids;
use super::type_system::DataType;
use super::type_system::Generics;
use super::type_system::GenericsCheckpoint;
use super::type_system::ReturnAnnotation;
use super::type_system::TypeCtx;
use super::type_system::TypeExpr;
use super::type_system::TypeParams;
use crate::compiler::FileNamespaces;
use crate::compiler::FileNamespacesCheckpoint;
use crate::compiler::Namespace;
use crate::data::Data;
use crate::data::NULL;
use crate::instr::Instr;
use crate::instr::LibFunc;
use crate::rt::FnValue;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_strc::SmolStr;
use std::rc::Rc;

// Runtime data types moved to `crate::rt` so the VM can be built without the
// compiler. Re-exported here to keep the compiler's `compiler_data::X` paths
// working unchanged.
pub use crate::rt::{
    DynamicLibFn, EnumType, EnumVariant, ErrorCatch, HostFnSig, InstrSrc, Pools, Source, Struct,
};

/// The type tables a message needs to call a user type by the name the program
/// gave it.
///
/// A [`DataType`] carries only a struct's or an enum's id, so
/// [`Display`](std::fmt::Display) has nothing to print but the words `struct`
/// and `enum`: every enum in the program reads the same, and a mismatch reads
/// "expects `enum[]`, got `int[]`". Pairing a type with these tables through
/// [`TypeNames::of`] prints `Value[]` instead.
///
/// Naming a type is a compile-time job: the names come from the tables a
/// compile builds, and nothing the VM runs reads them. A frontend that shows a
/// type to a person reaches for this too, which is why the crate root
/// re-exports it.
#[derive(Clone, Copy)]
pub struct TypeNames<'a> {
    pub structs: &'a [Struct],
    pub enums: &'a [EnumType],
}

impl<'a> TypeNames<'a> {
    /// `ty` written out with these tables.
    #[must_use]
    pub const fn of(self, ty: &'a DataType) -> NamedType<'a> {
        NamedType { ty, names: self }
    }
}

/// A [`DataType`] together with the tables that name its structs and enums, as
/// produced by [`TypeNames::of`].
pub struct NamedType<'a> {
    ty: &'a DataType,
    names: TypeNames<'a>,
}

impl std::fmt::Display for NamedType<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.ty {
            DataType::Struct(id) => match self.names.structs.get(*id as usize) {
                Some(declared) => write!(f, "{}", declared.name),
                None => write!(f, "{}", self.ty),
            },
            DataType::Enum(id) => match self.names.enums.get(*id as usize) {
                Some(declared) => write!(f, "{}", declared.name),
                None => write!(f, "{}", self.ty),
            },
            DataType::Array(element) => match element {
                Some(element) => write!(f, "{}[]", self.names.of(element)),
                None => write!(f, "any[]"),
            },
            DataType::Map(entry) => write!(
                f,
                "{{{}: {}}}",
                self.names
                    .of(entry.0.as_ref().unwrap_or(&DataType::Unknown)),
                self.names
                    .of(entry.1.as_ref().unwrap_or(&DataType::Unknown))
            ),
            DataType::Union(types) => {
                for (i, member) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, "|")?;
                    }
                    write!(f, "{}", self.names.of(member))?;
                }
                Ok(())
            }
            // The dynamic slot is spelled `any` in a program, and a
            // message about one names it the way the program does.
            DataType::Unknown => write!(f, "any"),
            other => write!(f, "{other}"),
        }
    }
}

#[derive(Debug)]
pub struct Function {
    pub name: SmolStr,
    pub args: Box<[(SmolStr, Option<DataType>)]>,
    pub code: Rc<[Expr]>,
    pub impls: Vec<FunctionImpl>,
    pub is_recursive: Option<bool>,
    pub returns_null: bool,
    pub src_file: u16,
    /// Cache of return types from track_returns, keyed by Box<arg types>
    pub return_type_cache: Vec<(Box<[DataType]>, DataType)>,
    pub direct_calls: Box<[SmolStr]>,
    pub name_span: Span,
    /// The declared `-> Type` return annotation with the span it was written
    /// at.
    ///
    /// `None` leaves the return type inferred from the body. When present, the
    /// inferred return type of every specialisation is checked against it.
    pub return_type: Option<(DataType, Span)>,
    /// Set for a function that has type parameters of its own (`fn first<T>`)
    /// or that came from an `impl` block on a generic type. `None` for an
    /// ordinary function.
    pub generics: Option<Box<FnGenerics>>,
    /// The variables an anonymous function reads from the scope it was written
    /// in, with the type each had there, in the order the environment records
    /// them. Empty for a declared function and for a closure that reads only
    /// its own parameters, which is what keeps that closure lowering the way
    /// it did before capture existed.
    pub captures: Box<[(SmolStr, DataType)]>,
    /// The register holding where this function's body starts, for a function
    /// that becomes a value. It is the first slot of every value built for the
    /// function, and an indirect call jumps to what it holds. `None` for a
    /// function no value is ever made of, which is every function a program
    /// only calls by name.
    pub entry_register: Option<u16>,
}

/// The generic side of a function: what its annotations mean once the type
/// parameters are known.
///
/// `args` and `return_type` on the [`Function`] hold the resolution with no
/// type arguments supplied, where an annotation naming a type parameter is left
/// un-pinned. The unresolved forms are kept here and resolved again for a call
/// site that names its type arguments.
#[derive(Debug)]
pub struct FnGenerics {
    /// Type parameters the function declares.
    pub params: TypeParams,
    /// The parameter annotations as written.
    pub arg_types: Box<[Option<TypeExpr>]>,
    /// The `-> Type` annotation as written.
    pub return_type: ReturnAnnotation,
    /// Type parameters already bound because the enclosing type was
    /// instantiated: `impl Cell<T>` lowered for `Cell<int>` binds `T` to `int`
    /// in every one of its methods.
    pub bindings: Box<[(SmolStr, DataType)]>,
    /// File the declaration was written in, whose scope its annotations resolve
    /// against.
    pub file_idx: u16,
}

#[derive(Debug)]
pub struct FunctionImpl {
    pub loc: u16,
    pub args_loc: Box<[u16]>,
    pub arg_types: Box<[DataType]>,
    /// The type arguments this specialisation was compiled for. A type
    /// parameter that no argument mentions (`fn signal<T>(name: string)`) is
    /// what makes two calls with the same argument types distinct
    /// specialisations.
    pub type_args: Box<[DataType]>,
    /// The register this specialisation reads its environment out of, for a
    /// closure that captures. The call site puts the closure value there
    /// before it jumps.
    pub env_loc: Option<u16>,
    /// Set for a specialisation a call through a value can reach. It takes its
    /// arguments from the shared registers every indirect call writes, so a
    /// call site that does not know which function it reaches still knows where
    /// to put them, and it returns the way a recursive call does, since the
    /// caller saved its registers on the way in.
    pub indirect: bool,
    /// The body a release-profile call site copies in place of the call, for a
    /// specialisation small and simple enough to inline. See
    /// `inlinable_body` in the user-function compiler for what qualifies.
    pub inline_body: Option<Box<[Instr]>>,
}

#[derive(Debug)]
pub struct FnSignature {
    pub name: SmolStr,
    pub args: Box<[DataType]>,
    pub return_type: DataType,
    pub id: u16,
    /// When true (host functions only) the call site accepts any number of
    /// arguments of any type and forwards them to the registered closure as a
    /// slice; `args` is empty and no arity/type checking is performed.
    pub variadic: bool,
}

#[derive(Debug)]
pub struct Dynamiclib {
    pub name: SmolStr,
    pub fns: Box<[FnSignature]>,
    /// When true, this namespace's functions are backed by Rust closures
    /// registered on the embedding `Engine` (a `host "..."` block) rather than
    /// by C symbols loaded from a shared object. Host calls compile to
    /// [`crate::instr::Instr::CallHostFunc`] instead of `CallDynamicLibFunc`, and
    /// their `FnSignature::id` indexes the program's host-function table.
    pub is_host: bool,
}

#[derive(Clone, Copy)]
pub struct Ctx {
    pub block_id: u16,
    /// Whether the code being compiled is within a recursive function
    pub is_compiling_recursive: bool,
    /// Whether the code being compiled is guaranteed to run at most once
    pub single_run: bool,
    /// Whether the code being compiled sits inside a function body. The body of
    /// `main` is compiled inline at the program's top level, where there is no
    /// call frame to return to, so a `return` there ends the program instead.
    pub in_function: bool,
    /// Index of the current file in State's `sources`
    pub file_idx: u16,
    /// Instruction offset that's only used when compiling a function
    pub offset: u16,
    /// The downcast a `return` of an `any` value goes through inside a
    /// function declared to return a concrete type. The declaration types the
    /// call site, so a dynamic value leaving the body is checked on the way
    /// out; a value of a known type was already checked at compile time and
    /// pays nothing.
    pub return_downcast: Option<ReturnCheck>,
}

/// How a `return` checks an `any` value against the declared return type.
#[derive(Clone, Copy)]
pub enum ReturnCheck {
    /// The downcast `as_int`, `as_string` and the rest call, for a scalar, a
    /// list or a map.
    Builtin(LibFunc),
    /// `AsTypeVal` against these type codes, for a struct, an enum, a
    /// function or a union. The list the instruction reads is made by the
    /// first `return` that needs it, so a function whose returns are all
    /// typed adds nothing to the program.
    Types(TypeCodes),
    /// `AsTypeVal` against the list of type codes in this register, for a
    /// union with more members than [`TypeCodes`] holds.
    TypesIn(u16),
}

/// Up to eight type codes, held by value so a [`Ctx`] can carry them.
#[derive(Clone, Copy)]
pub struct TypeCodes {
    pub codes: [i64; 8],
    pub len: u8,
}

impl TypeCodes {
    #[must_use]
    pub fn as_slice(&self) -> &[i64] {
        &self.codes[..self.len as usize]
    }
}

impl Ctx {
    #[inline(always)]
    #[must_use]
    pub const fn no_single_run(self) -> Self {
        Self {
            single_run: false,
            ..self
        }
    }
    #[inline(always)]
    #[must_use]
    pub const fn advance_offset(self, output_len: u16) -> Self {
        Self {
            offset: self.offset + output_len,
            ..self
        }
    }
    #[inline(always)]
    #[must_use]
    pub const fn set_offset(self, offset: u16) -> Self {
        Self { offset, ..self }
    }
}

/// The registers an indirect call passes its arguments and the callee's
/// environment in.
///
/// Every function reachable through a value takes its parameters from these, so
/// a call site that only learns which function it reaches at run time still
/// knows where to put the arguments. They are allocated once, on first use, and
/// never handed to anything else.
#[derive(Debug, Default)]
pub struct IndirectRegisters {
    /// One register per parameter position, grown as wider signatures appear.
    pub args: Vec<u16>,
    /// Where the callee finds the value it was called through, which is its
    /// environment.
    pub env: Option<u16>,
    /// The functions each variable that holds a function value may hold, one
    /// entry per such variable.
    pub value_sets: Vec<FnValueSet>,
}

/// The functions one variable holding a function value may hold.
///
/// A variable grows its set as the code writes more functions into it, and a
/// closure that captured the variable reads the set rather than the copy it
/// took, so a function written in after the closure still reaches it.
#[derive(Debug, Default)]
pub struct FnValueSet {
    pub candidates: Vec<u16>,
    /// The argument types a call through the set has already been compiled
    /// for. A function joining the set later is compiled for each of them, so
    /// a call compiled before it arrived still reaches a body.
    pub used_at: Vec<Box<[DataType]>>,
}

pub struct State<'a> {
    pub registers: &'a mut Vec<Data>,
    pub fns: &'a mut Vec<Function>,
    pub structs: &'a mut Vec<Struct>,
    pub enums: &'a mut Vec<EnumType>,
    pub pools: &'a mut Pools,
    pub instr_src: &'a mut Vec<InstrSrc>,
    pub callsite_registers: &'a mut Vec<Vec<u16>>,
    pub dyn_libs: &'a mut Vec<Dynamiclib>,
    pub allocated_arg_count: &'a mut usize,
    pub allocated_call_depth: &'a mut usize,
    pub const_registers: &'a mut FxHashMap<Data, u16>,
    pub free_registers: &'a mut Vec<u16>,
    pub sources: &'a mut Vec<Source>,
    pub reserved_registers: FxHashSet<u16>,
    /// The call sites, among those that save registers, whose call returns a
    /// value. Such a call writes its return register on the way back, so that
    /// register holds nothing a save would have to keep.
    pub value_callsites: FxHashSet<u16>,
    /// Whether this is the release profile. `candela build` compiles with it
    /// and runs the passes that make the program faster to run; a run from
    /// source, the REPL and an embedding host compile without it, straight
    /// from the syntax tree, so the dev loop never waits on a pass.
    pub optimize: bool,
    pub namespaces: &'a mut FileNamespaces,
    pub generics: &'a mut Generics,
    pub indirect_registers: &'a mut IndirectRegisters,
}

/// A checkpoint of the tables a compile attempt grows as it goes, taken by
/// [`State::checkpoint`] and undone by [`State::rollback_to`] when the attempt
/// ends in a diagnostic.
///
/// Two attempts use it: a host's [`Program::call`](crate::Program::call),
/// which compiles a trampoline against a program that must stay callable
/// after a refused call, and the entry-point pass, which tries an `any` entry
/// point for a host-called function with a bare parameter and keeps building
/// when the body does not compile at `any`.
///
/// This is a handful of lengths, one per table plus one per function and one
/// per file, not a copy of the tables, so an attempt that succeeds pays for
/// little more than reading them.
pub struct CompileCheckpoint {
    registers: usize,
    /// The function table itself grows during a compile: an anonymous function
    /// literal hoists to a fresh entry the first time it is reached, and
    /// instantiating a generic type lowers every applicable `impl` method the
    /// same way. A function the aborted attempt hoisted is left holding a
    /// bytecode address the instruction stream never reached, so it goes back
    /// with everything else.
    functions: usize,
    /// Per function that was already there: how many specialisations it had,
    /// how many return types it had cached, and the register a value of it
    /// starts from. A specialisation compiled during the attempt records a
    /// bytecode address in code that is dropped; a cached return type or an
    /// entry register can name a specialisation or a register that goes too.
    fn_state: Box<[(usize, usize, Option<u16>)]>,
    /// One entry per recursive call site the attempt compiled, each written
    /// once, at the end of the body that holds the call site, so the length
    /// alone undoes the growth.
    callsite_registers: usize,
    /// Every file's scope length, not only the entry file's: a body compiled
    /// from an imported module declares into that module's scope.
    namespaces: FileNamespacesCheckpoint,
    /// A generic type is instantiated, and an enum's variants added, the first
    /// time a call site needs it; `structs` and `enums` cover that growth the
    /// way `functions` covers a closure or a lowered `impl` method.
    structs: usize,
    enums: usize,
    /// Grows in step with the attempt's local instruction buffer, keyed by
    /// instruction value rather than position. Left behind, a stale entry can
    /// mislabel a later instruction identical to one the attempt compiled.
    instr_src: usize,
    generics: GenericsCheckpoint,
}

impl State<'_> {
    /// Snapshots the tables a compile attempt writes into as it goes, so a
    /// diagnostic raised partway through can be undone with
    /// [`State::rollback_to`].
    ///
    /// The instruction stream needs no entry here: an attempt compiles into a
    /// local buffer that only reaches the stream once the whole attempt
    /// succeeds. Everything captured below is written in place, because a
    /// nested call can only reuse a specialisation another call site already
    /// produced.
    #[must_use]
    pub fn checkpoint(&self) -> CompileCheckpoint {
        CompileCheckpoint {
            registers: self.registers.len(),
            functions: self.fns.len(),
            fn_state: self
                .fns
                .iter()
                .map(|f| (f.impls.len(), f.return_type_cache.len(), f.entry_register))
                .collect(),
            callsite_registers: self.callsite_registers.len(),
            namespaces: self.namespaces.checkpoint(),
            structs: self.structs.len(),
            enums: self.enums.len(),
            instr_src: self.instr_src.len(),
            generics: self.generics.checkpoint(),
        }
    }

    /// Undoes everything a failed compile attempt wrote to the tables, back to
    /// `checkpoint`. A successful attempt leaves the tables as they stand and
    /// commits its bytecode alongside them.
    pub fn rollback_to(&mut self, checkpoint: &CompileCheckpoint) {
        // A `const_registers` or `free_registers` entry naming a register at or
        // past `registers` was added during the aborted attempt (registers only
        // grow, and a constant is registered the moment its register is
        // pushed), so it goes with the registers themselves. An entry
        // `free_registers` lost, because the attempt reused an already-free
        // register, is not restored: that register is orphaned rather than
        // double-allocated, which the next compile cannot observe.
        self.const_registers
            .retain(|_, &mut reg| (reg as usize) < checkpoint.registers);
        self.free_registers
            .retain(|&reg| (reg as usize) < checkpoint.registers);
        self.registers.truncate(checkpoint.registers);

        self.fns.truncate(checkpoint.functions);
        for (func, &(impls, cached, entry_register)) in
            self.fns.iter_mut().zip(checkpoint.fn_state.iter())
        {
            func.impls.truncate(impls);
            func.return_type_cache.truncate(cached);
            func.entry_register = entry_register;
        }
        self.callsite_registers
            .truncate(checkpoint.callsite_registers);

        self.namespaces.rollback_to(&checkpoint.namespaces);

        // A generic type instantiated during the attempt is cached in
        // `generics` by rendered name, pointing at the struct or enum entry the
        // attempt pushed; both go together.
        self.structs.truncate(checkpoint.structs);
        self.enums.truncate(checkpoint.enums);
        self.generics.rollback_to(&checkpoint.generics);

        self.instr_src.truncate(checkpoint.instr_src);
    }

    /// The tables a diagnostic needs to name a struct or an enum the program
    /// declared, rather than printing the bare words `struct` and `enum`.
    #[must_use]
    pub fn type_names(&self) -> TypeNames<'_> {
        TypeNames {
            structs: self.structs,
            enums: self.enums,
        }
    }

    /// The scope names written in `file_idx` resolve in: that file's own
    /// declarations, the symbols its bare imports merged in, and the modules it
    /// bound with `as`.
    #[must_use]
    pub fn scope(&self, file_idx: u16) -> &Namespace {
        self.namespaces.get(file_idx)
    }
    /// The same scope, to declare into. A function or struct declared inside a
    /// block is registered here for as long as the block is being compiled.
    pub fn scope_mut(&mut self, file_idx: u16) -> &mut Namespace {
        self.namespaces.get_mut(file_idx)
    }
    /// The scope and registries a [`TypeExpr`] resolves against, borrowed from
    /// this state. Resolving a generic type registers the instantiation, which
    /// is why it needs more than the namespace.
    pub fn type_ctx(&mut self, file_idx: u16) -> TypeCtx<'_> {
        TypeCtx {
            file_idx,
            namespaces: self.namespaces,
            sources: self.sources,
            structs: self.structs,
            enums: self.enums,
            fns: self.fns,
            generics: self.generics,
        }
    }
    /// Marks a register as free, allowing it to later be reused by `alloc_reg`.
    /// The register is marked as free iff:
    /// - the register isn't tied to any variable
    /// - the register isn't a constant register
    /// - the register isn't reserved in `reserved_registers`
    /// - the register isn't already marked as free
    pub fn free_reg(&mut self, id: u16, v: &[Variable]) {
        if !v.iter().any(|var| var.register_id == id)
            && !self.const_registers.values().any(|&reg| reg == id)
            && !self.reserved_registers.contains(&id)
            && !self.free_registers.contains(&id)
        {
            self.free_registers.push(id);
        }
    }
    /// Allocates a register. It `free_registers` isn't empty, it will reuse the latest one. Else, it will allocate a new one.
    pub fn alloc_reg(&mut self) -> u16 {
        if let Some(reg) = self.free_registers.pop() {
            reg
        } else {
            self.registers.push(NULL);
            (self.registers.len() - 1) as u16
        }
    }
    /// The register a constant integer lives in, allocated on first use.
    ///
    /// A constant register is written once by the compiler and read by the
    /// instructions that name it, so every use of the same constant shares
    /// one.
    pub fn const_int_register(&mut self, n: i64) -> u16 {
        self.const_register(Data::int(n))
    }
    /// A new register holding `data`, the template a `Clone*` instruction
    /// copies from.
    ///
    /// Nothing writes a template: every run of the literal reads the value the
    /// compiler put there. The code holding the literal can run again, through
    /// another call of its function or another turn of an enclosing loop, so
    /// the register stays out of the allocator for good.
    pub fn template_register(&mut self, data: Data) -> u16 {
        self.registers.push(data);
        let id = (self.registers.len() - 1) as u16;
        self.reserve_register(id)
    }
    /// The register a constant lives in, allocated on first use and shared by
    /// every use of the same value.
    /// The constant register holding a list of these type codes, the one an
    /// earlier check made when there is one.
    pub fn type_codes_register(&mut self, codes: &[i64]) -> u16 {
        let list: Vec<Data> = codes.iter().map(|&code| Data::int(code)).collect();
        if let Some(&reg) = self.const_registers.iter().find_map(|(data, reg)| {
            (data.is_array() && self.pools.objs[data.as_array()] == list).then_some(reg)
        }) {
            return reg;
        }
        self.pools.objs.push(list);
        let list = Data::array((self.pools.objs.len() - 1) as u32);
        self.const_register(list)
    }
    pub fn const_register(&mut self, data: Data) -> u16 {
        if let Some(&id) = self.const_registers.get(&data) {
            return id;
        }
        let id = self.registers.len() as u16;
        self.const_registers.insert(data, id);
        self.registers.push(data);
        id
    }
    /// Reserves `id` so the allocator never hands it out again, and answers it.
    fn reserve_register(&mut self, id: u16) -> u16 {
        self.reserved_registers.insert(id);
        self.free_registers.retain(|reg| *reg != id);
        id
    }
    /// The register the argument at `idx` of an indirect call travels in.
    ///
    /// Reserved on first use, so the allocator never hands it to anything else:
    /// the caller writes it and the callee reads it as its parameter, and the
    /// two never meet at compile time.
    pub fn indirect_arg_register(&mut self, idx: usize) -> u16 {
        while self.indirect_registers.args.len() <= idx {
            self.registers.push(NULL);
            let id = (self.registers.len() - 1) as u16;
            self.indirect_registers.args.push(id);
        }
        self.reserve_register(self.indirect_registers.args[idx])
    }
    /// A fresh set of the functions one variable may hold, starting with
    /// `fn_id`.
    pub fn new_fn_value_set(&mut self, fn_id: u16) -> u32 {
        self.indirect_registers.value_sets.push(FnValueSet {
            candidates: vec![fn_id],
            used_at: Vec::new(),
        });
        (self.indirect_registers.value_sets.len() - 1) as u32
    }
    /// The functions a call through `fn_type` may reach.
    #[must_use]
    pub fn fn_value_candidates(&self, fn_type: &FnValue) -> Vec<u16> {
        let mut candidates = fn_type.candidates.to_vec();
        if let Some(set) = fn_type.set
            && let Some(set) = self.indirect_registers.value_sets.get(set as usize)
        {
            for id in &set.candidates {
                if !candidates.contains(id) {
                    candidates.push(*id);
                }
            }
        }
        candidates
    }
    /// The register an indirect call leaves the callee's environment in.
    pub fn indirect_env_register(&mut self) -> u16 {
        if let Some(id) = self.indirect_registers.env {
            return self.reserve_register(id);
        }
        self.registers.push(NULL);
        let id = (self.registers.len() - 1) as u16;
        self.indirect_registers.env = Some(id);
        self.reserve_register(id)
    }
    /// The register holding where `fn_id`'s body starts, allocated the first
    /// time the function becomes a value. Every value built for the function
    /// carries it, and the compiler writes the location into it once the
    /// function's indirect specialisation is compiled.
    pub fn fn_entry_register(&mut self, fn_id: usize) -> u16 {
        if let Some(id) = self.fns[fn_id].entry_register {
            return self.reserve_register(id);
        }
        self.registers.push(NULL);
        let id = (self.registers.len() - 1) as u16;
        self.fns[fn_id].entry_register = Some(id);
        self.reserve_register(id)
    }
    /// Allocates a register, reusing `tgt_id` if it holds some register id.
    /// If `tgt_id == None`, it calls `alloc_reg()`.
    #[inline(always)]
    pub fn alloc_reg_tgt(&mut self, tgt_id: Option<u16>) -> u16 {
        if let Some(id) = tgt_id {
            id
        } else {
            self.alloc_reg()
        }
    }
    /// Frees registers that are written by instructions in scope_instrs.
    pub fn free_scope_registers(
        &mut self,
        regs_before: u16,
        scope_instrs: &[Instr],
        v: &[Variable],
    ) {
        for id in get_tgt_ids(scope_instrs) {
            if id >= regs_before {
                self.free_reg(id, v);
            }
        }
    }

    /// Associates the last instruction in `output` with `span` and adds the `InstrSrc` to `instr_src`.
    /// This allows runtime errors to be traced back to `span` in the source code.
    #[inline(always)]
    pub fn add_to_src(&mut self, ctx: Ctx, output: &[Instr], span: Span) {
        self.add_instr_to_src(ctx, unsafe { *output.last().unwrap_unchecked() }, span);
    }
    /// [`add_to_src`](Self::add_to_src) for an instruction that is written back
    /// into `output` after later ones were emitted, so it is no longer the last.
    #[inline(always)]
    pub fn add_instr_to_src(&mut self, ctx: Ctx, instr: Instr, span: Span) {
        self.instr_src.push(InstrSrc {
            instr,
            span,
            file_id: ctx.file_idx,
        });
    }
}

#[derive(Debug)]
pub struct Variable {
    pub name: SmolStr,
    pub register_id: u16,
    pub var_type: DataType,
    /// Set for a variable a closure captures. Its register holds a cell rather
    /// than the value, so a read goes through [`Instr::LoadCell`] and a write
    /// through [`Instr::StoreCell`], and the closure that took it writes the
    /// slot this scope reads.
    pub cell: bool,
}
