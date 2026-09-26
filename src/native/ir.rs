//! The lowering IR: a program's functions as typed code with explicit calls,
//! explicit trips into the interpreter and explicit error exits, ready for a
//! backend to turn into machine code.
//!
//! It sits between the bytecode and a code generator, and holds what every
//! backend needs worked out once: which functions qualify, the machine kind
//! of every value, which calls stay native and which go back to the
//! interpreter, what has to be kept alive across a call that can collect, and
//! which stores need the collector's barrier.
//!
//! Values live in variables, each of one [`Kind`], written and read any
//! number of times; control flow is basic blocks ending in a jump, a branch
//! or a return. The bytecode compiles from structured source, so the blocks
//! always form reducible control flow, which is what a backend for a
//! structured target needs to rebuild loops and conditionals.

/// A variable of a function.
pub type Var = u32;
/// A basic block of a function.
pub type BlockId = u32;
/// A function of a [`Module`].
pub type FuncId = u32;

/// How a value is held in machine code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A 64-bit integer.
    Int,
    /// A 64-bit float.
    Float,
    /// A bool, held as 0 or 1.
    Bool,
    /// A whole value in its two words, the way the VM stores one: any type,
    /// including a reference to a list, a struct, a map or a string.
    Value,
}

/// What an operation reads: a variable, or a constant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand {
    Var(Var),
    Int(i64),
    /// A float, by its bits.
    Float(u64),
    Bool(bool),
    /// A whole value, by its two words.
    Value(u64, i64),
}

/// A program's native functions.
#[derive(Debug, Default)]
pub struct Module {
    pub funcs: Vec<Func>,
    /// The function that stands in for `main`.
    pub main: Option<FuncId>,
    /// The call instructions the VM points at a native function: the call,
    /// and its callee.
    pub sites: Vec<(u32, FuncId)>,
}

/// One native function.
#[derive(Debug)]
pub struct Func {
    /// The instruction its bytecode starts at, which is also what the VM
    /// runs in its place when a call goes back to the interpreter.
    pub loc: u32,
    /// The registers it reads on entry, each bound to the variable it lands
    /// in. A native caller passes them as arguments; the interpreter leaves
    /// them in the register file.
    pub params: Vec<(u16, Var)>,
    /// What it returns, if anything.
    pub ret: Option<Kind>,
    /// Every variable's kind.
    pub vars: Vec<Kind>,
    /// The blocks, the entry first.
    pub blocks: Vec<Block>,
    /// Whether a call it makes, directly or through native callees, can go
    /// back into the interpreter, and so allocate and move the object pool.
    pub reenters: bool,
    pub is_main: bool,
}

/// A straight run of operations and how it ends.
#[derive(Debug)]
pub struct Block {
    pub ops: Vec<Op>,
    pub term: Term,
    /// Whether the block is rarely run, so a backend lays it out of the way
    /// of the rest.
    pub cold: bool,
}

/// Integer operations on two operands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IntOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
}

/// Float operations on two operands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FloatOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Operations on one operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnaryOp {
    /// `-x` on an integer, wrapping.
    IntNeg,
    /// Every bit of an integer flipped.
    IntNot,
    /// The absolute value of an integer, wrapping.
    IntAbs,
    FloatNeg,
    FloatAbs,
    FloatSqrt,
    /// An integer converted to a float.
    IntToFloat,
    /// A float converted to an integer, rounding towards zero and saturating,
    /// with NaN as 0.
    FloatToInt,
    /// `!x` on a bool.
    Not,
}

/// Comparisons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmp {
    IntLt,
    IntLe,
    IntGt,
    IntGe,
    FloatLt,
    FloatLe,
    FloatGt,
    FloatGe,
    /// Two whole values the same word for word, the way `==` compares values
    /// that are not strings, lists or structs.
    ValueEq,
    ValueNe,
}

/// A call to a native function.
#[derive(Clone, Debug)]
pub struct Call {
    pub callee: FuncId,
    /// One per callee parameter, in order.
    pub args: Vec<Operand>,
    pub dst: Option<Var>,
    /// The instruction a call past the depth limit is reported at.
    pub at: u32,
    /// The register the interpreter leaves the result in when the call runs
    /// there because the stack has no more room for native frames.
    pub ret_reg: u16,
    /// Values to keep alive across the call, which can collect.
    pub roots: Vec<Var>,
    /// Whether a [`Cond::Room`] the call is only reached through already
    /// said there is room for its frame.
    pub has_room: bool,
}

/// A call into the interpreter.
#[derive(Clone, Debug)]
pub struct CallVm {
    /// The instruction the callee starts at.
    pub loc: u32,
    /// The registers the callee reads, and what to put in each first.
    pub args: Vec<(u16, Operand)>,
    /// The register the interpreter leaves the result in.
    pub ret_reg: u16,
    /// The variable to read the result into, when there is one.
    pub dst: Option<Var>,
    /// The instruction a call past the depth limit is reported at.
    pub at: u32,
    /// Values to keep alive across the call, which can collect.
    pub roots: Vec<Var>,
}

/// One operation.
#[derive(Clone, Debug)]
pub enum Op {
    Copy {
        dst: Var,
        src: Operand,
    },
    Int {
        op: IntOp,
        dst: Var,
        a: Operand,
        b: Operand,
    },
    /// A shift, raising `shift_count_out_of_range` at `at` for a count outside
    /// 0..64. Right shifts are arithmetic.
    Shift {
        left: bool,
        dst: Var,
        a: Operand,
        b: Operand,
        at: u32,
    },
    Float {
        op: FloatOp,
        dst: Var,
        a: Operand,
        b: Operand,
    },
    Unary {
        op: UnaryOp,
        dst: Var,
        a: Operand,
    },
    /// A comparison into a bool.
    Compare {
        cmp: Cmp,
        dst: Var,
        a: Operand,
        b: Operand,
    },
    /// Both bools true, or either.
    BoolAnd {
        dst: Var,
        a: Operand,
        b: Operand,
    },
    BoolOr {
        dst: Var,
        a: Operand,
        b: Operand,
    },
    /// Element `index` of a list, raising `index_out_of_bounds` at `at` when
    /// it is out of range.
    ListGet {
        dst: Var,
        list: Operand,
        index: Operand,
        at: u32,
    },
    /// Stores element `index` of a list, raising at `at` when it is out of
    /// range.
    ListSet {
        list: Operand,
        index: Operand,
        value: Operand,
        at: u32,
        barrier: bool,
    },
    /// How many elements a list holds.
    ListLen {
        dst: Var,
        list: Operand,
    },
    /// Field `field` of a struct.
    FieldGet {
        dst: Var,
        obj: Operand,
        field: u16,
    },
    FieldSet {
        obj: Operand,
        field: u16,
        value: Operand,
        barrier: bool,
    },
    /// Adds a float to a float field.
    FieldAddFloat {
        obj: Operand,
        field: u16,
        value: Operand,
    },
    /// Stores element `index` of the object the compiler placed at pool
    /// index `id`, the way a literal is filled in.
    PoolSet {
        id: u32,
        index: u16,
        value: Operand,
        barrier: bool,
    },
    Call(Call),
    CallVm(CallVm),
    /// Reads a register back from the interpreter's register file, after a
    /// call that writes it.
    Reload {
        reg: u16,
        dst: Var,
    },
    /// Writes a register of the interpreter's register file, which code
    /// after a call reads.
    Store {
        reg: u16,
        src: Operand,
    },
    /// A `dylib` function called with scalar arguments.
    Dylib {
        id: u16,
        args: Vec<(Operand, Kind)>,
        ret: Option<(Var, Kind)>,
    },
    Print {
        value: Operand,
    },
}

/// A branch condition.
#[derive(Clone, Copy, Debug)]
pub enum Cond {
    /// A bool is true.
    True(Operand),
    /// A bool is false. Only a bool is either; any other value is neither.
    False(Operand),
    /// One more native call would still be within the call depth limit and
    /// the stack native code may use.
    Room,
    Compare(Cmp, Operand, Operand),
}

/// How a block ends.
#[derive(Clone, Debug)]
pub enum Term {
    Jump(BlockId),
    Branch {
        cond: Cond,
        then: BlockId,
        other: BlockId,
    },
    Return(Option<Operand>),
    /// The end of `main`.
    Halt,
}

impl Op {
    /// Calls `f` with each variable the operation reads.
    pub fn for_each_use(&self, mut f: impl FnMut(Var)) {
        let mut operand = |o: &Operand| {
            if let Operand::Var(v) = o {
                f(*v);
            }
        };
        match self {
            Self::Copy { src, .. } | Self::Store { src, .. } => operand(src),
            Self::Int { a, b, .. }
            | Self::Shift { a, b, .. }
            | Self::Float { a, b, .. }
            | Self::Compare { a, b, .. }
            | Self::BoolAnd { a, b, .. }
            | Self::BoolOr { a, b, .. } => {
                operand(a);
                operand(b);
            }
            Self::Unary { a, .. } => operand(a),
            Self::ListGet { list, index, .. } => {
                operand(list);
                operand(index);
            }
            Self::ListSet {
                list, index, value, ..
            } => {
                operand(list);
                operand(index);
                operand(value);
            }
            Self::ListLen { list, .. } => operand(list),
            Self::FieldGet { obj, .. } => operand(obj),
            Self::FieldSet { obj, value, .. } | Self::FieldAddFloat { obj, value, .. } => {
                operand(obj);
                operand(value);
            }
            Self::PoolSet { value, .. } | Self::Print { value } => operand(value),
            Self::Call(call) => {
                for a in &call.args {
                    operand(a);
                }
            }
            Self::CallVm(call) => {
                for (_, a) in &call.args {
                    operand(a);
                }
            }
            Self::Reload { .. } => {}
            Self::Dylib { args, .. } => {
                for (a, _) in args {
                    operand(a);
                }
            }
        }
    }

    /// The variable the operation writes, if any.
    #[must_use]
    pub const fn def(&self) -> Option<Var> {
        match self {
            Self::Copy { dst, .. }
            | Self::Int { dst, .. }
            | Self::Shift { dst, .. }
            | Self::Float { dst, .. }
            | Self::Unary { dst, .. }
            | Self::Compare { dst, .. }
            | Self::BoolAnd { dst, .. }
            | Self::BoolOr { dst, .. }
            | Self::ListGet { dst, .. }
            | Self::ListLen { dst, .. }
            | Self::FieldGet { dst, .. }
            | Self::Reload { dst, .. } => Some(*dst),
            Self::Call(call) => call.dst,
            Self::CallVm(call) => call.dst,
            Self::Dylib {
                ret: Some((v, _)), ..
            } => Some(*v),
            _ => None,
        }
    }
}
