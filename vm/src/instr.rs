use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum Instr {
    Print(u16),

    // LOGIC
    /// Jumps x instructions forwards
    Jmp(u16),
    /// Jumps x instructions backwards
    JmpBack(u16),
    IsFalseJmp(u16, u16),
    IsTrueJmp(u16, u16),
    SupEqFloatJmp(u16, u16, u16),
    SupEqIntJmp(u16, u16, u16),
    SupFloatJmp(u16, u16, u16),
    SupIntJmp(u16, u16, u16),
    InfEqFloatJmp(u16, u16, u16),
    InfEqIntJmp(u16, u16, u16),
    InfFloatJmp(u16, u16, u16),
    InfIntJmp(u16, u16, u16),
    InfIntJmpBack(u16, u16, u16),
    NotEqJmp(u16, u16, u16),
    EqJmp(u16, u16, u16),
    ObjNotEqJmp(u16, u16, u16),
    ObjEqJmp(u16, u16, u16),
    StrNotEqJmp(u16, u16, u16),
    StrEqJmp(u16, u16, u16),

    Mov(u16, u16),
    /// SetInt(dest_reg_id, val)\
    /// Writes val directly into dest_reg_id
    SetInt(u16, i32),
    /// SetBool(dest_reg_id, val)\
    /// Writes val directly into dest_reg_id
    SetBool(bool, u16),

    // OPS
    AddFloat(u16, u16, u16),
    AddInt(u16, u16, u16),
    AddArray(u16, u16, u16),
    AddStr(u16, u16, u16),
    MulFloat(u16, u16, u16),
    MulInt(u16, u16, u16),
    SubFloat(u16, u16, u16),
    SubInt(u16, u16, u16),
    DivFloat(u16, u16, u16),
    DivInt(u16, u16, u16),
    ModFloat(u16, u16, u16),
    ModInt(u16, u16, u16),
    PowFloat(u16, u16, u16),
    PowInt(u16, u16, u16),
    /// Increments the integer in-place by 1
    IncInt(u16),
    /// Decrements the integer in-place by 1
    DecInt(u16),
    /// IncIntTo(src, dst)\
    /// dst = src + 1
    IncIntTo(u16, u16),
    /// DecIntTo(src, dst)\
    /// dst = src - 1
    DecIntTo(u16, u16),
    Eq(u16, u16, u16),
    NotEq(u16, u16, u16),
    ObjEq(u16, u16, u16),
    ObjNotEq(u16, u16, u16),
    StrEq(u16, u16, u16),
    StrNotEq(u16, u16, u16),
    SupFloat(u16, u16, u16),
    SupInt(u16, u16, u16),
    SupEqFloat(u16, u16, u16),
    SupEqInt(u16, u16, u16),
    InfFloat(u16, u16, u16),
    InfInt(u16, u16, u16),
    InfEqFloat(u16, u16, u16),
    InfEqInt(u16, u16, u16),
    BoolAnd(u16, u16, u16),
    BoolOr(u16, u16, u16),
    NegBool(u16, u16),
    NegFloat(u16, u16),
    NegInt(u16, u16),

    /// CallFunc(n,y)\
    /// Jumps to the nth instruction, and adds y as a slot to be set by the Return instruction\
    /// VoidReturn/Return will jump back to this location
    CallFunc(u16, u16),
    /// CallFuncRecursive(n, register_id)\
    /// Jumps to the nth instruction, register_id is only used during parsing.
    CallFuncRecursive(u16, u16),
    /// Jumps to the instruction right after the last CallFunc encountered by the interpreter
    VoidReturn,
    /// Return(n)\
    /// Returns the data located in register n
    Return(u16),
    /// RecursiveReturn(n,function_id)\
    /// Returns the data located in register n, and restores the function's register state
    RecursiveReturn(u16),
    /// SaveFrame(function_location, return_register, function_id)
    SaveFrame(u16, u16, u16),

    /// CallDynamicLibFunc(fn_id, dest_register_id)
    CallDynamicLibFunc(u16, u16),

    /// CallHostFunc(host_fn_id, dest_register_id)
    /// Dispatches to a Rust closure registered on the embedding `Engine`,
    /// marshalling `StoreFuncArg` operands into the closure and its result back
    /// into `dest_register_id`.
    CallHostFunc(u16, u16),

    StoreFuncArg(u16),
    /// CallLibFunc(function, src_register_id, dest_register_id)
    CallLibFunc(LibFunc, u16, u16),
    /// CallLibFuncVoid(function, src_register_1, src_register_2)
    /// For single-source ops, src register 2 is unused
    CallLibFuncVoid(LibFuncVoid, u16, u16),

    /// StartErrorCatch(jump_size, error_register_id)
    StartErrorCatch(u16, u16),
    StopErrorCatch,

    /// ThrowError(error_register_id)
    /// Throws the error (a string) located in error_register_id
    ThrowError(u16),

    /// ArrayMov(new_elem_reg_id, array_id, idx)\
    /// Replaces the idx-th element in the array (with the id array_id) with the element located in new_elem_reg_id
    ObjElemMov(u16, u16, u16),

    /// EmptyArray(array_reg_id)
    /// Allocates a fresh empty array and stores its address in array_reg_id
    EmptyArray(u16),

    /// CloneArray(src_reg, dest_reg, len)
    /// Allocates a fresh array with exact capacity len and clones the array in src_reg to dest_reg
    CloneArray(u16, u16, u16),

    CloneStruct(u16, u16),

    /// CloneEnum(template_reg, dest_reg)
    /// Clones the enum value in `template_reg` into a fresh object-pool entry,
    /// preserving the enum type tag. Mirrors `CloneStruct` for enum values.
    CloneEnum(u16, u16),

    /// SetElementArray(array_reg_id, new_elem_reg_id, idx)\
    /// Replaces the idx-th element in the array located in array_reg_id with the element located in new_elem_reg_id
    SetElementObj(u16, u16, u16),

    /// SetElementString(string_reg_id, new_str_reg_id, idx)\
    /// Replaces the idx-th char in the string located in string_reg_id with the string located in new_str_reg_id
    SetElementString(u16, u16, u16),

    /// SetFieldStruct(struct_reg_id, new_elem_reg_id, idx)
    SetFieldStruct(u16, u16, u16),

    /// GetIndexArray(array_reg_id, index_reg_id, output_reg_id)
    GetIndexArray(u16, u16, u16),

    /// GetIndexString(str_reg_id, index_reg_id, output_reg_id)
    GetIndexString(u16, u16, u16),

    /// GetFieldStruct(struct_reg_id, index, output_reg_id)
    GetFieldStruct(u16, u16, u16),

    /// GetSliceArray(array_reg_id, idx_start_reg_id, output_reg_id)\
    /// idx_end_reg_id is pulled from a StoreFuncArg
    GetSliceArray(u16, u16, u16),
    /// GetSliceString(str_reg_id, idx_start_reg_id, output_reg_id)\
    /// idx_end_reg_id is pulled from a StoreFuncArg
    GetSliceString(u16, u16, u16),

    /// Push(array_reg_id, elem_reg_id)
    Push(u16, u16),

    /// Remove(array_reg_id, elem_index_reg_id)
    Remove(u16, u16),

    /// MapGet(map_pool_id, key_reg_id, dest_reg_id)
    MapGet(u16, u16, u16),
    /// MapSet(map_pool_id, key_reg_id, val_reg_id)
    MapInsert(u16, u16, u16),
    /// MapSet(map_reg_id, key_reg_id, val_reg_id)
    MapInsertReg(u16, u16, u16),
    CloneMap(u16, u16),

    /// Exits the program with the i32 code if it's != 0
    Halt(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibFunc {
    Uppercase = 0,
    Lowercase = 1,
    Contains = 2,
    Trim = 3,
    TrimSequence = 4,
    Find = 5,
    IsFloat = 6,
    IsInt = 7,
    TrimLeft = 8,
    TrimRight = 9,
    TrimSequenceLeft = 10,
    TrimSequenceRight = 11,
    Repeat = 12,
    Round = 13,
    Abs = 14,
    Argv = 15,
    SqrtFloat = 16,
    Float = 17,
    Int = 18,
    Str = 19,
    Bool = 20,
    Input = 21,
    Floor = 22,
    TheAnswer = 23,
    Len = 24,
    StartsWith = 25,
    EndsWith = 26,
    Replace = 27,
    Split = 28,
    Range = 29,
    JoinStringArray = 30,
    Reverse = 31,
    FsRead = 32,
    FsExists = 33,
    /// Map key array. Reads the map in the source register and emits a fresh
    /// array of its keys.
    Keys = 34,
    /// Map value array. Reads the map in the source register and emits a fresh
    /// array of its values.
    Values = 35,
    /// json::parse. Parses the source string into a value graph (maps, arrays,
    /// scalars); a malformed string raises a catchable error.
    JsonParse = 36,
    /// json::stringify. Serializes the source value to a json string.
    JsonStringify = 37,
    // Runtime type tests on an `any` value: read the source register's tag and
    // emit a bool. The `Val` suffix separates these from the string-parsing
    // `IsInt`/`IsFloat` above.
    IsIntVal = 38,
    IsFloatVal = 39,
    IsStrVal = 40,
    IsBoolVal = 41,
    IsListVal = 42,
    IsMapVal = 43,
    IsNullVal = 44,
    // Checked downcasts of an `any` value to a concrete type: pass the value
    // through when the tag matches, otherwise raise a catchable error. The
    // compiler types the result as the target type so it can be used directly.
    AsIntVal = 45,
    AsFloatVal = 46,
    AsStrVal = 47,
    AsBoolVal = 48,
    AsListVal = 49,
    AsMapVal = 50,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibFuncVoid {
    Reverse = 0,
    FsWrite = 1,
    FsAppend = 2,
    FsDelete = 3,
    FsDeleteDir = 4,
    Sort = 5,
}

impl Instr {
    /// Returns the ID of the register that will be modified by the given instruction
    #[must_use]
    pub const fn get_tgt_id(self) -> Option<u16> {
        match self {
            // Instructions that modify no register.
            Self::Print(_)
            | Self::Jmp(_)
            | Self::JmpBack(_)
            | Self::IsFalseJmp(_, _)
            | Self::IsTrueJmp(_, _)
            | Self::NotEqJmp(_, _, _)
            | Self::ObjNotEqJmp(_, _, _)
            | Self::StrNotEqJmp(_, _, _)
            | Self::EqJmp(_, _, _)
            | Self::ObjEqJmp(_, _, _)
            | Self::StrEqJmp(_, _, _)
            | Self::SupFloatJmp(_, _, _)
            | Self::SupIntJmp(_, _, _)
            | Self::SupEqFloatJmp(_, _, _)
            | Self::SupEqIntJmp(_, _, _)
            | Self::InfEqFloatJmp(_, _, _)
            | Self::InfEqIntJmp(_, _, _)
            | Self::InfFloatJmp(_, _, _)
            | Self::InfIntJmp(_, _, _)
            | Self::InfIntJmpBack(_, _, _)
            | Self::StoreFuncArg(_)
            | Self::SetElementObj(_, _, _)
            | Self::SetFieldStruct(_, _, _)
            | Self::MapInsert(_, _, _)
            | Self::MapInsertReg(_, _, _)
            | Self::ObjElemMov(_, _, _)
            | Self::Push(_, _)
            | Self::Return(_) // Modifies a register, but this function doesn't know which one
            | Self::RecursiveReturn(_) // Modifies a register, but this function doesn't know which one
            | Self::VoidReturn
            | Self::Remove(_, _)
            | Self::CallLibFuncVoid(_, _, _)
            | Self::Halt(_)
            | Self::StopErrorCatch
            | Self::ThrowError(_)
            => None,

            Self::StartErrorCatch(_, y) if y == u16::MAX => None,

            Self::Mov(_, y)
            | Self::SetInt(y, _)
            | Self::SetBool(_, y)
            | Self::CallFunc(_, y)
            | Self::CallFuncRecursive(_, y)
            | Self::SaveFrame(_, y, _)
            | Self::AddFloat(_, _, y)
            | Self::AddInt(_, _, y)
            | Self::AddArray(_, _, y)
            | Self::AddStr(_, _, y)
            | Self::MulFloat(_, _, y)
            | Self::MulInt(_, _, y)
            | Self::SubFloat(_, _, y)
            | Self::SubInt(_, _, y)
            | Self::DivFloat(_, _, y)
            | Self::DivInt(_, _, y)
            | Self::ModFloat(_, _, y)
            | Self::ModInt(_, _, y)
            | Self::PowFloat(_, _, y)
            | Self::PowInt(_, _, y)
            | Self::Eq(_, _, y)
            | Self::ObjEq(_, _, y)
            | Self::StrEq(_, _, y)
            | Self::NotEq(_, _, y)
            | Self::ObjNotEq(_, _, y)
            | Self::StrNotEq(_, _, y)
            | Self::SupFloat(_, _, y)
            | Self::SupInt(_, _, y)
            | Self::SupEqFloat(_, _, y)
            | Self::SupEqInt(_, _, y)
            | Self::InfFloat(_, _, y)
            | Self::InfInt(_, _, y)
            | Self::InfEqFloat(_, _, y)
            | Self::InfEqInt(_, _, y)
            | Self::BoolAnd(_, _, y)
            | Self::BoolOr(_, _, y)
            | Self::NegBool(_, y)
            | Self::EmptyArray(y)
            | Self::NegFloat(_, y)
            | Self::NegInt(_, y)
            | Self::CallLibFunc(_, _, y)
            | Self::GetIndexArray(_, _, y)
            | Self::GetFieldStruct(_, _, y)
            | Self::MapGet(_, _, y)
            | Self::GetSliceArray(_, _, y)
            | Self::GetIndexString(_, _, y)
            | Self::GetSliceString(_, _, y)
            | Self::SetElementString(y, _, _)
            | Self::CallDynamicLibFunc(_, y)
            | Self::CallHostFunc(_, y)
            | Self::IncInt(y)
            | Self::DecInt(y)
            | Self::IncIntTo(_, y)
            | Self::DecIntTo(_, y)
            | Self::StartErrorCatch(_,y)
            | Self::CloneStruct(_, y)
            | Self::CloneEnum(_, y)
            | Self::CloneMap(_, y)
            | Self::CloneArray(_, y, _) => Some(y),
        }
    }

    pub fn for_each_read_reg(self, mut f: impl FnMut(u16)) {
        match self {
            Self::AddFloat(a, b, _)
            | Self::AddInt(a, b, _)
            | Self::AddArray(a, b, _)
            | Self::AddStr(a, b, _)
            | Self::MulFloat(a, b, _)
            | Self::MulInt(a, b, _)
            | Self::SubFloat(a, b, _)
            | Self::SubInt(a, b, _)
            | Self::DivFloat(a, b, _)
            | Self::DivInt(a, b, _)
            | Self::ModFloat(a, b, _)
            | Self::ModInt(a, b, _)
            | Self::PowFloat(a, b, _)
            | Self::PowInt(a, b, _)
            | Self::Eq(a, b, _)
            | Self::NotEq(a, b, _)
            | Self::ObjEq(a, b, _)
            | Self::ObjNotEq(a, b, _)
            | Self::StrEq(a, b, _)
            | Self::StrNotEq(a, b, _)
            | Self::SupFloat(a, b, _)
            | Self::SupInt(a, b, _)
            | Self::SupEqFloat(a, b, _)
            | Self::SupEqInt(a, b, _)
            | Self::InfFloat(a, b, _)
            | Self::InfInt(a, b, _)
            | Self::InfEqFloat(a, b, _)
            | Self::InfEqInt(a, b, _)
            | Self::BoolAnd(a, b, _)
            | Self::BoolOr(a, b, _)
            | Self::GetIndexArray(a, b, _)
            | Self::GetSliceArray(a, b, _)
            | Self::GetIndexString(a, b, _)
            | Self::GetSliceString(a, b, _)
            | Self::NotEqJmp(a, b, _)
            | Self::EqJmp(a, b, _)
            | Self::ObjNotEqJmp(a, b, _)
            | Self::ObjEqJmp(a, b, _)
            | Self::StrNotEqJmp(a, b, _)
            | Self::StrEqJmp(a, b, _)
            | Self::SupFloatJmp(a, b, _)
            | Self::SupIntJmp(a, b, _)
            | Self::SupEqFloatJmp(a, b, _)
            | Self::SupEqIntJmp(a, b, _)
            | Self::InfFloatJmp(a, b, _)
            | Self::InfIntJmp(a, b, _)
            | Self::InfEqFloatJmp(a, b, _)
            | Self::InfEqIntJmp(a, b, _)
            | Self::InfIntJmpBack(a, b, _)
            | Self::Push(a, b)
            | Self::SetFieldStruct(a, b, _)
            | Self::MapGet(a, b, _)
            | Self::MapInsert(_, a, b)
            | Self::Remove(a, b) => {
                f(a);
                f(b);
            }

            Self::SetElementObj(a, b, c)
            | Self::SetElementString(a, b, c)
            | Self::MapInsertReg(a, b, c) => {
                f(a);
                f(b);
                f(c);
            }

            Self::Mov(a, _)
            | Self::IncInt(a)
            | Self::DecInt(a)
            | Self::IncIntTo(a, _)
            | Self::DecIntTo(a, _)
            | Self::NegFloat(a, _)
            | Self::NegInt(a, _)
            | Self::CallLibFunc(_, a, _)
            | Self::Print(a)
            | Self::StoreFuncArg(a)
            | Self::Return(a)
            | Self::RecursiveReturn(a)
            | Self::IsFalseJmp(a, _)
            | Self::IsTrueJmp(a, _)
            | Self::ThrowError(a)
            | Self::GetFieldStruct(a, _, _)
            | Self::NegBool(a, _)
            | Self::ObjElemMov(a, _, _) => f(a),

            Self::CallLibFuncVoid(func, a, b) => {
                f(a);
                if matches!(func, LibFuncVoid::FsWrite | LibFuncVoid::FsAppend) {
                    f(b);
                }
            }
            Self::Halt(x) if x != 0 => f(x),

            Self::CloneArray(src, _, _)
            | Self::CloneStruct(src, _)
            | Self::CloneEnum(src, _)
            | Self::CloneMap(src, _) => {
                f(src);
            }

            Self::Halt(_)
            | Self::Jmp(_)
            | Self::JmpBack(_)
            | Self::VoidReturn
            | Self::CallFunc(_, _)
            | Self::CallFuncRecursive(_, _)
            | Self::SaveFrame(_, _, _)
            | Self::CallDynamicLibFunc(_, _)
            | Self::CallHostFunc(_, _)
            | Self::EmptyArray(_)
            | Self::SetInt(_, _)
            | Self::StartErrorCatch(_, _)
            | Self::StopErrorCatch
            | Self::SetBool(_, _) => {}
        }
    }
}
