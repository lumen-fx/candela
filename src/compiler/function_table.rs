//! The function table a `.cdlb` carries after its image: for every compiled
//! function, where it starts, where its parameters arrive and what type they
//! are, what it returns, and the type of every value its instructions write.
//!
//! Code generation reads types from here instead of working them out of the
//! bytecode. A register the compiler reuses for values of different types
//! within one function gets the right type at each write, because each write
//! is typed on its own.
//!
//! The table is built once the program is final, after the release passes
//! have moved, merged and dropped instructions, so the positions it records
//! are the ones the artifact holds. Each function's writes are typed forward
//! from what the compiler settled for it: its parameter types, the return
//! type of every specialisation it calls, the declared fields of each struct,
//! and the signatures of `dylib` and `host` functions.

use crate::compiler::CompileOutput;
use crate::compiler::flow::body;
use crate::compiler::flow::successors;
use crate::compiler::type_system::arg_types_specialize_equal;
use crate::compiler::type_system::specialization_key;
use crate::data::Data;
use crate::instr::Instr;
use crate::instr::LibFunc;
use candela_vm::artifact::DefImage;
use candela_vm::artifact::FunctionImage;
use candela_vm::artifact::FunctionTable;
use candela_vm::rt::DataType;
use rustc_hash::FxHashMap;

/// The table for a finished compile, `main` first.
#[must_use]
pub fn function_table(out: &CompileOutput) -> FunctionTable {
    let program = Program::new(out);
    let mut functions = Vec::new();
    // `main` is compiled in place at the top of the program.
    if out
        .functions
        .iter()
        .any(|f| f.name == "main" && f.src_file == 0)
    {
        functions.push(program.function("main", 0, &[], &[], DataType::Null, None, false));
    }
    for function in &out.functions {
        for fn_impl in &function.impls {
            let key = specialization_key(&fn_impl.type_args, &fn_impl.arg_types);
            let returns = function
                .return_type_cache
                .iter()
                .find(|(args, _)| arg_types_specialize_equal(args, &key))
                .map_or(DataType::Unknown, |(_, ty)| ty.clone());
            functions.push(program.function(
                &function.name,
                fn_impl.loc as usize,
                &fn_impl.args_loc,
                &fn_impl.arg_types,
                returns,
                fn_impl.env_loc,
                fn_impl.indirect,
            ));
        }
    }
    FunctionTable { functions }
}

/// What typing a function's writes reads from the rest of the program.
struct Program<'a> {
    out: &'a CompileOutput,
    /// What each function returns, by the instruction it starts at.
    returns: FxHashMap<usize, DataType>,
    /// The registers no instruction writes: they hold the value the compiler
    /// put in them for the whole run.
    constant: Vec<bool>,
}

/// The type every register holds at one point of a function.
type Types = FxHashMap<u16, DataType>;

impl<'a> Program<'a> {
    fn new(out: &'a CompileOutput) -> Self {
        let mut returns = FxHashMap::default();
        for function in &out.functions {
            for fn_impl in &function.impls {
                let key = specialization_key(&fn_impl.type_args, &fn_impl.arg_types);
                if let Some((_, ty)) = function
                    .return_type_cache
                    .iter()
                    .find(|(args, _)| arg_types_specialize_equal(args, &key))
                {
                    returns.insert(fn_impl.loc as usize, ty.clone());
                }
            }
        }
        let mut constant = vec![true; out.registers.len()];
        let mut written = |reg: u16| {
            if let Some(slot) = constant.get_mut(reg as usize) {
                *slot = false;
            }
        };
        for instr in &out.instructions {
            instr.for_each_write_reg(&mut written);
        }
        // A caller writes a function's parameters, even one every call site
        // inlined.
        for function in &out.functions {
            for fn_impl in &function.impls {
                for reg in fn_impl.args_loc.iter().chain(&fn_impl.env_loc) {
                    written(*reg);
                }
            }
        }
        Self {
            out,
            returns,
            constant,
        }
    }

    /// The table entry for one function.
    fn function(
        &self,
        name: &str,
        entry: usize,
        params: &[u16],
        param_types: &[DataType],
        returns: DataType,
        env: Option<u16>,
        indirect: bool,
    ) -> FunctionImage {
        let instructions = &self.out.instructions;
        let body = body(instructions, entry);
        let mut entry_types = Types::default();
        for (reg, ty) in params.iter().zip(param_types) {
            entry_types.insert(*reg, ty.clone());
        }

        // Forward over the body until the types at the start of every
        // instruction settle. A register two paths give different types to
        // is `Unknown` where they meet.
        let mut at: FxHashMap<usize, Types> = FxHashMap::default();
        at.insert(entry, entry_types);
        let mut work = vec![entry];
        while let Some(pos) = work.pop() {
            let mut types = at[&pos].clone();
            self.step(pos, instructions[pos], &mut types, &mut |_, _| {});
            for next in successors(pos, instructions[pos]).into_iter().flatten() {
                if next >= instructions.len() {
                    continue;
                }
                let changed = match at.get_mut(&next) {
                    None => {
                        at.insert(next, types.clone());
                        true
                    }
                    Some(existing) => merge(existing, &types),
                };
                if changed {
                    work.push(next);
                }
            }
        }

        let mut defs = Vec::new();
        for pos in body {
            let Some(types) = at.get(&pos) else { continue };
            let mut types = types.clone();
            self.step(pos, instructions[pos], &mut types, &mut |reg, ty| {
                defs.push(DefImage {
                    at: pos as u32,
                    reg,
                    ty: ty.clone(),
                });
            });
        }

        FunctionImage {
            name: name.to_owned(),
            entry: entry as u32,
            params: params.to_vec(),
            param_types: param_types.to_vec(),
            returns,
            env,
            indirect,
            defs,
        }
    }

    /// The type register `reg` holds, given the types at this point.
    fn type_of(&self, types: &Types, reg: u16) -> DataType {
        if let Some(ty) = types.get(&reg) {
            return ty.clone();
        }
        if self.constant.get(reg as usize).copied().unwrap_or(false) {
            return constant_type(self.out.registers[reg as usize]);
        }
        DataType::Unknown
    }

    /// Applies the instruction at `pos` to `types`, and hands `def` each
    /// register it writes with the type of what it writes there.
    fn step(
        &self,
        pos: usize,
        instr: Instr,
        types: &mut Types,
        def: &mut dyn FnMut(u16, &DataType),
    ) {
        let t = |reg: u16| self.type_of(types, reg);
        let written: Vec<(u16, DataType)> = match instr {
            Instr::Mov(src, dest) => vec![(dest, t(src))],
            // The second move reads what the first wrote when `src` is `dest`.
            Instr::MovChain(dest, mid, src) => {
                let second = if src == dest { t(mid) } else { t(src) };
                vec![(dest, t(mid)), (mid, second)]
            }
            Instr::SetInt(dest, _)
            | Instr::AddInt(_, _, dest)
            | Instr::AddIntImm(_, _, dest)
            | Instr::SubInt(_, _, dest)
            | Instr::MulInt(_, _, dest)
            | Instr::DivInt(_, _, dest)
            | Instr::ModInt(_, _, dest)
            | Instr::PowInt(_, _, dest)
            | Instr::BitAndInt(_, _, dest)
            | Instr::BitOrInt(_, _, dest)
            | Instr::BitXorInt(_, _, dest)
            | Instr::BitNotInt(_, dest)
            | Instr::ShlInt(_, _, dest)
            | Instr::ShrInt(_, _, dest)
            | Instr::IncInt(dest)
            | Instr::DecInt(dest)
            | Instr::IncIntTo(_, dest)
            | Instr::DecIntTo(_, dest)
            | Instr::NegInt(_, dest)
            | Instr::StepIntJmpBack(dest, _, _) => vec![(dest, DataType::Int)],
            Instr::AddFloat(_, _, dest)
            | Instr::SubFloat(_, _, dest)
            | Instr::MulFloat(_, _, dest)
            | Instr::DivFloat(_, _, dest)
            | Instr::ModFloat(_, _, dest)
            | Instr::PowFloat(_, _, dest)
            | Instr::NegFloat(_, dest) => vec![(dest, DataType::Float)],
            Instr::SetBool(_, dest)
            | Instr::Eq(_, _, dest)
            | Instr::NotEq(_, _, dest)
            | Instr::ObjEq(_, _, dest)
            | Instr::ObjNotEq(_, _, dest)
            | Instr::StrEq(_, _, dest)
            | Instr::StrNotEq(_, _, dest)
            | Instr::SupFloat(_, _, dest)
            | Instr::SupInt(_, _, dest)
            | Instr::SupEqFloat(_, _, dest)
            | Instr::SupEqInt(_, _, dest)
            | Instr::InfFloat(_, _, dest)
            | Instr::InfInt(_, _, dest)
            | Instr::InfEqFloat(_, _, dest)
            | Instr::InfEqInt(_, _, dest)
            | Instr::SupStr(_, _, dest)
            | Instr::SupEqStr(_, _, dest)
            | Instr::InfStr(_, _, dest)
            | Instr::InfEqStr(_, _, dest)
            | Instr::BoolAnd(_, _, dest)
            | Instr::BoolOr(_, _, dest)
            | Instr::NegBool(_, dest) => vec![(dest, DataType::Bool)],
            Instr::AddStr(_, _, dest)
            | Instr::GetIndexString(_, _, dest)
            | Instr::GetSliceString(_, _, dest)
            | Instr::SetElementString(dest, _, _) => vec![(dest, DataType::String)],
            Instr::StartErrorCatch(_, dest) if dest != u16::MAX => vec![(dest, DataType::String)],
            Instr::AddArray(a, b, dest) => {
                let ty = match t(a) {
                    DataType::Array(None) | DataType::Unknown => t(b),
                    ty => ty,
                };
                vec![(dest, ty)]
            }
            Instr::GetSliceArray(list, _, dest)
            | Instr::CloneArray(list, dest, _)
            | Instr::CloneStruct(list, dest)
            | Instr::CloneEnum(list, dest)
            | Instr::CloneMap(list, dest) => vec![(dest, t(list))],
            Instr::EmptyArray(dest) => vec![(dest, DataType::Array(None))],
            Instr::EmptyFnValue(dest) => vec![(dest, DataType::Unknown)],
            Instr::NewCell(src, dest) => vec![(dest, DataType::Array(Some(Box::new(t(src)))))],
            Instr::GetIndexArray(list, _, dest) | Instr::LoadCell(list, dest) => {
                vec![(dest, element_type(&t(list)))]
            }
            Instr::GetFieldStruct(obj, field, dest) => {
                vec![(dest, self.field_type(&t(obj), field))]
            }
            Instr::MapGet(map, _, dest) => {
                let ty = match t(map) {
                    DataType::Map(entry) => entry.1.clone().unwrap_or(DataType::Unknown),
                    _ => DataType::Unknown,
                };
                vec![(dest, ty)]
            }
            Instr::CallFunc(loc, dest) | Instr::CallFuncRecursive(loc, dest) => {
                vec![(dest, self.returns_of(loc as usize))]
            }
            // The register the call's value comes back in. The call itself,
            // `offset` further on, writes it on the way back.
            Instr::SaveFrame(offset, dest, _) => {
                let ty = match self.out.instructions.get(pos + offset as usize) {
                    Some(Instr::CallFuncRecursive(loc, _)) => self.returns_of(*loc as usize),
                    _ => DataType::Unknown,
                };
                vec![(dest, ty)]
            }
            Instr::CallDynamicLibFunc(id, dest) => {
                let ty = self
                    .out
                    .dyn_lib_fns
                    .get(id as usize)
                    .and_then(|f| f.types.first().cloned())
                    .unwrap_or(DataType::Unknown);
                vec![(dest, ty)]
            }
            Instr::CallHostFunc(id, dest) => {
                let ty = self
                    .out
                    .host_fns
                    .get(id as usize)
                    .and_then(|f| f.types.first().cloned())
                    .unwrap_or(DataType::Unknown);
                vec![(dest, ty)]
            }
            Instr::CallLibFunc(func, src, dest) => vec![(dest, lib_func_type(func, t(src)))],
            other => {
                // Everything else writes nothing, or a register this table
                // cannot type.
                let mut written = Vec::new();
                other.for_each_write_reg(|reg| written.push((reg, DataType::Unknown)));
                written
            }
        };
        for (reg, ty) in written {
            def(reg, &ty);
            types.insert(reg, ty);
        }
    }

    /// What the function starting at `loc` returns.
    fn returns_of(&self, loc: usize) -> DataType {
        self.returns.get(&loc).cloned().unwrap_or(DataType::Unknown)
    }

    /// The declared type of field `field` of a value of type `ty`.
    fn field_type(&self, ty: &DataType, field: u16) -> DataType {
        match ty {
            DataType::Struct(id) => self
                .out
                .structs
                .get(*id as usize)
                .and_then(|s| s.fields.get(field as usize))
                .map_or(DataType::Unknown, |(_, ty, _)| ty.clone()),
            _ => DataType::Unknown,
        }
    }
}

/// The type of a value the compiler placed in a register.
fn constant_type(value: Data) -> DataType {
    if value.is_int() {
        DataType::Int
    } else if value.is_float() {
        DataType::Float
    } else if value.is_bool() {
        DataType::Bool
    } else if value.is_string() {
        DataType::String
    } else if value.is_null() {
        DataType::Null
    } else if value.is_function() {
        DataType::Unknown
    } else if value.is_array() {
        DataType::Array(None)
    } else if value.is_map() {
        DataType::Map(Box::new((None, None)))
    } else if value.is_struct() {
        DataType::Struct(value.struct_type_id())
    } else if value.is_enum() {
        DataType::Enum(value.enum_type_id())
    } else {
        DataType::Unknown
    }
}

/// What a list of type `ty` holds.
fn element_type(ty: &DataType) -> DataType {
    match ty {
        DataType::Array(Some(element)) => (**element).clone(),
        _ => DataType::Unknown,
    }
}

/// What the library function `func` returns for an argument of type `arg`.
fn lib_func_type(func: LibFunc, arg: DataType) -> DataType {
    match func {
        LibFunc::Uppercase
        | LibFunc::Lowercase
        | LibFunc::Trim
        | LibFunc::TrimSequence
        | LibFunc::TrimLeft
        | LibFunc::TrimRight
        | LibFunc::TrimSequenceLeft
        | LibFunc::TrimSequenceRight
        | LibFunc::Replace
        | LibFunc::Str
        | LibFunc::JoinStringArray
        | LibFunc::Reverse
        | LibFunc::FsRead
        | LibFunc::Input
        | LibFunc::JsonStringify
        | LibFunc::AsStrVal => DataType::String,
        LibFunc::Contains
        | LibFunc::IsFloat
        | LibFunc::IsInt
        | LibFunc::StartsWith
        | LibFunc::EndsWith
        | LibFunc::FsExists
        | LibFunc::IsIntVal
        | LibFunc::IsFloatVal
        | LibFunc::IsStrVal
        | LibFunc::IsBoolVal
        | LibFunc::IsListVal
        | LibFunc::IsMapVal
        | LibFunc::IsNullVal
        | LibFunc::Bool
        | LibFunc::AsBoolVal => DataType::Bool,
        LibFunc::Find | LibFunc::Len | LibFunc::TheAnswer | LibFunc::Int | LibFunc::AsIntVal => {
            DataType::Int
        }
        LibFunc::Float
        | LibFunc::SqrtFloat
        | LibFunc::Round
        | LibFunc::Floor
        | LibFunc::AsFloatVal => DataType::Float,
        LibFunc::Abs | LibFunc::Repeat => arg,
        LibFunc::Argv | LibFunc::Split => DataType::Array(Some(Box::new(DataType::String))),
        LibFunc::Range => DataType::Array(Some(Box::new(DataType::Int))),
        LibFunc::Keys => match arg {
            DataType::Map(entry) => DataType::Array(entry.0.clone().map(Box::new)),
            _ => DataType::Array(None),
        },
        LibFunc::Values => match arg {
            DataType::Map(entry) => DataType::Array(entry.1.clone().map(Box::new)),
            _ => DataType::Array(None),
        },
        LibFunc::AsListVal => DataType::Array(None),
        LibFunc::AsMapVal => DataType::Map(Box::new((None, None))),
        LibFunc::JsonParse | LibFunc::AsTypeVal => DataType::Unknown,
    }
}

/// Joins what reaches an instruction along one more path into what reached
/// it before. Answers whether anything changed.
fn merge(existing: &mut Types, incoming: &Types) -> bool {
    let mut changed = false;
    for (reg, ty) in incoming {
        match existing.get_mut(reg) {
            None => {
                existing.insert(*reg, ty.clone());
                changed = true;
            }
            Some(old) if old != ty && *old != DataType::Unknown => {
                *old = DataType::Unknown;
                changed = true;
            }
            Some(_) => {}
        }
    }
    changed
}
