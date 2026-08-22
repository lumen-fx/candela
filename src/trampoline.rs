//! Call trampolines: the short instruction run that lets a Rust host invoke a
//! script function by name.
//!
//! A trampoline moves the call's arguments into the registers the target
//! specialisation expects, calls it, and leaves the result in one register. It
//! is compiled onto the end of the program's instruction stream, so running it
//! means executing from its entry index against the resident register and heap
//! state.
//!
//! Both halves of the toolchain build them here. `Engine`/`Program` compiles
//! one per call, with the arguments known; `candela build` compiles one per
//! exported function ahead of time, with the arguments still to come, and
//! records where it starts in the `.cdlb` export table.

use crate::compiler::compiler_data::Ctx;
use crate::compiler::compiler_data::State;
use crate::compiler::compiler_data::Variable;
use crate::compiler::expr::Expr;
use crate::instr::Instr;

/// Compiles a call trampoline for `call_expr`, specialising the target function
/// for the argument types if that has not happened yet.
///
/// `offset` is the index the returned run will sit at once appended, which the
/// emitted jumps and call targets are absolute against. `seed_vars` are
/// pre-allocated registers standing in for arguments the expression refers to
/// by name. Returns the instruction run and the register holding the result,
/// which is `None` for a function that returns nothing.
pub fn compile_trampoline(
    state: &mut State<'_>,
    offset: u16,
    call_expr: &Expr,
    seed_vars: Vec<Variable>,
) -> (Vec<Instr>, Option<u16>) {
    // A prior trampoline that aborted mid-inference (error unwind) may have
    // left stale entries in the return-type inference thread-local; clear it so
    // this compile starts clean, exactly as `compile()` does.
    crate::compiler::type_system::reset_inference_state();

    let ctx = Ctx {
        block_id: 0,
        is_compiling_recursive: false,
        single_run: false,
        in_function: false,
        file_idx: 0,
        offset,
    };
    let mut variables = seed_vars;
    let mut output = Vec::new();
    let ret = call_expr.compile(&mut variables, ctx, state, &mut output, None, false, true);
    (output, ret)
}
