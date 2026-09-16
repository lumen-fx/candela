use super::expr::Expr;
use super::expr::Span;
use super::registers::get_tgt_ids;
use super::type_system::DataType;
use super::type_system::Generics;
use super::type_system::ReturnAnnotation;
use super::type_system::TypeCtx;
use super::type_system::TypeExpr;
use super::type_system::TypeParams;
use crate::compiler::FileNamespaces;
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
    pub namespaces: &'a mut FileNamespaces,
    pub generics: &'a mut Generics,
}

impl State<'_> {
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
            fn_registers: self.fn_registers,
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
