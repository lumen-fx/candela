//! The Cranelift backend: the lowering IR to machine code.
//!
//! Every function becomes two: the function itself, which native callers
//! call with its parameters in machine registers, and an entry point with
//! the one signature the VM calls, which reads the parameters out of the
//! register file and leaves the result in the context. All of them are laid
//! out in one block of code with every call between them resolved, so the
//! code needs no relocation when it is loaded.

use super::ir::Call;
use super::ir::CallVm;
use super::ir::Cmp;
use super::ir::Cond;
use super::ir::FloatOp;
use super::ir::Func;
use super::ir::IntOp;
use super::ir::Kind;
use super::ir::Module;
use super::ir::Op;
use super::ir::Operand;
use super::ir::Term;
use super::ir::UnaryOp;
use candela_vm::native::abi;
use cranelift_codegen::Context;
use cranelift_codegen::FinalizedRelocTarget;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir;
use cranelift_codegen::ir::AbiParam;
use cranelift_codegen::ir::InstBuilder;
use cranelift_codegen::ir::MemFlagsData as MemFlags;
use cranelift_codegen::ir::StackSlotData;
use cranelift_codegen::ir::StackSlotKind;
use cranelift_codegen::ir::UserExternalName;
use cranelift_codegen::ir::UserFuncName;
use cranelift_codegen::ir::Value;
use cranelift_codegen::ir::condcodes::FloatCC;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types;
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::isa::TargetIsa;
use cranelift_frontend::FunctionBuilder;
use cranelift_frontend::FunctionBuilderContext;
use cranelift_frontend::Variable;

/// The tag bits every value but a float sets.
const NAN_BASE: u64 = 0xfff8_0000_0000_0000;
const NAN_BOOL: u64 = NAN_BASE | (1 << 48);
const NAN_INT: u64 = NAN_BASE | (6 << 48);
/// The one NaN a float value is stored as.
const CANONICAL_NAN: u64 = 0x7ff8_0000_0000_0000;
/// The bytes a value takes.
const VALUE_BYTES: i64 = 16;

/// The machine code for a module.
pub struct Code {
    pub bytes: Vec<u8>,
    /// Where each function's entry point starts, in module order.
    pub entries: Vec<u32>,
    /// The most stack one function takes.
    pub frame_bytes: u32,
}

/// Compiles `module` for `isa`, reading objects through `layout` (see
/// [`abi::object_layout`]).
///
/// # Errors
///
/// Returns what Cranelift refused, or a relocation the loader could not
/// resolve.
pub fn compile(module: &Module, isa: &dyn TargetIsa, layout: [u8; 3]) -> Result<Code, String> {
    let cc = isa.default_call_conv();
    let n = module.funcs.len();
    let mut fbctx = FunctionBuilderContext::new();
    let mut ctx = Context::new();
    // Each compiled function's bytes and the calls in it: offset, callee,
    // addend.
    let mut compiled: Vec<(Vec<u8>, Vec<(u32, u32, i64)>)> = Vec::with_capacity(2 * n);
    let mut frame_bytes = 0u32;

    for (id, func) in module.funcs.iter().enumerate() {
        let mut clif = ir::Function::with_name_signature(
            UserFuncName::user(0, id as u32),
            signature(func, cc),
        );
        Emitter::build(&mut clif, &mut fbctx, module, func, isa, layout)?;
        let (bytes, relocs, frame) = compile_one(&mut ctx, clif, isa)?;
        frame_bytes = frame_bytes.max(frame);
        compiled.push((bytes, relocs));
    }
    for (id, func) in module.funcs.iter().enumerate() {
        let mut clif = ir::Function::with_name_signature(
            UserFuncName::user(0, (n + id) as u32),
            entry_signature(cc),
        );
        build_entry(&mut clif, &mut fbctx, func, id as u32, isa);
        let (bytes, relocs, frame) = compile_one(&mut ctx, clif, isa)?;
        frame_bytes = frame_bytes.max(frame);
        compiled.push((bytes, relocs));
    }

    // Lay the functions out and resolve every call between them now.
    let x86 = matches!(
        isa.triple().architecture,
        target_lexicon::Architecture::X86_64
    );
    let mut bytes: Vec<u8> = Vec::new();
    let mut offsets = Vec::with_capacity(compiled.len());
    for (code, _) in &compiled {
        while !bytes.len().is_multiple_of(16) {
            bytes.push(if x86 { 0xcc } else { 0 });
        }
        offsets.push(bytes.len() as u32);
        bytes.extend_from_slice(code);
    }
    for (k, (_, relocs)) in compiled.iter().enumerate() {
        for &(off, target, addend) in relocs {
            let at = (offsets[k] + off) as usize;
            let dest = i64::from(offsets[target as usize]);
            let distance = dest + addend - at as i64;
            if x86 {
                let rel = i32::try_from(distance).map_err(|_| "call out of range")?;
                bytes[at..at + 4].copy_from_slice(&rel.to_le_bytes());
            } else {
                // A `bl`: a signed 26-bit distance in instructions.
                let words = distance >> 2;
                if !(-(1 << 25)..(1 << 25)).contains(&words) {
                    return Err(String::from("call out of range"));
                }
                let word = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default());
                let word = (word & 0xfc00_0000) | ((words as u32) & 0x03ff_ffff);
                bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
    }
    Ok(Code {
        bytes,
        entries: offsets[n..].to_vec(),
        frame_bytes,
    })
}

/// Compiles one function, answering its bytes, its calls to other functions
/// of the module, and the stack one call of it takes.
fn compile_one(
    ctx: &mut Context,
    func: ir::Function,
    isa: &dyn TargetIsa,
) -> Result<(Vec<u8>, Vec<(u32, u32, i64)>, u32), String> {
    ctx.clear();
    ctx.func = func;
    let names: Vec<u32> = ctx
        .func
        .params
        .user_named_funcs()
        .values()
        .map(|name| name.index)
        .collect();
    let code = ctx
        .compile(isa, &mut ControlPlane::default())
        .map_err(|e| format!("{:?}", e.inner))?;
    let bytes = code.code_buffer().to_vec();
    let mut relocs = Vec::new();
    for reloc in code.buffer.relocs() {
        let FinalizedRelocTarget::ExternalName(ir::ExternalName::User(name)) = &reloc.target else {
            return Err(format!("unexpected relocation {:?}", reloc.kind));
        };
        relocs.push((reloc.offset, names[name.as_u32() as usize], reloc.addend));
    }
    // The frame below the frame pointer, and the return address and frame
    // pointer above it.
    let frame = code
        .buffer
        .frame_layout()
        .map_or(0, |layout| layout.frame_to_fp_offset)
        + 32;
    Ok((bytes, relocs, frame))
}

/// The machine types a value of kind `kind` travels in.
const fn clif_types(kind: Kind) -> &'static [ir::Type] {
    match kind {
        Kind::Int | Kind::Bool => &[types::I64],
        Kind::Float => &[types::F64],
        Kind::Value => &[types::I64, types::I64],
    }
}

/// A native function's own signature: the context, how far below the
/// ceiling it stands, and its parameters, returning its result.
fn signature(func: &Func, cc: CallConv) -> ir::Signature {
    let mut sig = ir::Signature::new(cc);
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    for (_, var) in &func.params {
        for t in clif_types(func.vars[*var as usize]) {
            sig.params.push(AbiParam::new(*t));
        }
    }
    if let Some(kind) = func.ret {
        for t in clif_types(kind) {
            sig.returns.push(AbiParam::new(*t));
        }
    }
    sig
}

/// An entry point's signature: [`candela_vm::native::Entry`].
fn entry_signature(cc: CallConv) -> ir::Signature {
    let mut sig = ir::Signature::new(cc);
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I32));
    sig
}

fn helper_signature(cc: CallConv, params: &[ir::Type], returns: &[ir::Type]) -> ir::Signature {
    let mut sig = ir::Signature::new(cc);
    for t in params {
        sig.params.push(AbiParam::new(*t));
    }
    for t in returns {
        sig.returns.push(AbiParam::new(*t));
    }
    sig
}

/// A value in machine code, by kind.
#[derive(Clone, Copy)]
enum Val {
    Int(Value),
    Float(Value),
    Bool(Value),
    /// A whole value: its tagged word and its integer word.
    Whole(Value, Value),
}

const fn trusted() -> MemFlags {
    MemFlags::trusted()
}

/// The entry point for function `id`: reads its parameters out of the
/// register file, calls it, and leaves its result in the context.
fn build_entry(
    clif: &mut ir::Function,
    fbctx: &mut FunctionBuilderContext,
    func: &Func,
    id: u32,
    isa: &dyn TargetIsa,
) {
    let cc = isa.default_call_conv();
    let mut b = FunctionBuilder::new(clif, fbctx);
    let block = b.create_block();
    b.append_block_params_for_function_params(block);
    b.switch_to_block(block);
    let ctx = b.block_params(block)[0];
    let rem = b.block_params(block)[1];
    let sig = b.import_signature(signature(func, cc));
    let name = b
        .func
        .declare_imported_user_function(UserExternalName::new(0, id));
    let callee = b.import_function(ir::ExtFuncData {
        name: ir::ExternalName::user(name),
        signature: sig,
        colocated: true,
        patchable: false,
    });
    let regs = b.ins().load(types::I64, trusted(), ctx, abi::CTX_REGS);
    let mut args = vec![ctx, rem];
    for (reg, var) in &func.params {
        let offset = i32::from(*reg) * VALUE_BYTES as i32;
        match func.vars[*var as usize] {
            Kind::Int => args.push(b.ins().load(types::I64, trusted(), regs, offset + 8)),
            Kind::Float => args.push(b.ins().load(types::F64, trusted(), regs, offset)),
            Kind::Bool => {
                let word = b.ins().load(types::I64, trusted(), regs, offset);
                args.push(b.ins().band_imm_u(word, 1));
            }
            Kind::Value => {
                args.push(b.ins().load(types::I64, trusted(), regs, offset));
                args.push(b.ins().load(types::I64, trusted(), regs, offset + 8));
            }
        }
    }
    let call = b.ins().call(callee, &args);
    let results = b.inst_results(call).to_vec();
    if let Some(kind) = func.ret {
        let value = match kind {
            Kind::Int => Val::Int(results[0]),
            Kind::Float => Val::Float(results[0]),
            Kind::Bool => Val::Bool(results[0]),
            Kind::Value => Val::Whole(results[0], results[1]),
        };
        let (word0, word1) = words(&mut b, value);
        b.ins().store(trusted(), word0, ctx, abi::CTX_RET);
        b.ins().store(trusted(), word1, ctx, abi::CTX_RET + 8);
    }
    let status = b.ins().load(types::I32, trusted(), ctx, abi::CTX_STATUS);
    let status = if func.is_main {
        // `main` ends at its halt.
        let halt = b.ins().iconst(types::I32, i64::from(abi::STATUS_HALT));
        let ok = b.ins().icmp_imm_s(IntCC::Equal, status, 0);
        b.ins().select(ok, halt, status)
    } else {
        status
    };
    b.ins().return_(&[status]);
    b.seal_all_blocks();
    b.finalize(isa.frontend_config());
}

/// A float's bits, with every NaN made the one a value stores.
fn canonical(b: &mut FunctionBuilder<'_>, float: Value) -> Value {
    let bits = b.ins().bitcast(types::I64, MemFlags::new(), float);
    let base = b.ins().iconst(types::I64, NAN_BASE as i64);
    let masked = b.ins().band(bits, base);
    let nan = b.ins().icmp(IntCC::Equal, masked, base);
    let canonical = b.ins().iconst(types::I64, CANONICAL_NAN as i64);
    b.ins().select(nan, canonical, bits)
}

/// A value's two words.
fn words(b: &mut FunctionBuilder<'_>, value: Val) -> (Value, Value) {
    match value {
        Val::Whole(word0, word1) => (word0, word1),
        Val::Int(x) => (b.ins().iconst(types::I64, NAN_INT as i64), x),
        Val::Float(x) => (canonical(b, x), b.ins().iconst(types::I64, 0)),
        Val::Bool(x) => {
            let tag = b.ins().iconst(types::I64, NAN_BOOL as i64);
            (b.ins().bor(tag, x), b.ins().iconst(types::I64, 0))
        }
    }
}

/// Builds one native function.
struct Emitter<'a, 'b> {
    b: FunctionBuilder<'b>,
    module: &'a Module,
    func: &'a Func,
    cc: CallConv,
    layout: [u8; 3],
    /// Each IR variable's machine variables.
    vars: Vec<Vec<Variable>>,
    ctx: Value,
    rem: Value,
    /// Returns what a failed call leaves, once the status is in the context.
    bail: ir::Block,
    /// Where the values kept alive across a call are laid out for the VM.
    roots: Option<ir::StackSlot>,
    callees: Vec<Option<ir::FuncRef>>,
    /// Whether the code being emitted is on a path that is rarely taken,
    /// which puts every block made for it out of the way of the rest.
    cold: bool,
}

impl<'a, 'b> Emitter<'a, 'b> {
    fn build(
        clif: &'b mut ir::Function,
        fbctx: &'b mut FunctionBuilderContext,
        module: &'a Module,
        func: &'a Func,
        isa: &dyn TargetIsa,
        layout: [u8; 3],
    ) -> Result<(), String> {
        let cc = isa.default_call_conv();
        let mut b = FunctionBuilder::new(clif, fbctx);
        let vars: Vec<Vec<Variable>> = func
            .vars
            .iter()
            .map(|kind| {
                clif_types(*kind)
                    .iter()
                    .map(|t| b.declare_var(*t))
                    .collect()
            })
            .collect();
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        let blocks: Vec<ir::Block> = func.blocks.iter().map(|_| b.create_block()).collect();
        let bail = b.create_block();
        b.set_cold_block(bail);
        let most_roots = func
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .map(|op| match op {
                Op::Call(call) => call.roots.len(),
                Op::CallVm(call) => call.roots.len(),
                _ => 0,
            })
            .max()
            .unwrap_or(0);
        let roots = (most_roots > 0).then(|| {
            b.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                (most_roots as u32) * VALUE_BYTES as u32,
                3,
            ))
        });
        b.switch_to_block(entry);
        let params = b.block_params(entry).to_vec();
        let mut e = Emitter {
            b,
            module,
            func,
            cc,
            layout,
            vars,
            ctx: params[0],
            rem: params[1],
            bail,
            roots,
            callees: vec![None; module.funcs.len()],
            cold: false,
        };
        let mut k = 2;
        for (_, var) in &func.params {
            let value = match func.vars[*var as usize] {
                Kind::Int => Val::Int(params[k]),
                Kind::Float => Val::Float(params[k]),
                Kind::Bool => Val::Bool(params[k]),
                Kind::Value => {
                    k += 1;
                    Val::Whole(params[k - 1], params[k])
                }
            };
            k += 1;
            e.set(*var, value)?;
        }
        e.b.ins().jump(blocks[0], &[]);

        for (id, block) in func.blocks.iter().enumerate() {
            if block.cold {
                e.b.set_cold_block(blocks[id]);
            }
            e.cold = block.cold;
            e.b.switch_to_block(blocks[id]);
            for op in &block.ops {
                e.op(op)?;
            }
            e.term(&block.term, &blocks)?;
        }
        e.cold = false;

        e.b.switch_to_block(bail);
        e.default_return();
        e.b.seal_all_blocks();
        e.b.finalize(isa.frontend_config());
        Ok(())
    }

    /// Whether one more native call fits: the function stands at least one
    /// frame below the ceiling.
    fn room(&mut self) -> Value {
        self.b
            .ins()
            .icmp_imm_s(IntCC::SignedGreaterThanOrEqual, self.rem, 1)
    }

    /// A new block, cold when the code around it is.
    fn block(&mut self) -> ir::Block {
        let block = self.b.create_block();
        if self.cold {
            self.b.set_cold_block(block);
        }
        block
    }

    /// Returns zeroes of the function's result type. The caller reads the
    /// status first and never looks at them.
    fn default_return(&mut self) {
        let mut values = Vec::new();
        if let Some(kind) = self.func.ret {
            for t in clif_types(kind) {
                values.push(if *t == types::F64 {
                    self.b.ins().f64const(0.0)
                } else {
                    self.b.ins().iconst(*t, 0)
                });
            }
        }
        self.b.ins().return_(&values);
    }

    fn convert(&mut self, value: Val, kind: Kind) -> Result<Val, String> {
        Ok(match (value, kind) {
            (Val::Int(_), Kind::Int)
            | (Val::Float(_), Kind::Float)
            | (Val::Bool(_), Kind::Bool)
            | (Val::Whole(..), Kind::Value) => value,
            (_, Kind::Value) => {
                let (word0, word1) = words(&mut self.b, value);
                Val::Whole(word0, word1)
            }
            (Val::Whole(_, word1), Kind::Int) => Val::Int(word1),
            (Val::Whole(word0, _), Kind::Float) => {
                Val::Float(self.b.ins().bitcast(types::F64, MemFlags::new(), word0))
            }
            (Val::Whole(word0, _), Kind::Bool) => Val::Bool(self.b.ins().band_imm_u(word0, 1)),
            _ => return Err(String::from("a value read as a kind it cannot be")),
        })
    }

    /// An operand, in whatever kind it is held.
    fn get(&mut self, operand: Operand) -> Val {
        match operand {
            Operand::Var(v) => {
                let vars = &self.vars[v as usize];
                match self.func.vars[v as usize] {
                    Kind::Int => Val::Int(self.b.use_var(vars[0])),
                    Kind::Float => Val::Float(self.b.use_var(vars[0])),
                    Kind::Bool => Val::Bool(self.b.use_var(vars[0])),
                    Kind::Value => {
                        let (a, c) = (vars[0], vars[1]);
                        Val::Whole(self.b.use_var(a), self.b.use_var(c))
                    }
                }
            }
            Operand::Int(n) => Val::Int(self.b.ins().iconst(types::I64, n)),
            Operand::Float(bits) => Val::Float(self.b.ins().f64const(f64::from_bits(bits))),
            Operand::Bool(v) => Val::Bool(self.b.ins().iconst(types::I64, i64::from(v))),
            Operand::Value(word0, word1) => Val::Whole(
                self.b.ins().iconst(types::I64, word0 as i64),
                self.b.ins().iconst(types::I64, word1),
            ),
        }
    }

    fn get_as(&mut self, operand: Operand, kind: Kind) -> Result<Val, String> {
        let value = self.get(operand);
        self.convert(value, kind)
    }

    fn int(&mut self, operand: Operand) -> Result<Value, String> {
        match self.get_as(operand, Kind::Int)? {
            Val::Int(x) => Ok(x),
            _ => unreachable!(),
        }
    }

    fn float(&mut self, operand: Operand) -> Result<Value, String> {
        match self.get_as(operand, Kind::Float)? {
            Val::Float(x) => Ok(x),
            _ => unreachable!(),
        }
    }

    fn boolean(&mut self, operand: Operand) -> Result<Value, String> {
        match self.get_as(operand, Kind::Bool)? {
            Val::Bool(x) => Ok(x),
            _ => unreachable!(),
        }
    }

    fn whole(&mut self, operand: Operand) -> (Value, Value) {
        let value = self.get(operand);
        words(&mut self.b, value)
    }

    fn set(&mut self, var: u32, value: Val) -> Result<(), String> {
        let value = self.convert(value, self.func.vars[var as usize])?;
        let vars = self.vars[var as usize].clone();
        match value {
            Val::Int(x) | Val::Float(x) | Val::Bool(x) => self.b.def_var(vars[0], x),
            Val::Whole(word0, word1) => {
                self.b.def_var(vars[0], word0);
                self.b.def_var(vars[1], word1);
            }
        }
        Ok(())
    }

    /// Reads a value of kind `kind` from memory at `addr`.
    fn load(&mut self, addr: Value, kind: Kind) -> Val {
        match kind {
            Kind::Int => Val::Int(self.b.ins().load(types::I64, trusted(), addr, 8)),
            Kind::Float => Val::Float(self.b.ins().load(types::F64, trusted(), addr, 0)),
            Kind::Bool => {
                let word = self.b.ins().load(types::I64, trusted(), addr, 0);
                Val::Bool(self.b.ins().band_imm_u(word, 1))
            }
            Kind::Value => Val::Whole(
                self.b.ins().load(types::I64, trusted(), addr, 0),
                self.b.ins().load(types::I64, trusted(), addr, 8),
            ),
        }
    }

    fn store(&mut self, addr: Value, value: Val) {
        let (word0, word1) = words(&mut self.b, value);
        self.b.ins().store(trusted(), word0, addr, 0);
        self.b.ins().store(trusted(), word1, addr, 8);
    }

    /// Flags for reading the object pool's headers. A function that cannot
    /// reach the interpreter leaves the pool where it is, so its reads of the
    /// pool can move out of loops.
    const fn header_flags(&self) -> MemFlags {
        if self.func.reenters {
            trusted()
        } else {
            trusted().with_readonly().with_can_move()
        }
    }

    /// The pool entry of the object a value refers to.
    fn object(&mut self, obj: Operand) -> Value {
        let (word0, _) = self.whole(obj);
        let id = self.b.ins().band_imm_u(word0, 0xffff_ffff);
        self.pool_entry_at(id)
    }

    fn pool_entry_at(&mut self, id: Value) -> Value {
        let flags = self.header_flags();
        let base = self
            .b
            .ins()
            .load(types::I64, flags, self.ctx, abi::CTX_OBJS);
        let offset = self.b.ins().imul_imm_s(id, i64::from(self.layout[0]));
        self.b.ins().iadd(base, offset)
    }

    /// The buffer and length of the object at pool entry `entry`.
    fn buffer(&mut self, entry: Value) -> Value {
        let flags = self.header_flags();
        self.b
            .ins()
            .load(types::I64, flags, entry, i32::from(self.layout[1]))
    }

    fn length(&mut self, entry: Value) -> Value {
        let flags = self.header_flags();
        self.b
            .ins()
            .load(types::I64, flags, entry, i32::from(self.layout[2]))
    }

    /// Records a failure in the context and leaves.
    fn fail(&mut self, status: u32, at: u32, a: Option<Value>, b: Option<Value>) {
        let s = self.b.ins().iconst(types::I32, i64::from(status));
        self.b.ins().store(trusted(), s, self.ctx, abi::CTX_STATUS);
        let at = self.b.ins().iconst(types::I32, i64::from(at));
        self.b.ins().store(trusted(), at, self.ctx, abi::CTX_ERR_AT);
        if let Some(a) = a {
            self.b.ins().store(trusted(), a, self.ctx, abi::CTX_ERR_A);
        }
        if let Some(b) = b {
            self.b.ins().store(trusted(), b, self.ctx, abi::CTX_ERR_B);
        }
        self.b.ins().jump(self.bail, &[]);
    }

    /// Branches to a cold block that fails when `bad` holds, and carries on
    /// otherwise.
    fn fail_if(&mut self, bad: Value, status: u32, at: u32, a: Option<Value>, b: Option<Value>) {
        let fail = self.b.create_block();
        let ok = self.block();
        self.b.set_cold_block(fail);
        self.b.ins().brif(bad, fail, &[], ok, &[]);
        self.b.switch_to_block(fail);
        self.fail(status, at, a, b);
        self.b.switch_to_block(ok);
    }

    /// Leaves when the status a call set says it failed.
    fn check_status(&mut self) {
        let status = self
            .b
            .ins()
            .load(types::I32, trusted(), self.ctx, abi::CTX_STATUS);
        let ok = self.block();
        self.b.ins().brif(status, self.bail, &[], ok, &[]);
        self.b.switch_to_block(ok);
    }

    /// Shades the value at `addr` when the collector is marking, before a
    /// store overwrites it.
    fn barrier(&mut self, addr: Value) {
        let marking = self
            .b
            .ins()
            .load(types::I64, trusted(), self.ctx, abi::CTX_MARKING);
        let shade = self.b.create_block();
        let done = self.block();
        self.b.set_cold_block(shade);
        self.b.ins().brif(marking, shade, &[], done, &[]);
        self.b.switch_to_block(shade);
        let old0 = self.b.ins().load(types::I64, trusted(), addr, 0);
        let old1 = self.b.ins().load(types::I64, trusted(), addr, 8);
        let helper = self
            .b
            .ins()
            .load(types::I64, trusted(), self.ctx, abi::CTX_SHADE);
        let sig = self.b.import_signature(helper_signature(
            self.cc,
            &[types::I64, types::I64, types::I64],
            &[],
        ));
        self.b
            .ins()
            .call_indirect(sig, helper, &[self.ctx, old0, old1]);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
    }

    /// The address of register `reg` in the register file.
    fn register(&mut self, reg: u16) -> Value {
        let regs = self.b.ins().load(
            types::I64,
            trusted().with_readonly(),
            self.ctx,
            abi::CTX_REGS,
        );
        self.b.ins().iadd_imm_s(regs, i64::from(reg) * VALUE_BYTES)
    }

    /// Lays `roots` out for the VM, and answers where they start.
    fn lay_out_roots(&mut self, roots: &[u32]) -> Value {
        let Some(slot) = self.roots else {
            return self.b.ins().iconst(types::I64, 0);
        };
        for (k, var) in roots.iter().enumerate() {
            let value = self.get(Operand::Var(*var));
            let (word0, word1) = words(&mut self.b, value);
            let at = (k as i32) * VALUE_BYTES as i32;
            self.b.ins().stack_store(types::I64, word0, slot, at);
            self.b.ins().stack_store(types::I64, word1, slot, at + 8);
        }
        self.b.ins().stack_addr(types::I64, slot, 0)
    }

    /// Calls a helper whose address sits at `offset` in the context.
    fn call_helper(
        &mut self,
        offset: i32,
        params: &[ir::Type],
        returns: &[ir::Type],
        args: &[Value],
    ) -> Vec<Value> {
        let helper = self.b.ins().load(types::I64, trusted(), self.ctx, offset);
        let sig = self
            .b
            .import_signature(helper_signature(self.cc, params, returns));
        let call = self.b.ins().call_indirect(sig, helper, args);
        self.b.inst_results(call).to_vec()
    }

    /// Runs a function on the interpreter: `loc` its first instruction,
    /// `ret_reg` where its result lands.
    fn call_vm(&mut self, loc: u32, ret_reg: u16, at: u32, roots: Option<(Value, usize)>) {
        let loc = self.b.ins().iconst(types::I64, i64::from(loc));
        let ret = self.b.ins().iconst(types::I64, i64::from(ret_reg));
        let at = self.b.ins().iconst(types::I64, i64::from(at));
        let (ptr, count) = match roots {
            Some((ptr, count)) => (ptr, self.b.ins().iconst(types::I64, count as i64)),
            None => (
                self.b.ins().iconst(types::I64, 0),
                self.b.ins().iconst(types::I64, 0),
            ),
        };
        let i64s = [types::I64; 7];
        let results = self.call_helper(
            abi::CTX_CALL_VM,
            &i64s,
            &[types::I32],
            &[self.ctx, loc, ret, at, self.rem, ptr, count],
        );
        let ok = self.block();
        self.b.ins().brif(results[0], self.bail, &[], ok, &[]);
        self.b.switch_to_block(ok);
    }

    fn native_call(&mut self, call: &Call) -> Result<(), String> {
        let callee = &self.module.funcs[call.callee as usize];
        // Keep what this function holds alive, in case the call ends up in
        // the interpreter and that collects.
        if !call.roots.is_empty() {
            let ptr = self.lay_out_roots(&call.roots);
            let count = self.b.ins().iconst(types::I64, call.roots.len() as i64);
            self.call_helper(
                abi::CTX_PUSH_ROOTS,
                &[types::I64; 3],
                &[],
                &[self.ctx, ptr, count],
            );
        }
        let native = self.block();
        let vm = self.b.create_block();
        let join = self.block();
        self.b.set_cold_block(vm);
        if call.has_room {
            self.b.ins().jump(native, &[]);
        } else {
            let room = self.room();
            self.b.ins().brif(room, native, &[], vm, &[]);
        }

        // The call itself, one frame further down.
        self.b.switch_to_block(native);
        let fref = if let Some(fref) = self.callees[call.callee as usize] {
            fref
        } else {
            let sig = self.b.import_signature(signature(callee, self.cc));
            let name = self
                .b
                .func
                .declare_imported_user_function(UserExternalName::new(0, call.callee));
            let fref = self.b.import_function(ir::ExtFuncData {
                name: ir::ExternalName::user(name),
                signature: sig,
                colocated: true,
                patchable: false,
            });
            self.callees[call.callee as usize] = Some(fref);
            fref
        };
        let rem = self.b.ins().iadd_imm_s(self.rem, -1);
        let mut args = vec![self.ctx, rem];
        for (arg, (_, var)) in call.args.iter().zip(&callee.params) {
            match self.get_as(*arg, callee.vars[*var as usize])? {
                Val::Int(x) | Val::Float(x) | Val::Bool(x) => args.push(x),
                Val::Whole(word0, word1) => {
                    args.push(word0);
                    args.push(word1);
                }
            }
        }
        let inst = self.b.ins().call(fref, &args);
        let results = self.b.inst_results(inst).to_vec();
        self.check_status();
        if let (Some(dst), Some(kind)) = (call.dst, callee.ret) {
            let value = match kind {
                Kind::Int => Val::Int(results[0]),
                Kind::Float => Val::Float(results[0]),
                Kind::Bool => Val::Bool(results[0]),
                Kind::Value => Val::Whole(results[0], results[1]),
            };
            self.set(dst, value)?;
        }
        self.b.ins().jump(join, &[]);

        // No room on the stack for another native frame: the call carries on
        // in the interpreter, whose frames are on the heap.
        self.b.switch_to_block(vm);
        let was_cold = std::mem::replace(&mut self.cold, true);
        for (arg, (reg, _)) in call.args.iter().zip(&callee.params) {
            let value = self.get(*arg);
            let addr = self.register(*reg);
            self.store(addr, value);
        }
        self.call_vm(callee.loc, call.ret_reg, call.at, None);
        if let Some(dst) = call.dst {
            let addr = self.register(call.ret_reg);
            let value = self.load(addr, Kind::Value);
            self.set(dst, value)?;
        }
        self.b.ins().jump(join, &[]);
        self.cold = was_cold;

        self.b.switch_to_block(join);
        if !call.roots.is_empty() {
            let count = self.b.ins().iconst(types::I64, call.roots.len() as i64);
            self.call_helper(
                abi::CTX_POP_ROOTS,
                &[types::I64; 2],
                &[],
                &[self.ctx, count],
            );
        }
        Ok(())
    }

    fn vm_call(&mut self, call: &CallVm) -> Result<(), String> {
        for (reg, arg) in &call.args {
            let value = self.get(*arg);
            let addr = self.register(*reg);
            self.store(addr, value);
        }
        let roots = if call.roots.is_empty() {
            None
        } else {
            Some((self.lay_out_roots(&call.roots), call.roots.len()))
        };
        self.call_vm(call.loc, call.ret_reg, call.at, roots);
        if let Some(dst) = call.dst {
            let addr = self.register(call.ret_reg);
            let value = self.load(addr, Kind::Value);
            self.set(dst, value)?;
        }
        Ok(())
    }

    /// A comparison as a machine condition.
    fn compare(&mut self, cmp: Cmp, a: Operand, b: Operand) -> Result<Value, String> {
        let int = |cc| (cc, false);
        let float = |cc: FloatCC| cc;
        Ok(match cmp {
            Cmp::IntLt | Cmp::IntLe | Cmp::IntGt | Cmp::IntGe => {
                let (cc, _) = int(match cmp {
                    Cmp::IntLt => IntCC::SignedLessThan,
                    Cmp::IntLe => IntCC::SignedLessThanOrEqual,
                    Cmp::IntGt => IntCC::SignedGreaterThan,
                    _ => IntCC::SignedGreaterThanOrEqual,
                });
                let (x, y) = (self.int(a)?, self.int(b)?);
                self.b.ins().icmp(cc, x, y)
            }
            Cmp::FloatLt | Cmp::FloatLe | Cmp::FloatGt | Cmp::FloatGe => {
                let cc = float(match cmp {
                    Cmp::FloatLt => FloatCC::LessThan,
                    Cmp::FloatLe => FloatCC::LessThanOrEqual,
                    Cmp::FloatGt => FloatCC::GreaterThan,
                    _ => FloatCC::GreaterThanOrEqual,
                });
                let (x, y) = (self.float(a)?, self.float(b)?);
                self.b.ins().fcmp(cc, x, y)
            }
            Cmp::ValueEq | Cmp::ValueNe => {
                let (x, y) = (self.get(a), self.get(b));
                let same = match (x, y) {
                    (Val::Int(x), Val::Int(y)) | (Val::Bool(x), Val::Bool(y)) => {
                        self.b.ins().icmp(IntCC::Equal, x, y)
                    }
                    _ => {
                        let (x0, x1) = words(&mut self.b, x);
                        let (y0, y1) = words(&mut self.b, y);
                        let e0 = self.b.ins().icmp(IntCC::Equal, x0, y0);
                        let e1 = self.b.ins().icmp(IntCC::Equal, x1, y1);
                        self.b.ins().band(e0, e1)
                    }
                };
                if cmp == Cmp::ValueNe {
                    self.b.ins().bxor_imm_u(same, 1)
                } else {
                    same
                }
            }
        })
    }

    #[allow(clippy::too_many_lines)]
    fn op(&mut self, op: &Op) -> Result<(), String> {
        match op {
            Op::Copy { dst, src } => {
                let value = self.get(*src);
                self.set(*dst, value)?;
            }
            Op::Int { op, dst, a, b } => {
                let (x, y) = (self.int(*a)?, self.int(*b)?);
                let v = match op {
                    IntOp::Add => self.b.ins().iadd(x, y),
                    IntOp::Sub => self.b.ins().isub(x, y),
                    IntOp::Mul => self.b.ins().imul(x, y),
                    IntOp::And => self.b.ins().band(x, y),
                    IntOp::Or => self.b.ins().bor(x, y),
                    IntOp::Xor => self.b.ins().bxor(x, y),
                };
                self.set(*dst, Val::Int(v))?;
            }
            Op::Shift {
                left,
                dst,
                a,
                b,
                at,
            } => {
                let (x, count) = (self.int(*a)?, self.int(*b)?);
                let out = self
                    .b
                    .ins()
                    .icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, count, 64);
                self.fail_if(out, abi::STATUS_SHIFT, *at, Some(count), None);
                let v = if *left {
                    self.b.ins().ishl(x, count)
                } else {
                    self.b.ins().sshr(x, count)
                };
                self.set(*dst, Val::Int(v))?;
            }
            Op::Float { op, dst, a, b } => {
                let (x, y) = (self.float(*a)?, self.float(*b)?);
                let v = match op {
                    FloatOp::Add => self.b.ins().fadd(x, y),
                    FloatOp::Sub => self.b.ins().fsub(x, y),
                    FloatOp::Mul => self.b.ins().fmul(x, y),
                    FloatOp::Div => self.b.ins().fdiv(x, y),
                };
                self.set(*dst, Val::Float(v))?;
            }
            Op::Unary { op, dst, a } => {
                let value = match op {
                    UnaryOp::IntNeg => {
                        let x = self.int(*a)?;
                        Val::Int(self.b.ins().ineg(x))
                    }
                    UnaryOp::IntNot => {
                        let x = self.int(*a)?;
                        Val::Int(self.b.ins().bnot(x))
                    }
                    UnaryOp::IntAbs => {
                        let x = self.int(*a)?;
                        Val::Int(self.b.ins().iabs(x))
                    }
                    UnaryOp::FloatNeg => {
                        let x = self.float(*a)?;
                        Val::Float(self.b.ins().fneg(x))
                    }
                    UnaryOp::FloatAbs => {
                        let x = self.float(*a)?;
                        Val::Float(self.b.ins().fabs(x))
                    }
                    UnaryOp::FloatSqrt => {
                        let x = self.float(*a)?;
                        Val::Float(self.b.ins().sqrt(x))
                    }
                    UnaryOp::IntToFloat => {
                        let x = self.int(*a)?;
                        Val::Float(self.b.ins().fcvt_from_sint(types::F64, x))
                    }
                    UnaryOp::FloatToInt => {
                        let x = self.float(*a)?;
                        Val::Int(self.b.ins().fcvt_to_sint_sat(types::I64, x))
                    }
                    UnaryOp::Not => {
                        let x = self.boolean(*a)?;
                        Val::Bool(self.b.ins().bxor_imm_u(x, 1))
                    }
                };
                self.set(*dst, value)?;
            }
            Op::Compare { cmp, dst, a, b } => {
                let c = self.compare(*cmp, *a, *b)?;
                let v = self.b.ins().uextend(types::I64, c);
                self.set(*dst, Val::Bool(v))?;
            }
            Op::BoolAnd { dst, a, b } | Op::BoolOr { dst, a, b } => {
                let (x, y) = (self.boolean(*a)?, self.boolean(*b)?);
                let v = if matches!(op, Op::BoolAnd { .. }) {
                    self.b.ins().band(x, y)
                } else {
                    self.b.ins().bor(x, y)
                };
                self.set(*dst, Val::Bool(v))?;
            }
            Op::ListGet {
                dst,
                list,
                index,
                at,
            } => {
                let entry = self.object(*list);
                let index = self.int(*index)?;
                let len = self.length(entry);
                let out = self
                    .b
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, index, len);
                self.fail_if(out, abi::STATUS_INDEX, *at, Some(len), Some(index));
                let buffer = self.buffer(entry);
                let offset = self.b.ins().imul_imm_s(index, VALUE_BYTES);
                let addr = self.b.ins().iadd(buffer, offset);
                let value = self.load(addr, self.func.vars[*dst as usize]);
                self.set(*dst, value)?;
            }
            Op::ListSet {
                list,
                index,
                value,
                at,
                barrier,
            } => {
                let entry = self.object(*list);
                let index = self.int(*index)?;
                let len = self.length(entry);
                let out = self
                    .b
                    .ins()
                    .icmp(IntCC::UnsignedGreaterThanOrEqual, index, len);
                self.fail_if(out, abi::STATUS_INDEX, *at, Some(len), Some(index));
                let buffer = self.buffer(entry);
                let offset = self.b.ins().imul_imm_s(index, VALUE_BYTES);
                let addr = self.b.ins().iadd(buffer, offset);
                if *barrier {
                    self.barrier(addr);
                }
                let value = self.get(*value);
                self.store(addr, value);
            }
            Op::ListLen { dst, list } => {
                let entry = self.object(*list);
                let len = self.length(entry);
                self.set(*dst, Val::Int(len))?;
            }
            Op::FieldGet { dst, obj, field } => {
                let entry = self.object(*obj);
                let buffer = self.buffer(entry);
                let addr = self
                    .b
                    .ins()
                    .iadd_imm_s(buffer, i64::from(*field) * VALUE_BYTES);
                let value = self.load(addr, self.func.vars[*dst as usize]);
                self.set(*dst, value)?;
            }
            Op::FieldSet {
                obj,
                field,
                value,
                barrier,
            } => {
                let entry = self.object(*obj);
                let buffer = self.buffer(entry);
                let addr = self
                    .b
                    .ins()
                    .iadd_imm_s(buffer, i64::from(*field) * VALUE_BYTES);
                if *barrier {
                    self.barrier(addr);
                }
                let value = self.get(*value);
                self.store(addr, value);
            }
            Op::FieldAddFloat { obj, field, value } => {
                let entry = self.object(*obj);
                let buffer = self.buffer(entry);
                let addr = self
                    .b
                    .ins()
                    .iadd_imm_s(buffer, i64::from(*field) * VALUE_BYTES);
                let old = self.b.ins().load(types::F64, trusted(), addr, 0);
                let add = self.float(*value)?;
                let sum = self.b.ins().fadd(old, add);
                self.store(addr, Val::Float(sum));
            }
            Op::PoolSet {
                id,
                index,
                value,
                barrier,
            } => {
                let id = self.b.ins().iconst(types::I64, i64::from(*id));
                let entry = self.pool_entry_at(id);
                let buffer = self.buffer(entry);
                let addr = self
                    .b
                    .ins()
                    .iadd_imm_s(buffer, i64::from(*index) * VALUE_BYTES);
                if *barrier {
                    self.barrier(addr);
                }
                let value = self.get(*value);
                self.store(addr, value);
            }
            Op::Call(call) => self.native_call(call)?,
            Op::CallVm(call) => self.vm_call(call)?,
            Op::Reload { reg, dst } => {
                let addr = self.register(*reg);
                let value = self.load(addr, Kind::Value);
                self.set(*dst, value)?;
            }
            Op::Store { reg, src } => {
                let value = self.get(*src);
                let addr = self.register(*reg);
                self.store(addr, value);
            }
            Op::Dylib { id, args, ret } => {
                let mut sig = ir::Signature::new(self.cc);
                let mut values = Vec::with_capacity(args.len());
                for (arg, kind) in args {
                    sig.params.push(AbiParam::new(clif_types(*kind)[0]));
                    match self.get_as(*arg, *kind)? {
                        Val::Int(x) | Val::Float(x) => values.push(x),
                        _ => return Err(String::from("a dylib argument that is not a scalar")),
                    }
                }
                if let Some((_, kind)) = ret {
                    sig.returns.push(AbiParam::new(clif_types(*kind)[0]));
                }
                let sig = self.b.import_signature(sig);
                let table = self.b.ins().load(
                    types::I64,
                    trusted().with_readonly(),
                    self.ctx,
                    abi::CTX_DYLIBS,
                );
                let target = self.b.ins().load(
                    types::I64,
                    trusted().with_readonly(),
                    table,
                    i32::from(*id) * 8,
                );
                let inst = self.b.ins().call_indirect(sig, target, &values);
                if let Some((dst, kind)) = ret {
                    let result = self.b.inst_results(inst)[0];
                    let value = if *kind == Kind::Float {
                        Val::Float(result)
                    } else {
                        Val::Int(result)
                    };
                    self.set(*dst, value)?;
                }
            }
            Op::Print { value } => {
                let (word0, word1) = self.whole(*value);
                self.call_helper(
                    abi::CTX_PRINT,
                    &[types::I64; 3],
                    &[],
                    &[self.ctx, word0, word1],
                );
            }
        }
        Ok(())
    }

    fn term(&mut self, term: &Term, blocks: &[ir::Block]) -> Result<(), String> {
        match term {
            Term::Jump(target) => {
                self.b.ins().jump(blocks[*target as usize], &[]);
            }
            Term::Branch { cond, then, other } => {
                let c = match *cond {
                    Cond::True(a) | Cond::False(a) => {
                        let want = matches!(cond, Cond::True(_));
                        match self.get(a) {
                            Val::Bool(x) => {
                                if want {
                                    x
                                } else {
                                    self.b.ins().icmp_imm_s(IntCC::Equal, x, 0)
                                }
                            }
                            Val::Whole(word0, _) => {
                                let tag = NAN_BOOL | u64::from(want);
                                self.b.ins().icmp_imm_s(IntCC::Equal, word0, tag as i64)
                            }
                            _ => return Err(String::from("a condition that is not a bool")),
                        }
                    }
                    Cond::Compare(cmp, a, b) => self.compare(cmp, a, b)?,
                    Cond::Room => self.room(),
                };
                self.b
                    .ins()
                    .brif(c, blocks[*then as usize], &[], blocks[*other as usize], &[]);
            }
            Term::Return(value) => {
                let mut values = Vec::new();
                if let Some(value) = value {
                    let kind = self
                        .func
                        .ret
                        .ok_or("a value returned from a void function")?;
                    match self.get_as(*value, kind)? {
                        Val::Int(x) | Val::Float(x) | Val::Bool(x) => values.push(x),
                        Val::Whole(word0, word1) => {
                            values.push(word0);
                            values.push(word1);
                        }
                    }
                } else if self.func.ret.is_some() {
                    return Err(String::from(
                        "no value returned from a function that returns one",
                    ));
                }
                self.b.ins().return_(&values);
            }
            Term::Halt => {
                self.default_return();
            }
        }
        Ok(())
    }
}
