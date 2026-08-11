use super::expr::Expr;
use super::expr::Span;
use super::registers::get_tgt_ids;
use super::type_system::DataType;
use crate::compiler::Namespace;
use crate::data::Data;
use crate::data::NULL;
use crate::instr::Instr;
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
}

#[derive(Debug)]
pub struct FunctionImpl {
    pub loc: u16,
    pub args_loc: Box<[u16]>,
    pub arg_types: Box<[DataType]>,
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
    /// Index of the current file in State's `sources`
    pub file_idx: u16,
    /// Instruction offset that's only used when compiling a function
    pub offset: u16,
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

pub struct State<'a> {
    pub registers: &'a mut Vec<Data>,
    pub fns: &'a mut Vec<Function>,
    pub structs: &'a mut Vec<Struct>,
    pub enums: &'a mut Vec<EnumType>,
    pub pools: &'a mut Pools,
    pub instr_src: &'a mut Vec<InstrSrc>,
    pub fn_registers: &'a mut Vec<Vec<u16>>,
    pub dyn_libs: &'a mut Vec<Dynamiclib>,
    pub allocated_arg_count: &'a mut usize,
    pub allocated_call_depth: &'a mut usize,
    pub const_registers: &'a mut FxHashMap<Data, u16>,
    pub free_registers: &'a mut Vec<u16>,
    pub sources: &'a mut Vec<Source>,
    pub reserved_registers: FxHashSet<u16>,
    pub namespace: &'a mut Namespace,
}

impl State<'_> {
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

    /// Similar to free_scope_registers, but also frees CloneArray template registers. Only call this after a loop ends.
    pub fn free_loop_scope_registers(
        &mut self,
        regs_before: u16,
        scope_instrs: &[Instr],
        v: &[Variable],
    ) {
        self.free_scope_registers(regs_before, scope_instrs, v);
        // Free CloneArray template registers
        for instr in scope_instrs {
            if let Instr::CloneArray(template_reg, _, _) = instr
                && *template_reg >= regs_before
            {
                self.free_reg(*template_reg, v);
            } else if let Instr::CloneStruct(template_reg, _) = instr
                && *template_reg >= regs_before
            {
                self.free_reg(*template_reg, v);
            } else if let Instr::CloneEnum(template_reg, _) = instr
                && *template_reg >= regs_before
            {
                self.free_reg(*template_reg, v);
            }
        }
    }
    /// Associates the last instruction in `output` with `span` and adds the `InstrSrc` to `instr_src`.
    /// This allows runtime errors to be traced back to `span` in the source code.
    #[inline(always)]
    pub fn add_to_src(&mut self, ctx: Ctx, output: &[Instr], span: Span) {
        self.instr_src.push(InstrSrc {
            instr: unsafe { *output.last().unwrap_unchecked() },
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
}
