//! Inlines a callee's early return at its native call sites.
//!
//! Many functions open with a test on their parameters that returns at once
//! when it holds: the base case of a recursion, a guard. A call that takes
//! that exit pays for a whole frame to run a comparison. This pass copies
//! the test and the early return into each native call site, so a call whose
//! arguments take the exit never calls.
//!
//! The copy runs only where the call itself could have run as native code:
//! the call depth and the stack must have room for its frame. Otherwise the
//! call happens as before and meets the limit where it always did.

use super::ir::Block;
use super::ir::BlockId;
use super::ir::Cond;
use super::ir::Func;
use super::ir::Kind;
use super::ir::Module;
use super::ir::Op;
use super::ir::Operand;
use super::ir::Term;
use super::ir::Var;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// The most operations the copied test and return may take.
const MAX_OPS: usize = 8;

/// The part of a function a call site copies.
struct EarlyReturn {
    params: Vec<Var>,
    vars: Vec<Kind>,
    /// The operations before the test.
    test_ops: Vec<Op>,
    cond: Cond,
    /// Whether the early return is where the test goes when it holds.
    on_true: bool,
    /// The operations of the early return, and what it returns.
    return_ops: Vec<Op>,
    value: Option<Operand>,
}

/// Inlines the early return of every native callee that has one.
pub fn inline_early_returns(module: &mut Module) {
    let early: Vec<Option<EarlyReturn>> = module.funcs.iter().map(early_return).collect();
    for func in &mut module.funcs {
        inline_in(func, &early);
    }
}

/// Operations a copy can run without a frame: nothing that calls, stores or
/// fails.
const fn pure(op: &Op) -> bool {
    matches!(
        op,
        Op::Copy { .. }
            | Op::Int { .. }
            | Op::Float { .. }
            | Op::Unary { .. }
            | Op::Compare { .. }
            | Op::BoolAnd { .. }
            | Op::BoolOr { .. }
    )
}

/// The early return `func` opens with, if it does.
fn early_return(func: &Func) -> Option<EarlyReturn> {
    let entry = func.blocks.first()?;
    if func.is_main || !entry.ops.iter().all(pure) {
        return None;
    }
    let Term::Branch { cond, then, other } = entry.term else {
        return None;
    };
    if matches!(cond, Cond::Room) {
        return None;
    }
    for (target, on_true) in [(then, true), (other, false)] {
        let block = &func.blocks[target as usize];
        if let Term::Return(value) = block.term
            && block.ops.iter().all(pure)
            && entry.ops.len() + block.ops.len() <= MAX_OPS
        {
            return Some(EarlyReturn {
                params: func.params.iter().map(|(_, v)| *v).collect(),
                vars: func.vars.clone(),
                test_ops: entry.ops.clone(),
                cond,
                on_true,
                return_ops: block.ops.clone(),
                value,
            });
        }
    }
    None
}

/// Copies the early returns into the calls of `func`.
fn inline_in(func: &mut Func, early: &[Option<EarlyReturn>]) {
    // Blocks holding a call this pass made, which it leaves alone.
    let mut done: FxHashSet<usize> = FxHashSet::default();
    // Each block split, with the blocks the split made.
    let mut order: Vec<(usize, [BlockId; 5])> = Vec::new();
    let mut b = 0;
    while b < func.blocks.len() {
        if done.contains(&b) {
            b += 1;
            continue;
        }
        let at = func.blocks[b]
            .ops
            .iter()
            .position(|op| matches!(op, Op::Call(call) if early[call.callee as usize].is_some()));
        let Some(at) = at else {
            b += 1;
            continue;
        };
        let after = func.blocks[b].ops.split_off(at + 1);
        let Some(Op::Call(call)) = func.blocks[b].ops.pop() else {
            unreachable!();
        };
        let template = early[call.callee as usize]
            .as_ref()
            .unwrap_or_else(|| unreachable!());
        let term = std::mem::replace(&mut func.blocks[b].term, Term::Halt);

        let next_id = |func: &Func, k: usize| (func.blocks.len() + k) as BlockId;
        let test = next_id(func, 0);
        let exit = next_id(func, 1);
        let calls = next_id(func, 2);
        let rest = next_id(func, 3);
        let no_room = next_id(func, 4);

        // The callee's variables, as fresh variables of this function.
        let mut map: FxHashMap<Var, Var> = FxHashMap::default();
        let mut fresh = |v: Var, func: &mut Func| -> Var {
            *map.entry(v).or_insert_with(|| {
                func.vars.push(template.vars[v as usize]);
                (func.vars.len() - 1) as Var
            })
        };

        let mut test_ops = Vec::new();
        for (param, arg) in template.params.iter().zip(&call.args) {
            test_ops.push(Op::Copy {
                dst: fresh(*param, func),
                src: *arg,
            });
        }
        for op in &template.test_ops {
            test_ops.push(map_op(op, &mut |v| fresh(v, func)));
        }
        let cond = map_cond(template.cond, &mut |v| fresh(v, func));
        let (then, other) = if template.on_true {
            (exit, calls)
        } else {
            (calls, exit)
        };

        let mut exit_ops: Vec<Op> = template
            .return_ops
            .iter()
            .map(|op| map_op(op, &mut |v| fresh(v, func)))
            .collect();
        if let (Some(dst), Some(value)) = (call.dst, template.value) {
            exit_ops.push(Op::Copy {
                dst,
                src: map_operand(value, &mut |v| fresh(v, func)),
            });
        }

        func.blocks[b].term = Term::Branch {
            cond: Cond::Room,
            then: test,
            other: no_room,
        };
        let cold = func.blocks[b].cold;
        func.blocks.push(Block {
            ops: test_ops,
            term: Term::Branch { cond, then, other },
            cold,
        });
        func.blocks.push(Block {
            ops: exit_ops,
            term: Term::Jump(rest),
            cold,
        });
        let mut slow = call.clone();
        let mut fast = call;
        fast.has_room = true;
        slow.has_room = false;
        func.blocks.push(Block {
            ops: vec![Op::Call(fast)],
            term: Term::Jump(rest),
            cold,
        });
        func.blocks.push(Block {
            ops: after,
            term,
            cold,
        });
        // Only a call standing at the depth limit or the edge of the stack
        // goes this way.
        func.blocks.push(Block {
            ops: vec![Op::Call(slow)],
            term: Term::Jump(rest),
            cold: true,
        });
        done.insert(calls as usize);
        done.insert(no_room as usize);
        order.push((b, [test, exit, calls, rest, no_room]));
        b += 1;
    }
    place_after(func, &order);
}

/// Moves the blocks each split made to right after the block it split, so
/// the code of a function stays in the order it runs.
fn place_after(func: &mut Func, splits: &[(usize, [BlockId; 5])]) {
    if splits.is_empty() {
        return;
    }
    let mut follows: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    let mut placed: FxHashSet<usize> = FxHashSet::default();
    for (b, new) in splits {
        // The block without room is rarely taken, so it goes last.
        let list: Vec<usize> = new[..4].iter().map(|n| *n as usize).collect();
        placed.extend(new.iter().map(|n| *n as usize));
        follows.insert(*b, list);
    }
    let mut order: Vec<usize> = Vec::with_capacity(func.blocks.len());
    fn visit(b: usize, follows: &FxHashMap<usize, Vec<usize>>, order: &mut Vec<usize>) {
        order.push(b);
        if let Some(list) = follows.get(&b) {
            for n in list {
                visit(*n, follows, order);
            }
        }
    }
    for b in 0..func.blocks.len() {
        if !placed.contains(&b) {
            visit(b, &follows, &mut order);
        }
    }
    for (_, new) in splits {
        order.push(new[4] as usize);
    }
    let mut new_id = vec![0 as BlockId; func.blocks.len()];
    for (k, b) in order.iter().enumerate() {
        new_id[*b] = k as BlockId;
    }
    let mut old: Vec<Option<Block>> = std::mem::take(&mut func.blocks)
        .into_iter()
        .map(Some)
        .collect();
    for b in order {
        let mut block = old[b].take().unwrap_or_else(|| unreachable!());
        match &mut block.term {
            Term::Jump(t) => *t = new_id[*t as usize],
            Term::Branch { then, other, .. } => {
                *then = new_id[*then as usize];
                *other = new_id[*other as usize];
            }
            Term::Return(_) | Term::Halt => {}
        }
        func.blocks.push(block);
    }
}

fn map_operand(operand: Operand, f: &mut impl FnMut(Var) -> Var) -> Operand {
    match operand {
        Operand::Var(v) => Operand::Var(f(v)),
        other => other,
    }
}

fn map_cond(cond: Cond, f: &mut impl FnMut(Var) -> Var) -> Cond {
    match cond {
        Cond::True(a) => Cond::True(map_operand(a, f)),
        Cond::False(a) => Cond::False(map_operand(a, f)),
        Cond::Compare(cmp, a, b) => Cond::Compare(cmp, map_operand(a, f), map_operand(b, f)),
        Cond::Room => Cond::Room,
    }
}

/// A pure operation with its variables renamed.
fn map_op(op: &Op, f: &mut impl FnMut(Var) -> Var) -> Op {
    match op.clone() {
        Op::Copy { dst, src } => Op::Copy {
            dst: f(dst),
            src: map_operand(src, f),
        },
        Op::Int { op, dst, a, b } => Op::Int {
            op,
            dst: f(dst),
            a: map_operand(a, f),
            b: map_operand(b, f),
        },
        Op::Float { op, dst, a, b } => Op::Float {
            op,
            dst: f(dst),
            a: map_operand(a, f),
            b: map_operand(b, f),
        },
        Op::Unary { op, dst, a } => Op::Unary {
            op,
            dst: f(dst),
            a: map_operand(a, f),
        },
        Op::Compare { cmp, dst, a, b } => Op::Compare {
            cmp,
            dst: f(dst),
            a: map_operand(a, f),
            b: map_operand(b, f),
        },
        Op::BoolAnd { dst, a, b } => Op::BoolAnd {
            dst: f(dst),
            a: map_operand(a, f),
            b: map_operand(b, f),
        },
        Op::BoolOr { dst, a, b } => Op::BoolOr {
            dst: f(dst),
            a: map_operand(a, f),
            b: map_operand(b, f),
        },
        other => other,
    }
}
