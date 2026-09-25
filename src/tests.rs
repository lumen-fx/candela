// This file is derived from keel (https://github.com/horacehoff/keel),
// Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
// It has been modified by the candela authors. See the NOTICE file.
use crate::RegisterFile;
use crate::compiler::compile;
use crate::compiler::compiler_data::Source;
use crate::data::Data;
use crate::instr::Instr;

/// Runs a compiled program the way the command line does: an error no `try`
/// catches prints its report and ends the run, which in a test build panics.
#[allow(clippy::too_many_arguments)] // the VM's own entry point, passed through
fn run_vm(
    instructions: &[Instr],
    r: &mut RegisterFile,
    pools: &mut crate::rt::Pools,
    err_ctx: &crate::errors::ErrorCtx<'_>,
    callsite_registers: &[Vec<u16>],
    dyn_libs: &[crate::rt::DynamicLibFn],
    structs: &[crate::rt::Struct],
    enums: &[crate::rt::EnumType],
    allocated_arg_count: usize,
    allocated_call_depth: usize,
    host_sigs: &[crate::rt::HostFnSig],
    host_dispatch: &[candela_vm::embed::HostDispatch],
    start: usize,
) {
    if let Err(error) = crate::vm::execute(
        instructions,
        r,
        pools,
        err_ctx,
        callsite_registers,
        dyn_libs,
        structs,
        enums,
        allocated_arg_count,
        allocated_call_depth,
        host_sigs,
        host_dispatch,
        start,
        None,
    ) {
        error.report(err_ctx);
    }
}

/// Compiles and runs `contents` with the compiler's debug dump on, then
/// asserts some `print` left `expected` in its register. It does so in both
/// profiles, since a release build has to reach the same answers.
macro_rules! run_and_check_registers {
    ($contents:expr, $expected:expr) => {
        for optimize in [false, true] {
        let filename = "test.kl";
        let out = crate::compiler::compile_profile(
            String::from($contents),
            filename,
            true,
            &crate::compiler::imports::ImportResolver::new(),
            optimize,
        );
        let instructions = out.instructions;
        let mut arrays = out.pools;
        let mut reg = RegisterFile(out.registers);
        run_vm(
            &instructions,
            &mut reg,
            &mut arrays,
            &crate::errors::ErrorCtx {
                instr_src: &out.instr_src,
                sources: &[Source {
                    filename: filename.into(),
                    contents: String::from($contents),
                }],
            },
            &out.callsite_registers,
            &[],
            &[],
            &[],
            out.allocated_arg_count,
            out.allocated_call_depth,
            &[],
            &[],
            0,
        );
        assert!(instructions.iter().any(|x| {
            if let Instr::Print(tgt) = x {
                reg[(*tgt) as usize] == $expected
            } else {
                false
            }
        }), "the program leaves the value in the register it prints (release profile: {optimize})");
        }
        native_agrees($contents, "test.kl", &crate::compiler::imports::ImportResolver::new());
    };
}

macro_rules! run {
    ($contents:expr) => {
        for optimize in [false, true] {
            let filename = "test.kl";
            let out = crate::compiler::compile_profile(
                String::from($contents),
                filename,
                true,
                &crate::compiler::imports::ImportResolver::new(),
                optimize,
            );
            let mut arrays = out.pools;
            run_vm(
                &out.instructions,
                &mut RegisterFile(out.registers),
                &mut arrays,
                &crate::errors::ErrorCtx {
                    instr_src: &out.instr_src,
                    sources: &[Source {
                        filename: filename.into(),
                        contents: String::from($contents),
                    }],
                },
                &out.callsite_registers,
                &[],
                &[],
                &[],
                out.allocated_arg_count,
                out.allocated_call_depth,
                &[],
                &[],
                0,
            );
        }
        native_agrees(
            $contents,
            "test.kl",
            &crate::compiler::imports::ImportResolver::new(),
        );
    };
}

/// What a run printed, and how it ended.
#[cfg(feature = "native")]
type Outcome = (String, Result<(), crate::Diagnostic>);

/// Runs `run` with output captured on this thread.
#[cfg(feature = "native")]
fn captured(run: impl FnOnce() -> Result<(), crate::Diagnostic>) -> Outcome {
    use candela_vm::captured_output::CAPTURED_OUTPUT;
    use candela_vm::captured_output::set_capturing;
    CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    let was_capturing = set_capturing(true);
    let result = run();
    set_capturing(was_capturing);
    (CAPTURED_OUTPUT.with(|o| o.take()), result)
}

/// Builds `src` the way `candela build` does, machine code included, and runs
/// the artifact the way `candela-vm` does. `None` for a program that does not
/// build into an artifact, such as one with no `main`.
#[cfg(feature = "native")]
fn run_native(
    src: &str,
    filename: &str,
    resolver: &crate::compiler::imports::ImportResolver,
) -> Option<Outcome> {
    let bytes = crate::errors::collect_diagnostic(|| {
        crate::build::build_artifact(
            String::from(src),
            filename,
            resolver,
            &crate::build::BuildOptions::default(),
        )
    })
    .ok()?
    .ok()?;
    let mut program = candela_vm::load_program(&bytes, &candela_vm::HostRegistry::new()).ok()?;
    Some(captured(|| {
        crate::errors::collect_diagnostic(|| program.run())
    }))
}

/// Checks that `src`, built with machine code, prints what the release
/// profile prints on the interpreter and stops with the same error at the
/// same span. A program the build does not make an artifact of is not
/// checked.
#[cfg(feature = "native")]
fn native_agrees(src: &str, filename: &str, resolver: &crate::compiler::imports::ImportResolver) {
    let Some(native) = run_native(src, filename, resolver) else {
        return;
    };
    let interpreted = captured(|| run_diag_profile_with(src, filename, true, resolver));
    assert_eq!(
        interpreted, native,
        "machine code prints and stops the way the interpreter does"
    );
}

#[cfg(not(feature = "native"))]
const fn native_agrees(_: &str, _: &str, _: &crate::compiler::imports::ImportResolver) {}

#[test]
pub fn rec_fib_1() {
    run_and_check_registers!(
        "
        fn fib(n) {
            if n <= 1 {return n;}
            else {return fib(n-1)+fib(n-2);}
        }

        fn main() {
            let x = fib(1);
            print(x);
        }
        ",
        1.into()
    );
}

#[test]
pub fn rec_fib_25() {
    run_and_check_registers!(
        "
        fn fib(n) {
            if n <= 1 {return n;}
            else {return fib(n-1)+fib(n-2);}
        }

        fn main() {
            let x = fib(25);
            print(x);
        }
        ",
        75025.into()
    );
}

/// A recursive call saves the registers that stay live across it, and every way
/// out of that call puts them back. A function that ends without a value leaves
/// through `VoidReturn`, which used to jump straight to the caller and leave it
/// reading whatever the deepest call left behind, so `mine` printed the
/// innermost `10` at all three levels instead of counting back up to `30`.
#[test]
pub fn void_recursion_restores_the_callers_registers() {
    run_and_check_registers!(
        "
        fn walk(n) {
            if n <= 0 {
                return;
            }
            let mine = n * 10;
            walk(n - 1);
            print(mine);
        }

        fn main() {
            walk(3);
        }
        ",
        30.into()
    );
}

/// An argument that is already in the parameter register it goes to is not
/// moved there, so the second call below reads `depth` without any instruction
/// naming it. The first call still has to save `depth` for it, or the second
/// call starts from whatever the first one's recursion left in the register.
#[test]
pub fn a_parameter_passed_on_unmoved_survives_the_call_before_it() {
    assert_eq!(
        run_output(
            "
            fn count(depth) {
                if depth == 0 { return 1; }
                depth -= 1;
                return count(depth) + count(depth);
            }

            fn main() {
                print(count(5));
            }
            "
        ),
        "32\n"
    );
}

/// A recursive call made to work out an argument of another saves a frame of
/// its own between the outer call's frame and the outer call. Each call has to
/// keep its own list of registers to save; the outer call used to take the
/// inner one's, so `n` came back from the inner call as the value the deepest
/// level left in it and `% n` divided by zero.
#[test]
pub fn a_recursive_call_inside_the_arguments_of_another_keeps_its_own_saves() {
    assert_eq!(
        run_output(
            "
            fn f(n) {
                if n <= 0 { return 1; }
                let keep = n * 100;
                let r = f(f(n - 1) % n);
                return keep + r;
            }

            fn main() {
                print(f(6));
            }
            "
        ),
        "701\n"
    );
}

/// A register the code after a call overwrites before reading holds nothing
/// the call has to save, but only on the path through that code. A throw out
/// of the call lands on the catch instead, which reads what the register held
/// before the call.
#[test]
pub fn a_catch_reads_what_the_code_after_the_call_would_have_overwritten() {
    assert_eq!(
        run_output(
            "
            fn dig(n, catching) {
                if n <= 0 { throw(\"bottom\"); }
                let mine = n * 10;
                if catching {
                    try {
                        dig(n - 1, false);
                        mine = 0;
                        print(mine);
                    } catch e {
                        print(mine);
                    }
                } else {
                    dig(n - 1, false);
                }
            }

            fn main() {
                dig(2, true);
            }
            "
        ),
        "20\n"
    );
}

/// A loop over a list or a string tests for its end once per turn, at the
/// bottom, so `continue` has to land on the step and `break` past the last
/// test, and an empty collection must never run the body.
#[test]
pub fn a_loop_over_a_collection_steps_breaks_and_continues() {
    assert_eq!(
        run_output(
            "
            fn main() {
                let s = 0;
                for x in [1, 2, 3, 4, 5, 6] {
                    if x == 2 { continue; }
                    if x == 5 { break; }
                    s += x;
                }
                print(s);
                let out = \"\";
                for c in \"hello\" {
                    if c == \"l\" { continue; }
                    out = out + c;
                }
                print(out);
                for _ in [] { print(\"never\"); }
                let fs = [];
                for x in [10, 20] {
                    fs.push(fn() { return x; });
                }
                for f in fs { print(f()); }
            }
            "
        ),
        "8\nheo\n10\n20\n"
    );
}

/// Adding or subtracting a constant `int` carries the constant in the
/// instruction. The constant may stand on either side of `+`, subtracting it
/// is adding its negation, and one too wide for the operand stays in its
/// register.
#[test]
pub fn a_constant_operand_adds_like_a_register() {
    assert_eq!(
        run_output(
            "
            fn main() {
                let n = 10;
                print(n + 7, 7 + n, n - 7, n - -7, n + 40000, 3 - n);
            }
            "
        ),
        "17\n17\n3\n17\n40010\n-7\n"
    );
}

/// A constant is carried into an instruction only from a register nothing
/// writes. Here `got` starts from a literal and is written inside a `try` and
/// a `catch`, and the sum has to read what was written.
#[test]
pub fn a_written_literal_register_is_not_a_constant() {
    assert_eq!(
        run_output(
            "
            fn sum(n) {
                if n <= 0 { throw(\"bottom\"); }
                let got = 0;
                try {
                    got = sum(n - 1);
                } catch e {
                    got = 1;
                }
                return n + got;
            }

            fn main() {
                print(sum(3));
            }
            "
        ),
        "7\n"
    );
}

/// A variable that starts from a literal shares the literal's register until
/// something assigns to it. An assignment inside a `try` or a `catch` counts,
/// or the write lands in the register every other use of that literal reads,
/// and the second call below would start from 6 instead of 0.
#[test]
pub fn an_assignment_inside_a_try_gets_its_own_register() {
    assert_eq!(
        run_output(
            "
            fn g(n) {
                let got = 0;
                try {
                    got = n + 5;
                } catch e {
                    got = 1;
                }
                return got;
            }

            fn main() {
                print(g(1));
                print(g(0) + 0);
            }
            "
        ),
        "6\n5\n"
    );
}

/// A release build copies a small function into its call site. An error the
/// copy raises has to read exactly as the call's would: the same message and
/// code, at the same span, including when the instruction that raises it was
/// rewritten to write the call's register directly.
#[test]
pub fn an_inlined_body_raises_what_the_call_would() {
    let src = "
fn share(total, parts) {
    return total / parts;
}

fn main() {
    print(share(10, 2));
    print(share(10, 0));
}
";
    let debug = run_diag_profile(src, "inline.cdl", false).unwrap_err();
    let release = run_diag_profile(src, "inline.cdl", true).unwrap_err();
    assert_eq!(debug, release);
    assert_eq!(debug.code, "division_by_zero");
    assert_eq!(&src[debug.span], "total / parts");
}

/// A body is copied into its call sites only in a release build; the debug
/// build of the same program still calls it.
#[test]
pub fn only_a_release_build_copies_a_body_into_its_call() {
    let src = "
fn twice(x) { return x * 2; }

fn main() {
    print(twice(21));
}
";
    let calls = |optimize: bool| {
        crate::compiler::compile_profile(
            String::from(src),
            "copy.cdl",
            false,
            &crate::compiler::imports::ImportResolver::new(),
            optimize,
        )
        .instructions
        .iter()
        .filter(|instr| matches!(instr, Instr::CallFunc(_, _)))
        .count()
    };
    assert_eq!(calls(false), 1);
    assert_eq!(calls(true), 0);
    assert_eq!(run_output(src), "42\n");
}

/// A call's arguments go into registers that belong to the function being
/// called. A later argument that calls the same function writes those same
/// registers, so an argument worked out before it has to wait in a register of
/// its own; `f(x, f(2, 3))` used to run the outer call with 2 where `x` was.
#[test]
pub fn a_later_argument_calling_the_callee_leaves_earlier_ones_alone() {
    assert_eq!(
        run_output(
            "
            fn f(a, b) { return a * 10 + b; }

            fn main() {
                let x = 1;
                print(f(x, f(2, 3)));
                print(f(f(4, 5), x));
                print(f(x + 1, f(x, x) + 0));
            }
            "
        ),
        "33\n451\n31\n"
    );
}

/// A release build keeps a struct that never leaves the registers as one
/// register per field. Whatever it keeps pooled or not, the program has to
/// print what a debug build prints: a struct shared by two names and written
/// through one, one stored in a list, compared, printed, or written after it
/// was built all have to behave as pooled structs do.
#[test]
pub fn a_struct_kept_in_registers_behaves_as_a_pooled_one() {
    let printed = run_output(
        "
        struct V { x: float, y: float }

        impl V {
            fn +(self, other: V) -> V { return V { x: self.x + other.x, y: self.y + other.y }; }
            fn scale(self, k: float) -> V { return V { x: self.x * k, y: self.y * k }; }
            fn len2(self) -> float { return self.x * self.x + self.y * self.y; }
        }

        fn main() {
            let p = V { x: 0.0, y: 0.0 };
            let v = V { x: 0.5, y: 0.25 };
            let s = 0.0;
            for i in 0..100 {
                p = p + v.scale(0.5);
                s += p.len2();
            }
            print(s, p.x, p.y);

            let a = V { x: 1.0, y: 2.0 };
            let b = a;
            b.x = 5.0;
            print(a.x);

            let kept = [];
            for i in 0..3 {
                let q = V { x: 1.0, y: 1.0 } + V { x: float(i), y: 0.0 };
                kept.push(q);
            }
            print(kept[2].x, kept.len());

            let c = V { x: 1.0, y: 1.0 } + V { x: 0.0, y: 0.0 };
            print(c == V { x: 1.0, y: 1.0 });
            print(c);

            let m = V { x: 3.0, y: 4.0 } + V { x: 0.0, y: 0.0 };
            for i in 0..3 {
                m.x = m.x + 1.0;
            }
            print(m.len2());
        }
        ",
    );
    assert_eq!(
        printed,
        "26433.59375\n25.0\n12.5\n5.0\n3.0\n3\ntrue\nV {x:1.0,y:1.0}\n52.0\n"
    );
}

/// A throw out of a call made inside a `try` skips the return that would have
/// put the caller's registers back, so the catch does it instead. The save the
/// aborted call left behind used to stay on the recursion stack, and every
/// level above the catch then read `mine` one call too deep.
#[test]
pub fn a_catch_restores_the_registers_of_the_call_it_ended() {
    run_and_check_registers!(
        "
        fn dig(n) {
            if n <= 0 {
                throw(\"bottom\");
            }
            let mine = n * 10;
            try {
                dig(n - 1);
            } catch e {
                print(mine);
            }
            print(mine);
        }

        fn main() {
            dig(3);
        }
        ",
        30.into()
    );
}

#[test]
pub fn fn_call_in_if_in_for() {
    run_and_check_registers!(
        "
        fn is_digit(c) {
            return c == \"0\" || c == \"1\" || c == \"2\" || c == \"3\" || c == \"4\" || c == \"5\" || c == \"6\" || c == \"7\" || c == \"8\" || c == \"9\";
        }
        fn main() {
            let count = 0;
            for x in \"3 + 4\" {
                if x != \" \" {
                    if is_digit(x) {
                        count += int(x);
                    }
                }
            }
            print(count);
        }
        ",
        7.into()
    );
}

#[test]
pub fn while_and_condition() {
    run_and_check_registers!(
        "
        fn main() {
        let count = 0;
        let limit = 1000000;
        let result = 1;
        while count < limit {
            result *= 2;
            if result > 1000000 {
                result %= 1000000;
            }
            count += 1;
        }
        print(result);
        }
        ",
        109_376.into()
    );
}

#[test]
pub fn iter_fib_40() {
    run_and_check_registers!(
        "
        fn main() {
        let n = 40;
        let a=0;
        let b=1;
        let c=0;
        let i=0;
        while i < (n-1) {
           c = a+b;
           a = b;
           b = c;
           i = i+1;
        }
        print(c);
        }
        ",
        102_334_155.into()
    );
}
#[test]
pub fn iter_fib_40_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let sum = 0;
            for _ in 0..200000 {
                let a = 0;
                let b = 1;
                let c = 0;
                for i in 0..39 {
                    c = a + b;
                    a = b;
                    b = c;
                }
                sum += (b % 10);
            }
            print(sum);
        }
        ",
        1_000_000.into()
    );
}

#[test]
pub fn string_split_array_len() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello world";
            let parts = s.split(" ");
            print(parts.len());
        }
        "#,
        2.into()
    );
}

#[test]
pub fn string_split_on_empty_separator_yields_characters() {
    run_and_check_registers!(
        r#"
        fn main() {
            let parts = "abc".split("");
            print(parts.len());
        }
        "#,
        3.into()
    );
}

#[test]
pub fn string_split_on_empty_separator_keeps_no_empty_ends() {
    run_and_check_registers!(
        r#"
        fn main() {
            print("abc".split("").join("-").len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn string_split_on_empty_separator_cuts_by_character() {
    // The accented e is two bytes, so a byte-wise split would answer five
    // parts and would cut the character in half.
    run_and_check_registers!(
        "
        fn main() {
            print(\"caf\u{e9}\".split(\"\").len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn string_split_on_empty_separator_keeps_a_character_whole() {
    // The last part is the whole accented e, which is one character however
    // many bytes it takes.
    run_and_check_registers!(
        "
        fn main() {
            let parts = \"caf\u{e9}\".split(\"\");
            print(parts[3].len());
        }
        ",
        1.into()
    );
}

#[test]
pub fn string_split_of_nothing_on_empty_separator_is_empty() {
    run_and_check_registers!(
        r#"
        fn main() {
            print("".split("").len());
        }
        "#,
        0.into()
    );
}

#[test]
pub fn string_contains() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello world";
            print(s.contains("world"));
        }
        "#,
        true.into()
    );
}

#[test]
pub fn for_loop_sum() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            let sum = 0;
            for x in arr {
                sum += x;
            }
            print(sum);
        }
        ",
        15.into()
    );
}

#[test]
pub fn array_sort() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [3, 1, 4, 1, 5, 9, 2, 6];
            arr.sort();
            print(arr[0]);
        }
        ",
        1.into()
    );
}

#[test]
pub fn array_push_len() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [1, 2, 3];
            arr.push(4);
            print(arr.len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn array_partition() {
    run_and_check_registers!(
        "
        fn main() {
            let x = [1,2,3,0,4,5,6];
            let p = x.partition(0);
            print(p[0][0]+p[1][2]);
        }
        ",
        7.into()
    );
}

#[test]
pub fn int_for_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let sum = 0;
            for i in 0..10 {
                sum += i;
            }
            print(sum);
        }
        ",
        45.into()
    );
}

#[test]
pub fn string_trim() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "  hello  ";
            let t = s.trim();
            print(t.len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn recursive_factorial() {
    run_and_check_registers!(
        "
        fn fact(n) {
            if n <= 1 { return 1; }
            else { return n * fact(n - 1); }
        }
        fn main() {
            print(fact(10));
        }
        ",
        3_628_800.into()
    );
}

#[test]
pub fn inline_condition_true() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 10;
            let result = if x > 5 { 1 } else { 0 };
            print(result);
        }
        ",
        1.into()
    );
}

#[test]
pub fn inline_condition_false() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 3;
            let result = if x > 5 { 1 } else { 0 };
            print(result);
        }
        ",
        0.into()
    );
}

#[test]
pub fn inline_condition_else_if() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            let result = if x > 10 { 2 } else if x > 3 { 1 } else { 0 };
            print(result);
        }
        ",
        1.into()
    );
}

#[test]
pub fn inline_condition_as_arg() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 42;
            print(if x == 42 { 99 } else { 0 });
        }
        ",
        99.into()
    );
}

#[test]
pub fn float_addition() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 1.5 + 2.5;
            print(x);
        }
        ",
        4.0f64.into()
    );
}

/// A float literal may carry an exponent, with or without a decimal point and
/// with either case of `e`.
#[test]
pub fn float_literal_with_exponent() {
    run_and_check_registers!(
        "
        fn main() {
            print(1e3);
        }
        ",
        1000.0f64.into()
    );
}

#[test]
pub fn float_literal_with_negative_exponent() {
    run_and_check_registers!(
        "
        fn main() {
            print(2.5e-1);
        }
        ",
        0.25f64.into()
    );
}

#[test]
pub fn float_literal_with_capital_exponent() {
    run_and_check_registers!(
        "
        fn main() {
            print(1E5);
        }
        ",
        100_000.0f64.into()
    );
}

#[test]
pub fn float_sqrt() {
    run_and_check_registers!(
        "
        fn main() {
            let x = float(144).sqrt();
            print(x);
        }
        ",
        12.0f64.into()
    );
}

#[test]
pub fn float_floor() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 3.9;
            print(x.floor());
        }
        ",
        3.0f64.into()
    );
}

#[test]
pub fn float_abs() {
    run_and_check_registers!(
        "
        fn main() {
            let x = -7.5;
            print(x.abs());
        }
        ",
        7.5f64.into()
    );
}

#[test]
pub fn int_to_float_conversion() {
    run_and_check_registers!(
        "
        fn main() {
            let x = float(42);
            print(x);
        }
        ",
        42.0f64.into()
    );
}

#[test]
pub fn float_to_int_conversion() {
    run_and_check_registers!(
        "
        fn main() {
            let x = int(3.9);
            print(x);
        }
        ",
        3.into()
    );
}

#[test]
pub fn int_to_str_conversion() {
    run_and_check_registers!(
        r"
        fn main() {
            let x = str(42);
            print(x.len());
        }
        ",
        2.into()
    );
}

#[test]
pub fn string_starts_ends_with() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello world";
            let a = s.starts_with("hello");
            let b = s.ends_with("world");
            print(a && b);
        }
        "#,
        true.into()
    );
}

#[test]
pub fn string_replace() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello world";
            let r = s.replace("world", "keel");
            print(r.len());
        }
        "#,
        10.into()
    );
}

#[test]
pub fn string_find() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello world";
            print(s.find("world"));
        }
        "#,
        6.into()
    );
}

#[test]
pub fn string_repeat() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "ab";
            print(s.repeat(3).len());
        }
        "#,
        6.into()
    );
}

/// `<`, `<=`, `>` and `>=` order two strings by their bytes, so a shorter
/// string sorts before the longer one it prefixes, uppercase sorts before
/// lowercase, and a multi-byte character sorts after every ASCII one.
#[test]
pub fn string_ordering_operators() {
    let printed = run_output(
        r#"
        fn main() {
            print("apple" < "banana");
            print("banana" < "apple");
            print("apple" <= "banana");
            print("banana" >= "apple");
            print("banana" > "apple");
            print("apple" > "banana");
            print("apple" <= "apple");
            print("apple" >= "apple");
            print("apple" < "apple");
            print("" < "a");
            print("" >= "");
            print("ab" < "abc");
            print("Z" < "a");
        }
        "#,
    );
    assert_eq!(
        printed,
        "true\nfalse\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue\ntrue\n"
    );
}

/// The order is over bytes, not over where a reader would file a letter in an
/// alphabet: every ASCII character sorts before a character that needs more
/// than one byte, whatever its accent.
#[test]
pub fn string_ordering_is_over_bytes() {
    let src = format!(
        r#"
        fn main() {{
            print("a" < "{e}");
            print("z" < "{e}");
            print("{e}" < "b");
        }}
        "#,
        e = '\u{e9}'
    );
    let printed = run_output(&src);
    assert_eq!(printed, "true\ntrue\nfalse\n");
}

/// A string comparison reads as a condition, in an `if` and in a `while`
/// alike. Neither fuses into a comparing jump, so both go through the bool the
/// comparison leaves in its register.
#[test]
pub fn string_ordering_as_a_condition() {
    let printed = run_output(
        r#"
        fn main() {
            if "fig" < "pear" {
                print("before");
            } else {
                print("after");
            }
            let names = ["apple", "fig", "pear"];
            let i = 0;
            while names[i] < "pear" {
                i = i + 1;
            }
            print(i);
        }
        "#,
    );
    assert_eq!(printed, "before\n2\n");
}

/// The ordering is the same one reached through a generic function and through
/// a closure, where the operand types arrive from the call site.
#[test]
pub fn string_ordering_through_a_generic_and_a_closure() {
    let printed = run_output(
        r#"
        fn smaller<T>(a: T, b: T) -> T {
            if a < b {
                return a;
            }
            return b;
        }

        fn main() {
            print(smaller("pear", "fig"));
            print(smaller(4, 9));
            let larger = fn(a, b) {
                if a > b {
                    return a;
                }
                return b;
            };
            print(larger("ant", "bee"));
        }
        "#,
    );
    assert_eq!(printed, "fig\n4\nbee\n");
}

/// `sort` orders a list of strings the same way the operators do, so every
/// neighbouring pair of a sorted list compares as ordered.
#[test]
pub fn string_ordering_matches_sort() {
    let printed = run_output(
        r#"
        fn main() {
            let names = ["pear", "Apple", "fig", "apple", "Fig"];
            names.sort();
            print(names);
            let ordered = true;
            for i in 1..names.len() {
                if names[i - 1] > names[i] {
                    ordered = false;
                }
            }
            print(ordered);
        }
        "#,
    );
    assert_eq!(
        printed,
        "[\"Apple\",\"Fig\",\"apple\",\"fig\",\"pear\"]\ntrue\n"
    );
}
#[test]
pub fn array_repeat() {
    run_and_check_registers!(
        r"
        fn main() {
            let s = [1,2];
            let t = s.repeat(3);
            print(t.len()+t[2]);
        }
        ",
        7.into()
    );
}

#[test]
pub fn array_contains() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [1, 2, 3, 4, 5];
            print(arr.contains(3));
        }
        ",
        true.into()
    );
}

#[test]
pub fn array_reverse() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [1, 2, 3];
            arr.reverse();
            print(arr[0]);
        }
        ",
        3.into()
    );
}

#[test]
pub fn array_remove() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [10, 20, 30];
            arr.remove(1);
            print(arr.len());
        }
        ",
        2.into()
    );
}

#[test]
pub fn array_join() {
    run_and_check_registers!(
        r#"
        fn main() {
            let arr = ["a", "b", "c"];
            let s = arr.join(",");
            print(s.len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn array_modify_index() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [1, 2, 3];
            arr[1] = 99;
            print(arr[1]);
        }
        ",
        99.into()
    );
}

#[test]
pub fn break_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 0;
            for i in 0..100 {
                if i == 5 { break; }
                x += 1;
            }
            print(x);
        }
        ",
        5.into()
    );
}

#[test]
pub fn continue_in_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let sum = 0;
            for i in 0..10 {
                if (i % 2) == 0 { continue; }
                sum += i;
            }
            print(sum);
        }
        ",
        25.into()
    );
}

#[test]
pub fn nested_loops() {
    run_and_check_registers!(
        "
        fn main() {
            let count = 0;
            for i in 0..4 {
                for j in 0..4 {
                    count += 1;
                }
            }
            print(count);
        }
        ",
        16.into()
    );
}

#[test]
pub fn bool_and_operator() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            print(x > 3 && x < 10);
        }
        ",
        true.into()
    );
}

#[test]
pub fn bool_or_operator() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 15;
            print(x < 3 || x > 10);
        }
        ",
        true.into()
    );
}

#[test]
pub fn negation() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            print(-x);
        }
        ",
        (-5).into()
    );
}

#[test]
pub fn power_operator() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 2 ^ 10;
            print(x);
        }
        ",
        1024.into()
    );
}

#[test]
pub fn multi_arg_function() {
    run_and_check_registers!(
        "
        fn add(a, b) { return a + b; }
        fn main() {
            print(add(3, 4));
        }
        ",
        7.into()
    );
}

#[test]
pub fn function_called_after_loop() {
    run_and_check_registers!(
        "
        fn double(n) { return n * 2; }
        fn main() {
            let sum = 0;
            for i in 0..10 { sum += i; }
            print(double(sum));
        }
        ",
        90.into()
    );
}

#[test]
pub fn recursive_fn_inside_for_loop() {
    run_and_check_registers!(
        "
        fn fib(n) {
            if n <= 1 { return n; }
            return fib(n-1) + fib(n-2);
        }
        fn main() {
            let x = [0, 1, 2];
            let sum = 0;
            for i in x {
                sum += fib(i);
            }
            print(sum);
        }
        ",
        2.into()
    );
}

#[test]
pub fn recursive_fib_after_loop() {
    run_and_check_registers!(
        "
        fn fib(n) {
            if n <= 1 { return n; }
            return fib(n - 1) + fib(n - 2);
        }
        fn main() {
            let x = 0;
            for i in 0..100 { x += i; }
            print(fib(10));
        }
        ",
        55.into()
    );
}

#[test]
pub fn sieve_of_eratosthenes() {
    run_and_check_registers!(
        "
        fn main() {
            let limit = 100000;
            let sieve = range(limit);
            sieve[0] = 0;
            sieve[1] = 0;
            let i = 2;
            while (i * i) <= limit {
                if sieve[i] != 0 {
                    let j = i * i;
                    while j < limit {
                        sieve[j] = 0;
                        j += i;
                    }
                }
                i += 1;
            }
            let count = 0;
            for x in sieve {
                if x != 0 { count += 1; }
            }
            print(count);
        }
        ",
        9592.into()
    );
}

#[test]
pub fn collatz_steps() {
    run_and_check_registers!(
        "
        fn main() {
            let n = 27;
            let steps = 0;
            while n != 1 {
                if (n % 2) == 0 {
                    n /= 2;
                } else {
                    n = n * 3 + 1;
                }
                steps += 1;
            }
            print(steps);
        }
        ",
        111.into()
    );
}

#[test]
pub fn string_word_count() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "the quick brown fox jumps";
            let words = s.split(" ");
            print(words.len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn range_sum() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = range(101);
            let sum = 0;
            for x in arr {
                sum += x;
            }
            print(sum);
        }
        ",
        5050.into()
    );
}

#[test]
pub fn bubble_sort() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [5, 3, 8, 1, 9, 2, 7, 4, 6];
            let n = arr.len();
            for i in 0..n {
                for j in 0..(n - 1) {
                    if arr[j] > arr[j + 1] {
                        let tmp = arr[j];
                        arr[j] = arr[j + 1];
                        arr[j + 1] = tmp;
                    }
                }
            }
            print(arr[0]+arr[8]);
        }
        ",
        10.into()
    );
}

#[test]
pub fn quicksort() {
    run_and_check_registers!(
        r"
        fn quicksort(arr) {
            if arr.len() <= 1 {
                return arr;
            }
            let pivot = arr[0];
            let left = [];
            let right = [];
            for i in 1..arr.len() {
                if arr[i] < pivot {
                    left.push(arr[i]);
                } else {
                    right.push(arr[i]);
                }
            }
            let sorted_left = quicksort(left);
            let sorted_right = quicksort(right);
            sorted_left.push(pivot);
            for x in sorted_right {
                sorted_left.push(x);
            }
            return sorted_left;
        }
        fn main() {
            let nums = [38, 27, 43, 3, 9, 82, 10];
            let sorted = quicksort(nums);
            print(sorted[0] + sorted[6]);
        }
        ",
        85.into()
    );
}

#[test]
pub fn for_loop_called_twice() {
    run_and_check_registers!(
        "
        fn sum(arr) {
            let s = 0;
            for x in arr {
                s += x;
            }
            return s;
        }
        fn main() {
            sum([1, 2, 3]);
            print(sum([1, 2, 3]));
        }
        ",
        6.into()
    );
}

#[test]
pub fn two_for_loops_in_sequence() {
    run_and_check_registers!(
        "
        fn main() {
            let a = [1, 2, 3];
            let b = [10, 20, 30];
            let sum = 0;
            for x in a { sum += x; }
            for x in b { sum += x; }
            print(sum);
        }
        ",
        66.into()
    );
}

#[test]
pub fn early_return_from_for_loop() {
    run_and_check_registers!(
        "
        fn first_positive(arr) {
            for x in arr {
                if x > 0 { return x; }
            }
            return 0;
        }
        fn main() {
            print(first_positive([-3, -1, 5, 8]));
        }
        ",
        5.into()
    );
}

#[test]
pub fn early_return_from_while_loop() {
    run_and_check_registers!(
        "
        fn find(limit) {
            let i = 0;
            while i < limit {
                if i == 7 { return i; }
                i += 1;
            }
            return -1;
        }
        fn main() {
            print(find(20));
        }
        ",
        7.into()
    );
}

#[test]
pub fn nested_fn_call_as_arg() {
    run_and_check_registers!(
        "
        fn double(n) { return n * 2; }
        fn inc(n)    { return n + 1; }
        fn main() {
            print(double(inc(double(3))));
        }
        ",
        14.into()
    );
}

#[test]
pub fn multi_loop_fn_called_twice() {
    run_and_check_registers!(
        "
        fn run(arr) {
            let s = 0;
            for x in arr { s += x; }
            for x in arr { s += x; }
            print(s);
        }
        fn main() {
            run([1, 2, 3]);
            run([1, 2, 3]);
        }
        ",
        12.into()
    );
}

#[test]
pub fn while_fn_called_twice() {
    run_and_check_registers!(
        "
        fn count_down(n) {
            let s = 0;
            while n > 0 {
                s += n;
                n -= 1;
            }
            return s;
        }
        fn main() {
            count_down(5);
            print(count_down(5));
        }
        ",
        15.into()
    );
}

#[test]
pub fn function_returns_array() {
    run_and_check_registers!(
        "
        fn make(n) {
            return [n, n * 2, n * 3];
        }
        fn main() {
            let arr = make(4);
            print(arr[0]+arr[1]+arr[2]);
        }
        ",
        24.into()
    );
}

#[test]
pub fn pass_array_to_function() {
    run_and_check_registers!(
        "
        fn last(arr) {
            let n = arr.len();
            return arr[n - 1];
        }
        fn main() {
            print(last([7, 8, 9]));
        }
        ",
        9.into()
    );
}

#[test]
pub fn string_split_then_iterate() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "a,b,c,d,e";
            let parts = s.split(",");
            let count = 0;
            for p in parts { count += 1; }
            print(count);
        }
        "#,
        5.into()
    );
}

#[test]
pub fn deeply_nested_conditions() {
    run_and_check_registers!(
        "
        fn classify(n) {
            if n < 0 {
                return 0;
            } else {
                if n < 10 {
                    return 1;
                } else {
                    if n < 100 {
                        return 2;
                    } else {
                        return 3;
                    }
                }
            }
        }
        fn main() {
            print(classify(50));
        }
        ",
        2.into()
    );
}

#[test]
pub fn break_in_while_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let i = 0;
            while i < 1000 {
                if i == 42 { break; }
                i += 1;
            }
            print(i);
        }
        ",
        42.into()
    );
}

#[test]
pub fn for_loop_discard_var() {
    run_and_check_registers!(
        "
        fn main() {
            let count = 0;
            for _ in [0, 0, 0, 0, 0] { count += 1; }
            print(count);
        }
        ",
        5.into()
    );
}

#[test]
pub fn int_range_loop_fn_called_twice() {
    run_and_check_registers!(
        "
        fn sum_to(n) {
            let s = 0;
            for i in 0..n { s += i; }
            return s;
        }
        fn main() {
            sum_to(10);
            print(sum_to(10));
        }
        ",
        45.into()
    );
}

#[test]
pub fn inc_int_to_basic() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            let y = x + 1;
            print(y);
        }
        ",
        6.into()
    );
}

#[test]
pub fn dec_int_to_basic() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            let y = x - 1;
            print(y);
        }
        ",
        4.into()
    );
}

#[test]
pub fn inc_int_commutative() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 10;
            let y = 1 + x;
            print(y);
        }
        ",
        11.into()
    );
}

#[test]
pub fn inc_int_to_chained() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 3;
            let y = x + 1;
            let z = y + 1;
            print(z);
        }
        ",
        5.into()
    );
}

#[test]
pub fn inc_int_as_function_arg() {
    run_and_check_registers!(
        "
        fn identity(n) { return n; }
        fn main() {
            let x = 7;
            print(identity(x + 1));
        }
        ",
        8.into()
    );
}

#[test]
pub fn dec_int_as_return_value() {
    run_and_check_registers!(
        "
        fn pred(n) { return n - 1; }
        fn main() {
            print(pred(20));
        }
        ",
        19.into()
    );
}

#[test]
pub fn inc_int_in_condition() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 9;
            let result = 0;
            if x + 1 > 9 { result = 1; }
            print(result);
        }
        ",
        1.into()
    );
}

#[test]
pub fn inc_int_does_not_mutate_source() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 41;
            let y = x + 1;
            print(x);
        }
        ",
        41.into()
    );
}

#[test]
pub fn dec_int_does_not_mutate_source() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 41;
            let y = x - 1;
            print(x);
        }
        ",
        41.into()
    );
}

#[test]
pub fn int_wraps_on_overflow() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 9223372036854775807;
            x += 1;
            print(x);
        }
        ",
        i64::MIN.into()
    );
}

#[test]
pub fn int_wraps_on_underflow() {
    run_and_check_registers!(
        "
        fn main() {
            let x = -9223372036854775808;
            x -= 1;
            print(x);
        }
        ",
        i64::MAX.into()
    );
}

#[test]
pub fn negative_int_literal() {
    run_and_check_registers!(
        "
        fn main() {
            let x = -9223372036854775808;
            print(x);
        }
        ",
        i64::MIN.into()
    );
}

#[test]
pub fn string_exactly_6_chars() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "abcdef";
            print(s.len());
        }
        "#,
        6.into()
    );
}

#[test]
pub fn string_exactly_7_chars() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "abcdefg";
            print(s.len());
        }
        "#,
        7.into()
    );
}

#[test]
pub fn string_small_to_large_concat() {
    run_and_check_registers!(
        r#"
        fn main() {
            let a = "abc";
            let b = "defgh";
            let c = a + b;
            print(c.len());
        }
        "#,
        8.into()
    );
}

#[test]
pub fn string_escape_newline() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "a\nb";
            print(s.len());
        }
        "#,
        3.into()
    );
}

#[test]
pub fn string_escape_tab() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "a\tb";
            print(s.len());
        }
        "#,
        3.into()
    );
}

#[test]
pub fn string_escape_backslash() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "a\\b";
            print(s.len());
        }
        "#,
        3.into()
    );
}

#[test]
pub fn string_escape_quote() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "say \"hello\"";
            print(s.len());
        }
        "#,
        11.into()
    );
}

#[test]
pub fn empty_range_for_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let count = 99;
            for _ in 0..0 { count += 1; }
            print(count);
        }
        ",
        99.into()
    );
}

#[test]
pub fn while_never_executes() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            while x > 10 { x += 1; }
            print(x);
        }
        ",
        5.into()
    );
}

#[test]
pub fn break_only_breaks_inner_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let outer = 0;
            for i in 0..3 {
                for j in 0..100 {
                    if j == 2 { break; }
                }
                outer += 1;
            }
            print(outer);
        }
        ",
        3.into()
    );
}

#[test]
pub fn empty_array_len() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [];
            print(arr.len());
        }
        ",
        0.into()
    );
}

#[test]
pub fn empty_array_iteration() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [];
            let count = 0;
            for _ in arr { count += 1; }
            print(count);
        }
        ",
        0.into()
    );
}

#[test]
pub fn single_element_array_len() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [42];
            print(arr.len());
        }
        ",
        1.into()
    );
}

#[test]
pub fn array_after_all_removes() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [1, 2, 3];
            arr.remove(0);
            arr.remove(0);
            arr.remove(0);
            print(arr.len());
        }
        ",
        0.into()
    );
}

#[test]
pub fn mutual_recursion() {
    run_and_check_registers!(
        "
        fn is_even(n) {
            if n == 0 { return true; }
            return is_odd(n - 1);
        }
        fn is_odd(n) {
            if n == 0 { return false; }
            return is_even(n - 1);
        }
        fn main() {
            print(is_even(10));
        }
        ",
        true.into()
    );
}

#[test]
pub fn null_literal_store_and_compare() {
    run_and_check_registers!(
        "
        fn main() {
            let x = null;
            print(x == null);
        }
        ",
        true.into()
    );
}

#[test]
pub fn null_literal_as_default() {
    run_and_check_registers!(
        "
        fn main() {
            let result = null;
            result = 42;
            print(result);
        }
        ",
        42.into()
    );
}

#[test]
pub fn array_push_type_inference_propagation() {
    run_and_check_registers!(
        "
        fn build_sieve(limit) {
            let sieve = range(limit);
            sieve[0] = 0;
            sieve[1] = 0;
            let i = 2;
            while (i * i) <= limit {
                if sieve[i] != 0 {
                    let j = i * i;
                    while j < limit {
                        sieve[j] = 0;
                        j += i;
                    }
                }
                i += 1;
            }
            return sieve;
        }

        fn collect_primes(sieve) {
            let primes = [];
            for x in sieve {
                if x != 0 {
                    primes.push(x);
                }
            }
            return primes;
        }

        fn largest_gap(primes) {
            let max = 0;
            let i = 1;
            while i < primes.len() {
                let gap = primes[i] - primes[i - 1];
                if gap > max {
                    max = gap;
                }
                i += 1;
            }
            return max;
        }

        fn main() {
            let primes = collect_primes(build_sieve(50));
            print(largest_gap(primes));
        }
        ",
        6.into()
    );
}

#[test]
pub fn split_result_survives_string_gc() {
    let text = "a abcdefghijk ".repeat(140);
    run_and_check_registers!(
        &format!(
            r#"
            fn longest_word(words) {{
                let longest = "";
                for word in words {{
                    if word.len() > longest.len() {{
                        longest = word;
                    }}
                }}
                return longest;
            }}

            fn main() {{
                let text = "{text}";
                let words = text.split(" ");
                print(longest_word(words).len());
            }}
        "#
        ),
        11.into()
    );
}

#[test]
pub fn expr_eval_mutual_recursion() {
    run_and_check_registers!(
        r#"
        fn is_digit(c) {
            return c == "0" || c == "1" || c == "2" || c == "3" || c == "4" || c == "5" || c == "6" || c == "7" || c == "8" || c == "9";
        }
        fn digit_value(c) {
            if c == "0" { return 0; } if c == "1" { return 1; } if c == "2" { return 2; }
            if c == "3" { return 3; } if c == "4" { return 4; } if c == "5" { return 5; }
            if c == "6" { return 6; } if c == "7" { return 7; } if c == "8" { return 8; }
            return 9;
        }
        fn skip_spaces(expr, pos) {
            while pos < expr.len() && expr[pos] == " " { pos += 1; }
            return pos;
        }
        fn parse_number(expr, pos) {
            let value = 0;
            while pos < expr.len() && is_digit(expr[pos]) {
                value = value * 10 + digit_value(expr[pos]);
                pos += 1;
            }
            return [value, pos];
        }
        fn parse_factor(expr, pos) {
            pos = skip_spaces(expr, pos);
            let c = expr[pos];
            if c == "(" {
                let parsed = parse_expr(expr, pos + 1);
                let value = parsed[0];
                pos = skip_spaces(expr, parsed[1]);
                return [value, pos + 1];
            }
            if c == "-" {
                let parsed = parse_factor(expr, pos + 1);
                return [0 - parsed[0], parsed[1]];
            }
            return parse_number(expr, pos);
        }
        fn parse_term(expr, pos) {
            let parsed = parse_factor(expr, pos);
            let value = parsed[0];
            pos = parsed[1];
            while pos < expr.len() {
                pos = skip_spaces(expr, pos);
                if pos >= expr.len() { break; }
                let op = expr[pos];
                if op != "*" && op != "/" && op != "%" { break; }
                parsed = parse_factor(expr, pos + 1);
                if op == "*" { value = value * parsed[0]; }
                if op == "/" { value = value / parsed[0]; }
                if op == "%" { value = value % parsed[0]; }
                pos = parsed[1];
            }
            return [value, pos];
        }
        fn parse_expr(expr, pos) {
            let parsed = parse_term(expr, pos);
            let value = parsed[0];
            pos = parsed[1];
            while pos < expr.len() {
                pos = skip_spaces(expr, pos);
                if pos >= expr.len() { break; }
                let op = expr[pos];
                if op != "+" && op != "-" { break; }
                parsed = parse_term(expr, pos + 1);
                if op == "+" { value += parsed[0]; }
                if op == "-" { value -= parsed[0]; }
                pos = parsed[1];
            }
            return [value, pos];
        }
        fn eval_expr(expr) { return parse_expr(expr, 0)[0]; }
        fn main() {
            let expressions = [
                "17 + 5 * (31 - 12) + 144 / 3 - 8 % 5",
                "((42 + 18) * 7 - 91) / 3 + 12 * (6 + 5)",
                "1000 - (35 * 17) + (256 / 8) * (19 - 4)",
                "-18 + 7 * (8 + 9 * (12 - 5)) - 64 / 4",
                "9 * 9 * 9 - (123 + 45) / 6 + 77 % 10",
                "(314 - 159) * (26 + 53) / 5 - 97",
                "12345 % 97 + 88 * (14 - 6) - 432 / 9",
                "7 + 11 * (13 + 17 * (19 - 23 + 29))",
                "(81 / 9 + 64 / 8) * (45 - 32) + 99",
                "2048 / 4 / 4 + 33 * (21 - 8) - 17"
            ];
            let checksum = 0;
            for i in 0..8000 {
                for expr in expressions {
                    checksum += eval_expr(expr) + (i % 17);
                }
            }
            print(checksum);
        }
        "#,
        90_023_650.into()
    );
}

#[test]
pub fn fn_call_in_if_and_in_nested_for() {
    run_and_check_registers!(
        r#"
        fn is_digit(c) {
            return c == "0" || c == "1" || c == "2" || c == "3" || c == "4" ||
                   c == "5" || c == "6" || c == "7" || c == "8" || c == "9";
        }

        fn main() {
            let sum = 0;
            for i in 0..2 {
                for x in "3 + 4" {
                    if x != " " && is_digit(x) {
                        sum += int(x);
                    }
                }
            }
            print(sum);
        }
        "#,
        14.into()
    );
}

#[test]
pub fn branch_without_return() {
    run_and_check_registers!(
        "
        fn choose(x) {
            if x > 0 {
                let unused = 1;
            }
            return 7;
        }

        fn main() {
            print(choose(1));
        }
        ",
        7.into()
    );
}

#[test]
pub fn unusued_branch_wth_return() {
    run_and_check_registers!(
        "
        fn choose(x) {
            if x > 0 {
                return 1;
            }
            return 2;
        }

        fn main() {
            print(choose(0));
        }
        ",
        2.into()
    );
}

#[test]
pub fn unreachable_return_after_exhaustive_condition() {
    run_and_check_registers!(
        "
        fn choose(x) {
            if x > 0 {
                return 1;
            } else {
                return 2;
            }
            return \"bad\";
        }

        fn main() {
            print(choose(1));
        }
        ",
        1.into()
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn partial_return_flow_with_null() {
    run!(
        r#"
        fn test(n) {
            if n == "" {
                return n;
            }
        }

        fn main() {
            print(test(input("> ")).uppercase());
        }
        "#
    );
}

#[test]
pub fn unused_nested_partial_branch() {
    run_and_check_registers!(
        r#"
        fn label(n) {
            if n > 0 {
                if n == 1 {
                    return "one";
                }
            }
            return "other";
        }

        fn main() {
            print(label(2).uppercase());
        }
        "#,
        crate::data::Data::small_str("OTHER")
    );
}

#[test]
pub fn return_flow_exhaustive_condition_ignores_later_conflicting_return() {
    run_and_check_registers!(
        r#"
        fn choose(n) {
            if n == 0 {
                return 10;
            } else if n == 1 {
                return 20;
            } else {
                return 30;
            }
            return "bad";
        }

        fn main() {
            print(choose(2) + 1);
        }
        "#,
        31.into()
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn return_flow_return_inside_for_loop_is_not_total() {
    run!(
        r#"
        fn first_word(words) {
            for word in words {
                return word;
            }
        }

        fn main() {
            print(first_word(["hello"]).uppercase());
        }
        "#
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn return_flow_return_inside_while_loop_is_not_total() {
    run!(
        r#"
        fn maybe_word(n) {
            while n > 0 {
                return "word";
            }
        }

        fn main() {
            print(maybe_word(0).uppercase());
        }
        "#
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn return_flow_branch_returns_null() {
    run!(
        "
        fn maybe_number(n) {
            if n > 0 {
                return;
            }
            return 1;
        }

        fn main() {
            print(maybe_number(0) + 1);
        }
        "
    );
}

#[test]
pub fn return_flow_branch_local_return_value_type_is_preserved() {
    run_and_check_registers!(
        r#"
        fn word(n) {
            if n > 0 {
                let value = "branch";
                return value;
            }
            return "fallback";
        }

        fn main() {
            print(word(1).uppercase());
        }
        "#,
        crate::data::Data::small_str("BRANCH")
    );
}

#[test]
pub fn match_basic_int() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 2;
            let result = 0;
            match x {
                1 => { result = 10; }
                2 => { result = 20; }
                3 => { result = 30; }
            }
            print(result);
        }
        ",
        20.into()
    );
}

#[test]
pub fn match_with_wildcard() {
    run_and_check_registers!(
        r#"
        fn main() {
            let x = "other";
            let result = 0;
            match x {
                "hello" => { result = 1; }
                "goodbye" => { result = 2; }
                _ => { result = 99; }
            }
            print(result);
        }
        "#,
        99.into()
    );
}

#[test]
pub fn match_first_arm() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 1;
            let result = 0;
            match x {
                1 => { result = 100; }
                2 => { result = 200; }
            }
            print(result);
        }
        ",
        100.into()
    );
}

#[test]
pub fn match_no_match() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 99;
            let result = 0;
            match x {
                1 => { result = 10; }
                2 => { result = 20; }
            }
            print(result);
        }
        ",
        0.into()
    );
}

#[test]
pub fn match_string_arms() {
    run_and_check_registers!(
        r#"
        fn main() {
            let cmd = "run";
            let code = 0;
            match cmd {
                "stop" => { code = 1; }
                "run" => { code = 2; }
                "pause" => { code = 3; }
                _ => { code = -1; }
            }
            print(code);
        }
        "#,
        2.into()
    );
}

#[test]
pub fn match_arm_computation() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 3;
            let result = 0;
            match x {
                1 => {
                    result = 10 + 5;
                }
                3 => {
                    let a = 7;
                    let b = 8;
                    result = a * b;
                }
            }
            print(result);
        }
        ",
        56.into()
    );
}

#[test]
pub fn loop_break() {
    run_and_check_registers!(
        "
        fn main() {
            let i = 0;
            loop {
                i += 1;
                if i == 10 { break; }
            }
            print(i);
        }
        ",
        10.into()
    );
}

#[test]
pub fn loop_continue() {
    run_and_check_registers!(
        "
        fn main() {
            let i = 0;
            let sum = 0;
            loop {
                i += 1;
                if i > 20 { break; }
                if (i % 2) == 0 { continue; }
                sum += i;
            }
            print(sum);
        }
        ",
        100.into()
    );
}

#[test]
pub fn nested_loop_blocks() {
    run_and_check_registers!(
        "
        fn main() {
            let count = 0;
            let i = 0;
            loop {
                i += 1;
                if i > 3 { break; }
                let j = 0;
                loop {
                    j += 1;
                    if j > 4 { break; }
                    count += 1;
                }
            }
            print(count);
        }
        ",
        12.into()
    );
}

#[test]
pub fn nested_loop_inner_break() {
    run_and_check_registers!(
        "
        fn main() {
            let outer = 0;
            let i = 0;
            loop {
                i += 1;
                if i > 3 { break; }
                let j = 0;
                loop {
                    j += 1;
                    if j > 1 { break; }
                }
                outer += 1;
            }
            print(outer);
        }
        ",
        3.into()
    );
}

// A `fn` inside a block is rejected outright. It used to parse and register
// itself only once the declaration statement was reached, so it worked when it
// came before its first call and was invisible to every other function.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn nested_fn() {
    run!(
        "
        fn main() {
            fn add(a, b) {
                return a + b;
            }
            print(add(3, 4));
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn nested_fn_in_loop() {
    run!(
        "
        fn main() {
            fn square(n) {
                return n * n;
            }
            let sum = 0;
            for i in 1..5 {
                sum += square(i);
            }
            print(sum);
        }
        "
    );
}

#[test]
pub fn block_scope() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 1;
            {
                let y = 2;
                x = x + y;
            }
            print(x);
        }
        ",
        3.into()
    );
}

#[test]
pub fn range_two_arg() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = range(5, 10);
            let sum = 0;
            for x in arr { sum += x; }
            print(sum);
        }
        ",
        35.into()
    );
}

#[test]
pub fn range_two_arg_index() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = range(3, 7);
            print(arr[0]);
        }
        ",
        3.into()
    );
}

#[test]
pub fn string_uppercase() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello";
            print(s.uppercase().len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn string_lowercase() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "ABCDE";
            print(s.lowercase().len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn string_is_float() {
    run_and_check_registers!(
        r#"
        fn main() {
            print("3.14".is_float());
        }
        "#,
        true.into()
    );
}

#[test]
pub fn string_is_float_false() {
    run_and_check_registers!(
        r#"
        fn main() {
            print("42".is_float());
        }
        "#,
        false.into()
    );
}

#[test]
pub fn string_is_int_true() {
    run_and_check_registers!(
        r#"
        fn main() {
            print("42".is_int());
        }
        "#,
        true.into()
    );
}

#[test]
pub fn string_is_int_false() {
    run_and_check_registers!(
        r#"
        fn main() {
            print("hello".is_int());
        }
        "#,
        false.into()
    );
}

#[test]
pub fn string_trim_sequence() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "--hello--";
            print(s.trim_sequence("-").len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn string_trim_sequence_left() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "--hello";
            print(s.trim_sequence_left("-").len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn string_trim_sequence_right() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello--";
            print(s.trim_sequence_right("-").len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn float_round() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 3.7;
            print(x.round());
        }
        ",
        4.0f64.into()
    );
}

#[test]
pub fn int_abs() {
    run_and_check_registers!(
        "
        fn main() {
            let x = -42;
            print(x.abs());
        }
        ",
        42.into()
    );
}

#[test]
pub fn string_reverse_method() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "abcde";
            let r = s.reverse();
            print(r.len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn array_find() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [10, 20, 30, 40];
            print(arr.find(30));
        }
        ",
        2.into()
    );
}

#[test]
pub fn array_find_missing() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [10, 20, 30];
            print(arr.find(99));
        }
        ",
        (-1).into()
    );
}

#[test]
pub fn array_sort_floats() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [3.1, 1.4, 2.7];
            arr.sort();
            print(arr[0]);
        }
        ",
        1.4f64.into()
    );
}

#[test]
pub fn array_sort_strings() {
    run_and_check_registers!(
        r#"
        fn main() {
            let arr = ["banana", "apple", "cherry"];
            arr.sort();
            print(arr[0].len());
        }
        "#,
        5.into()
    );
}

#[test]
pub fn nested_array_index() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [[1, 2], [3, 4], [5, 6]];
            print(arr[1][1]);
        }
        ",
        4.into()
    );
}

#[test]
pub fn nested_array_set() {
    run_and_check_registers!(
        "
        fn main() {
            let arr = [[1, 2], [3, 4]];
            arr[0][1] = 99;
            print(arr[0][1]);
        }
        ",
        99.into()
    );
}

#[test]
pub fn bool_from_string_true() {
    run_and_check_registers!(
        r#"
        fn main() {
            print(bool("true"));
        }
        "#,
        true.into()
    );
}

#[test]
pub fn bool_from_string_false() {
    run_and_check_registers!(
        r#"
        fn main() {
            print(bool("false"));
        }
        "#,
        false.into()
    );
}

#[test]
pub fn the_answer() {
    run_and_check_registers!(
        "
        fn main() {
            print(the_answer());
        }
        ",
        42.into()
    );
}

#[test]
pub fn compound_mul_assign() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 5;
            x *= 3;
            print(x);
        }
        ",
        15.into()
    );
}

#[test]
pub fn compound_div_assign() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 20;
            x /= 4;
            print(x);
        }
        ",
        5.into()
    );
}

#[test]
pub fn compound_mod_assign() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 17;
            x %= 5;
            print(x);
        }
        ",
        2.into()
    );
}

#[test]
pub fn compound_pow_assign() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 2;
            x ^= 8;
            print(x);
        }
        ",
        256.into()
    );
}

// ---------------------------------------------------------------------------
// BITWISE OPERATORS
//
// `&`, `|`, `^^`, `~`, `<<` and `>>` take `int` operands. `^` stays the power
// operator, so xor is spelled `^^`.
// ---------------------------------------------------------------------------

#[test]
pub fn bitwise_and() {
    run_and_check_registers!("fn main() { print(12 & 10); }", 8.into());
    run_and_check_registers!(
        "fn main() { let a = -8; let b = 6; print(a & b); }",
        0.into()
    );
}

#[test]
pub fn bitwise_or() {
    run_and_check_registers!("fn main() { print(12 | 3); }", 15.into());
    run_and_check_registers!(
        "fn main() { let a = -8; let b = 6; print(a | b); }",
        (-2).into()
    );
}

#[test]
pub fn bitwise_xor() {
    run_and_check_registers!("fn main() { print(12 ^^ 10); }", 6.into());
    run_and_check_registers!(
        "fn main() { let a = -8; let b = 6; print(a ^^ b); }",
        (-2).into()
    );
}

/// Every bit of zero flipped is every bit set, which is -1 in two's complement.
#[test]
pub fn bitwise_not() {
    run_and_check_registers!("fn main() { print(~0); }", (-1).into());
    run_and_check_registers!("fn main() { let x = 5; print(~x); }", (-6).into());
    run_and_check_registers!("fn main() { let x = -1; print(~x); }", 0.into());
}

#[test]
pub fn shift_left() {
    run_and_check_registers!("fn main() { print(1 << 4); }", 16.into());
    run_and_check_registers!(
        "fn main() { let x = 3; let n = 2; print(x << n); }",
        12.into()
    );
    run_and_check_registers!("fn main() { print(1 << 63); }", i64::MIN.into());
}

/// `int` is signed, so `>>` is arithmetic: a negative value stays negative and
/// the vacated places take the sign bit.
#[test]
pub fn shift_right_is_arithmetic() {
    run_and_check_registers!("fn main() { print(-16 >> 2); }", (-4).into());
    run_and_check_registers!(
        "fn main() { let x = -1; let n = 40; print(x >> n); }",
        (-1).into()
    );
    run_and_check_registers!("fn main() { let x = 16; print(x >> 2); }", 4.into());
}

/// The bitwise levels sit between comparison and the shifts, as they do in
/// Rust, so `a | b == c` compares what the `|` produced.
#[test]
pub fn bitwise_precedence() {
    run_and_check_registers!("fn main() { print(1 | 2 & 3 == 3); }", true.into());
    run_and_check_registers!("fn main() { print(1 << 2 + 1); }", 8.into());
    run_and_check_registers!("fn main() { print(2 ^ 3 ^^ 1); }", 9.into());
    run_and_check_registers!("fn main() { print(1 | 2 ^^ 4 & 12); }", 7.into());
    run_and_check_registers!("fn main() { print(~1 + 1); }", (-1).into());
}

/// The same table applies to values the compiler cannot fold.
#[test]
pub fn bitwise_precedence_on_variables() {
    run_and_check_registers!(
        "
        fn main() {
            let a = 1;
            let b = 2;
            let c = 3;
            print(a | b & c == c);
        }
        ",
        true.into()
    );
    run_and_check_registers!(
        "
        fn main() {
            let one = 1;
            let two = 2;
            print(one << two + 1);
        }
        ",
        8.into()
    );
}

#[test]
pub fn compound_bit_and_assign() {
    run_and_check_registers!("fn main() { let x = 12; x &= 10; print(x); }", 8.into());
}

#[test]
pub fn compound_bit_or_assign() {
    run_and_check_registers!("fn main() { let x = 12; x |= 3; print(x); }", 15.into());
}

#[test]
pub fn compound_bit_xor_assign() {
    run_and_check_registers!("fn main() { let x = 12; x ^^= 10; print(x); }", 6.into());
}

#[test]
pub fn compound_shift_left_assign() {
    run_and_check_registers!("fn main() { let x = 3; x <<= 4; print(x); }", 48.into());
}

#[test]
pub fn compound_shift_right_assign() {
    run_and_check_registers!("fn main() { let x = -48; x >>= 4; print(x); }", (-3).into());
}

/// A compound assignment reaches an array element and a struct field, the way
/// the arithmetic ones do.
#[test]
pub fn compound_bitwise_assign_on_a_field_and_an_element() {
    run_and_check_registers!(
        "
        struct Flags { bits: int }
        fn main() {
            let f = Flags { bits: 1 };
            f.bits |= 4;
            let row = [3, 12];
            row[1] &= 10;
            print(f.bits + row[1]);
        }
        ",
        13.into()
    );
}

/// `^` is the power operator still, and `^=` still raises in place. The doubled
/// spelling is what tells xor from it.
#[test]
pub fn caret_stays_the_power_operator() {
    run_and_check_registers!("fn main() { print(2 ^ 8); }", 256.into());
    run_and_check_registers!("fn main() { let x = 3; x ^= 3; print(x); }", 27.into());
    run_and_check_registers!("fn main() { print(2 ^ 3 == 8); }", true.into());
}

/// `|` separates the members of a union type where a type is read, and is
/// bitwise or where an expression is. The two never meet: a type and an
/// expression are read by different parsers.
#[test]
pub fn a_union_type_still_parses() {
    run_and_check_registers!(
        "
        struct Cell { v: int | string }
        fn label(x: int | string, mask: int) -> int {
            return mask | 1;
        }
        fn main() {
            let c = Cell { v: \"hi\" };
            print(label(c.v, 6));
        }
        ",
        7.into()
    );
}

/// The two closing `>` of a nested type-argument list lex as the one `>>` the
/// shift is written with, so the type parser splits it. Both the list that is
/// parsed outright and the one the walk ahead of a call has to recognise are
/// covered.
#[test]
pub fn nested_generics_still_close_with_two_angle_brackets() {
    run_and_check_registers!(
        "
        struct Box<T> { v: T }
        fn pick<T>(a: T) -> T { return a; }
        fn unwrap(b: Box<Box<int>>) -> int { return b.v.v; }
        fn main() {
            let inner = Box<int>{ v: 7 };
            let outer = Box<Box<int>>{ v: inner };
            print(unwrap(pick<Box<Box<int>>>(outer)));
        }
        ",
        7.into()
    );
}

/// A comparison keeps reading as one next to the shift tokens.
#[test]
pub fn comparison_still_parses_beside_the_shifts() {
    run_and_check_registers!(
        "
        fn main() {
            let a = 1;
            let b = 2;
            let c = 3;
            let d = 0;
            print(a < b && c > d);
        }
        ",
        true.into()
    );
}

#[test]
pub fn string_index() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello";
            print(s[0] == "h");
        }
        "#,
        true.into()
    );
}

#[test]
pub fn string_set_index() {
    run_and_check_registers!(
        r#"
        fn main() {
            let s = "hello";
            s[0] = "He";
            print(s.len());
        }
        "#,
        6.into()
    );
}

#[test]
pub fn neq_3_4() {
    run_and_check_registers!(
        "
        fn main() {
            print(3 != 4);
        }
        ",
        true.into()
    );
}

#[test]
pub fn eq_3_4() {
    run_and_check_registers!(
        "
        fn main() {
            print(3 == 4);
        }
        ",
        false.into()
    );
}

#[test]
pub fn array_join_sep() {
    run_and_check_registers!(
        r#"
        fn main() {
            let arr = ["a", "b", "c"];
            let s = arr.join("--");
            print(s.len());
        }
        "#,
        7.into()
    );
}

#[test]
pub fn array_join_no_sep() {
    run_and_check_registers!(
        r#"
        fn main() {
            let arr = ["a", "b", "c"];
            let s = arr.join();
            print(s.len());
        }
        "#,
        3.into()
    );
}

#[test]
pub fn float_div_zero() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 1.0 / 0.0;
            print(x > 9999999.0);
        }
        ",
        true.into()
    );
}

#[test]
pub fn float_negative_pow() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 2.0 ^ -1.0;
            print(x);
        }
        ",
        0.5f64.into()
    );
}

#[test]
pub fn float_negative_pow_square() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 4.0 ^ -0.5;
            print(x);
        }
        ",
        0.5f64.into()
    );
}

/// `type` names a user type inside a map the way it already named one inside a
/// list: both halves of the map are rendered through the same formatter.
#[test]
pub fn type_function_names_a_map_of_a_user_type() {
    run_and_check_registers!(
        r#"
        enum Value { Num(int), Text(string) }
        fn main() {
            print(type({"a": Value::Num(1)}) == "{string: Value}");
        }
        "#,
        true.into()
    );
}

#[test]
pub fn type_function() {
    run_and_check_registers!(
        r#"
        fn main() {
            print(type(42)+type("hello")+type(3.14)+type(true) == "intstringfloatbool");
        }
        "#,
        true.into()
    );
}

#[test]
pub fn array_slice() {
    run_and_check_registers!(
        r"
        fn main() {
            let x = [0,1,2,3,4,5];
            let y = x[3..5];
            print(y[0]);
        }
        ",
        3.into()
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn array_slice_negative_index() {
    run_and_check_registers!(
        r"
        fn main() {
            let x = [0,1,2,3,4,5];
            let y = x[3..-5];
            print(y[0]);
        }
        ",
        3.into()
    );
}

#[test]
pub fn string_slice() {
    run_and_check_registers!(
        r#"
        fn main() {
            let x = "Hello world";
            let y = x[6..11];
            print(y);
        }
        "#,
        Data::small_str("world")
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn string_slice_negative_index() {
    run_and_check_registers!(
        r#"
        fn main() {
            let x = "Hello world";
            let y = x[-6..11];
            print(y);
        }
        "#,
        Data::small_str("world")
    );
}

#[test]
pub fn try_catch_no_error() {
    run_and_check_registers!(
        "
        fn main() {
            let result = 0;
            try {
                result = 1;
            } catch e {
                result = 2;
            }
            print(result);
        }
        ",
        1.into()
    );
}

#[test]
pub fn try_catch_catches_error() {
    run_and_check_registers!(
        "
        fn main() {
            let x = [0,1];
            let result = 0;
            try {
                print(x[5]);
                result = 1;
            } catch e {
                result = 2;
            }
            print(result);
        }
        ",
        2.into()
    );
}

#[test]
pub fn try_catch_filtered_match() {
    run_and_check_registers!(
        "
        fn main() {
            let x = [0,1];
            let result = 0;
            try {
                print(x[5]);
            } catch \"index_out_of_bounds\" {
                result = 1;
            } catch e {
                result = 2;
            }
            print(result);
        }
        ",
        1.into()
    );
}

#[test]
pub fn try_catch_filtered_fallthrough() {
    run_and_check_registers!(
        "
        fn main() {
            let x = [0,1];
            let result = 0;
            try {
                print(x[5]);
            } catch \"division_by_zero\" {
                result = 1;
            } catch e {
                result = 2;
            }
            print(result);
        }
        ",
        2.into()
    );
}

#[test]
pub fn throw_is_catchable() {
    run_and_check_registers!(
        "
        fn main() {
            let result = 0;
            try {
                throw(\"boom\");
                result = 1;
            } catch \"boom\" {
                result = 2;
            } catch e {
                result = 3;
            }
            print(result);
        }
        ",
        2.into()
    );
}

#[test]
pub fn try_catch_division_by_zero() {
    run_and_check_registers!(
        "
        fn main() {
            let z = 0;
            let result = 0;
            try {
                print(10 / z);
            } catch \"division_by_zero\" {
                result = 1;
            } catch e {
                result = 2;
            }
            print(result);
        }
        ",
        1.into()
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn try_catch_insufficient() {
    run!(
        "
        fn main() {
            let z = 0;
            let result = 0;
            try {
                print(10 / z);
            } catch \"invalid_int\" {
                result = 1;
            }
            print(result);
        }
        "
    );
}

#[test]
pub fn struct_field_access() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let p = Point { x: 7, y: 3 };
            print(p.x);
        }
        ",
        7.into()
    );
}

#[test]
pub fn struct_trailing_comma() {
    run!(
        "
        fn main() {
            struct Test {
                x: Test[],
                y: Test[],
            }
        }
        "
    );
}

#[test]
pub fn struct_field_modify() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let p = Point { x: 7, y: 3 };
            p.x = 42;
            print(p.x);
        }
        ",
        42.into()
    );
}

#[test]
pub fn struct_fields_exprs() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let n = 5;
            let p = Point { x: n + 1, y: n * 2 };
            print(p.x + p.y);
        }
        ",
        16.into()
    );
}

#[test]
pub fn struct_field_assign_shorthand() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let p = Point { x: 10, y: 0 };
            p.x += 5;
            print(p.x);
        }
        ",
        15.into()
    );
}

#[test]
pub fn struct_nested_field_access() {
    run_and_check_registers!(
        "
        struct Test { v: int }
        struct OtherTest { test: Test, i: int }
        fn main() {
            let o = OtherTest { test: Test { v: 99 }, i: 1 };
            print(o.test.v);
        }
        ",
        99.into()
    );
}

#[test]
pub fn struct_nested_field_modify() {
    run_and_check_registers!(
        "
        struct Test { v: int }
        struct OtherTest { test: Test, i: int }
        fn main() {
            let o = OtherTest { test: Test { v: 99 }, i: 1 };
            o.test.v = 50;
            print(o.test.v);
        }
        ",
        50.into()
    );
}

#[test]
pub fn struct_passed_to_function() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn get_x(p) {
            return p.x;
        }
        fn main() {
            let p = Point { x: 8, y: 2 };
            print(get_x(p));
        }
        ",
        8.into()
    );
}

#[test]
pub fn struct_functin_ret() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn make(n) {
            return Point { x: n, y: n + 1 };
        }
        fn main() {
            let p = make(5);
            print(p.y);
        }
        ",
        6.into()
    );
}

#[test]
pub fn struct_array_field_access() {
    run_and_check_registers!(
        "
        struct Container { items: int[] }
        fn main() {
            let b = Container { items: [10, 20, 30] };
            print(b.items[1]);
        }
        ",
        20.into()
    );
}

#[test]
pub fn struct_array_field_modify() {
    run_and_check_registers!(
        "
        struct Container { items: int[] }
        fn main() {
            let b = Container { items: [10, 20, 30] };
            b.items[2] = 99;
            print(b.items[2]);
        }
        ",
        99.into()
    );
}

#[test]
pub fn struct_nested_array_field_access() {
    run_and_check_registers!(
        "
        struct Matrix { cells: int[][] }
        fn main() {
            let g = Matrix { cells: [[1, 2], [3, 4]] };
            print(g.cells[1][0]);
        }
        ",
        3.into()
    );
}

#[test]
pub fn struct_nested_array_field_modify() {
    run_and_check_registers!(
        "
        struct Matrix { cells: int[][] }
        fn main() {
            let g = Matrix { cells: [[1, 2], [3, 4]] };
            g.cells[0][1] = 77;
            print(g.cells[0][1]);
        }
        ",
        77.into()
    );
}

#[test]
pub fn struct_structs_array_access() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let arr = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
            print(arr[1].x);
        }
        ",
        3.into()
    );
}

#[test]
pub fn struct_structs_array_modify() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let arr =[Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
            arr[0].y = 50;
            print(arr[0].y);
        }
        ",
        50.into()
    );
}

#[test]
pub fn struct_eq_true() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let a = Point { x: 1, y: 2 };
            let b = Point { x: 1, y: 2 };
            print(a == b);
        }
        ",
        true.into()
    );
}

#[test]
pub fn struct_eq_false() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let a = Point { x: 1, y: 2 };
            let b = Point { x: 1, y: 9 };
            print(a == b);
        }
        ",
        false.into()
    );
}

#[test]
pub fn struct_deep_structural_eq() {
    run_and_check_registers!(
        "
        struct Inner { v: int }
        struct Outer { inner: Inner }
        fn main() {
            let a = Outer { inner: Inner { v: 5 } };
            let b = Outer { inner: Inner { v: 5 } };
            print(a == b);
        }
        ",
        true.into()
    );
}

#[test]
pub fn struct_ref() {
    run_and_check_registers!(
        "
        struct Box { v: int }
        fn main() {
            let a = Box { v: 1 };
            let b = a;
            b.v = 9;
            print(a.v);
        }
        ",
        9.into()
    );
}

#[test]
pub fn struct_field_condition() {
    run_and_check_registers!(
        "
        struct Point { x: int }
        fn main() {
            let p = Point { x: 7 };
            if p.x > 5 {
                print(1);
            } else {
                print(0);
            }
        }
        ",
        1.into()
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn struct_unknown_name() {
    run!(
        "
        fn main() {
            let a = Idk { x: 1 };
        }
        "
    );
}

#[test]
#[ignore = "stalls CI runners; run with --ignored"]
#[should_panic(expected = "explicit panic")]
pub fn struct_missing_field() {
    run!(
        "
        struct Point { x: int, y: int }
        fn main() {
            let a = Point { x: 67 };
        }
        "
    );
}

#[test]
#[ignore = "stalls CI runners; run with --ignored"]
#[should_panic(expected = "explicit panic")]
pub fn struct_unknown_field() {
    run!(
        "
        struct Point { x: int }
        fn main() {
            let a = Point { z: 67 };
        }
        "
    );
}

#[test]
#[ignore = "stalls CI runners; run with --ignored"]
#[should_panic(expected = "explicit panic")]
pub fn struct_field_wrong_type() {
    run!(
        "
        struct Point { x: int }
        fn main() {
            let a = Point { x: true };
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn struct_unknown_field_access() {
    run!(
        "
        struct Point { x: int }
        fn main() {
            let a = Point { x: 67 };
            print(a.z);
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn struct_field_assign_wrong_type() {
    run!(
        "
        struct Point { x: int }
        fn main() {
            let a = Point { x: 1 };
            a.x = true;
        }
        "
    );
}

#[test]
pub fn nbody() {
    run_and_check_registers!(
        r"
        struct Body {
            x: float,
            y: float,
            z: float,
            vx: float,
            vy: float,
            vz: float,
            mass: float
        }

        fn combinations(l) {
            let result = [];
            for x in ..l.len() - 1 {
                let ls = l[x+1..l.len()];
                for y in ls {
                    result.push([l[x], y]);
                }
            }
            return result;
        }

        fn advance(dt, n, bodies, pairs) {
            for _ in ..n {
                for pair in pairs {
                    let b1 = pair[0];
                    let b2 = pair[1];
                    let dx = b1.x - b2.x;
                    let dy = b1.y - b2.y;
                    let dz = b1.z - b2.z;
                    let mag = dt * ((dx * dx + dy * dy + dz * dz) ^ -1.5);
                    let b1m = b1.mass * mag;
                    let b2m = b2.mass * mag;
                    b1.vx -= dx * b2m;
                    b1.vy -= dy * b2m;
                    b1.vz -= dz * b2m;
                    b2.vx += dx * b1m;
                    b2.vy += dy * b1m;
                    b2.vz += dz * b1m;
                }
                for body in bodies {
                    body.x += dt * body.vx;
                    body.y += dt * body.vy;
                    body.z += dt * body.vz;
                }
            }
        }

        fn report_energy(bodies, pairs) {
            let e = 0.0;
            for pair in pairs {
                let b1 = pair[0];
                let b2 = pair[1];
                let dx = b1.x - b2.x;
                let dy = b1.y - b2.y;
                let dz = b1.z - b2.z;
                e -= (b1.mass * b2.mass) / (dx * dx + dy * dy + dz * dz).sqrt();
            }
            for body in bodies {
                e += body.mass * (body.vx * body.vx + body.vy * body.vy + body.vz * body.vz) / 2.0;
            }
            print(e);
        }

        fn offset_momentum(ref, bodies) {
            let px = 0.0;
            let py = 0.0;
            let pz = 0.0;
            for body in bodies {
                px -= body.vx * body.mass;
                py -= body.vy * body.mass;
                pz -= body.vz * body.mass;
            }
            ref.vx = px / ref.mass;
            ref.vy = py / ref.mass;
            ref.vz = pz / ref.mass;
        }

        fn main() {
            let PI = 3.14159265358979323;
            let SOLAR_MASS = 4.0 * PI * PI;
            let DAYS_PER_YEAR = 365.24;

            let sun = Body {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                vx: 0.0,
                vy: 0.0,
                vz: 0.0,
                mass: SOLAR_MASS
            };

            let jupiter = Body {
                x: 4.84143144246472090,
                y: -1.16032004402742839,
                z: -0.103622044471123109,
                vx: 0.00166007664274403694 * DAYS_PER_YEAR,
                vy: 0.00769901118419740425 * DAYS_PER_YEAR,
                vz: -0.0000690460016974260023 * DAYS_PER_YEAR,
                mass: 0.000954791938424326609 * SOLAR_MASS
            };

            let saturn = Body {
                x: 8.34336671824457987,
                y: 4.12479856412430479,
                z: -0.403523417114321381,
                vx: -0.00276742510726862411 * DAYS_PER_YEAR,
                vy: 0.00499852801234917238 * DAYS_PER_YEAR,
                vz: 0.0000230417297573763929 * DAYS_PER_YEAR,
                mass: 0.000285885980666130812 * SOLAR_MASS
            };

            let uranus = Body {
                x: 12.8943695621391310,
                y: -15.1111514016986312,
                z: -0.223307578892655734,
                vx: 0.00296460137564761618 * DAYS_PER_YEAR,
                vy: 0.00237847173959480950 * DAYS_PER_YEAR,
                vz: -0.0000296589568540237556 * DAYS_PER_YEAR,
                mass: 0.0000436624404335156298 * SOLAR_MASS
            };

            let neptune = Body {
                x: 15.3796971148509165,
                y: -25.9193146099879641,
                z: 0.179258772950371181,
                vx: 0.00268067772490389322 * DAYS_PER_YEAR,
                vy: 0.00162824170038242295 * DAYS_PER_YEAR,
                vz: -0.0000951592254519715870 * DAYS_PER_YEAR,
                mass: 0.0000515138902046611451 * SOLAR_MASS
            };

            let bodies = [sun, jupiter, saturn, uranus, neptune];
            let pairs = combinations(bodies);

            offset_momentum(sun, bodies);
            report_energy(bodies, pairs);
            advance(0.01, 10, bodies, pairs);
            report_energy(bodies, pairs);
        }
        ",
        (-0.169_073_021_714_699_8).into()
    );
}

#[test]
pub fn loop_function_reg_interference() {
    run_and_check_registers!(
        r"
        struct Test { v: int }
        fn f(s) { return 0; }

        fn run(x) {
            let j = 0;
            loop {
                f(x);
                j += 1;
                if j >= 1 { break; }
            }
            return j;
        }

        fn main() {
            print(run(Test { v: 42 }));
        }
        ",
        1.into()
    );
}

#[test]
pub fn map_init() {
    run!(
        "
        fn main() {
            let m = {\"test\": 42, \"othertest\": 67};
        }
        "
    );
}

#[test]
pub fn map_get_key() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {0: 42, 1: 67};
            print(m.get(0));
        }
        ",
        42.into()
    );
}

#[test]
pub fn map_insert_new_pair() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {[0,1,2]: 0, [3,4,5]: 1};
            let a = [6,7,8];
            m.insert(a, 2);
            print(m.get(a));
        }
        ",
        2.into()
    );
}

#[test]
pub fn map_overwrite_pair() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {false: \"false\", true: \"true\"};
            m.insert(false, \"true?\");
            print(m.get(false).len());
        }
        ",
        5.into()
    );
}

#[test]
pub fn map_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let sum = 0;
            for _ in 0..10 {
                let x = 10;
                let m = {1.0: x+10, 2.0: x+20};
                sum += m.get(1.0) + m.get(2.0);
            }
            print(sum);
        }",
        500.into()
    );
}

// ---------------------------------------------------------------------------
// STRUCTURED DIAGNOSTICS
//
// The error funnels (throw_parser_error, throw_compiler_error, and
// RuntimeError::report for a run-time error) record a structured `Diagnostic`
// and unwind instead of printing + exiting whenever `collect_diagnostic` has
// installed a sink on the thread.
// Without a sink the CLI path is byte-for-byte unchanged.
// ---------------------------------------------------------------------------

use crate::Diagnostic;
use crate::errors::collect_diagnostic;
use crate::errors::strip_ansi;
use candela_vm::captured_output::CAPTURED_OUTPUT;
use candela_vm::captured_output::set_capturing;

/// Compiles `src` under a diagnostic sink, returning the first structured error
/// (parser or compiler) instead of printing + exiting. This is exactly what an
/// embedder would write against the public `collect_diagnostic` surface.
///
/// The release profile compiles the same program, so it has to report the
/// same error, word for word and at the same span; this checks that too.
fn compile_diag(src: &str, filename: &str) -> Result<(), Diagnostic> {
    let [debug, release] = [false, true].map(|optimize| {
        collect_diagnostic(|| {
            let _ = crate::compiler::compile_profile(
                String::from(src),
                filename,
                false,
                &crate::compiler::imports::ImportResolver::new(),
                optimize,
            );
        })
    });
    assert_eq!(
        debug, release,
        "a release build reports what a debug build does"
    );
    debug
}

/// Compiles then executes `src` under a diagnostic sink, surfacing parser,
/// compiler and runtime errors as structured `Diagnostic`s.
///
/// Both profiles run it, and a release build has to stop with the same error
/// at the same span as a debug one.
fn run_diag(src: &str, filename: &str) -> Result<(), Diagnostic> {
    let [debug, release] = [false, true].map(|optimize| run_diag_profile(src, filename, optimize));
    assert_eq!(
        debug, release,
        "a release build stops where a debug build does"
    );
    native_agrees(
        src,
        filename,
        &crate::compiler::imports::ImportResolver::new(),
    );
    debug
}

/// [`run_diag`] in one profile.
fn run_diag_profile(src: &str, filename: &str, optimize: bool) -> Result<(), Diagnostic> {
    run_diag_profile_with(
        src,
        filename,
        optimize,
        &crate::compiler::imports::ImportResolver::new(),
    )
}

/// [`run_diag_profile`] with the imports resolved by `resolver`.
fn run_diag_profile_with(
    src: &str,
    filename: &str,
    optimize: bool,
    resolver: &crate::compiler::imports::ImportResolver,
) -> Result<(), Diagnostic> {
    collect_diagnostic(|| {
        let out = crate::compiler::compile_profile(
            String::from(src),
            filename,
            false,
            resolver,
            optimize,
        );
        let mut arrays = out.pools;
        run_vm(
            &out.instructions,
            &mut RegisterFile(out.registers),
            &mut arrays,
            &crate::errors::ErrorCtx {
                instr_src: &out.instr_src,
                sources: &[Source {
                    filename: filename.into(),
                    contents: String::from(src),
                }],
            },
            &out.callsite_registers,
            &[],
            &out.structs,
            &out.enums,
            out.allocated_arg_count,
            out.allocated_call_depth,
            &[],
            &[],
            0,
        );
    })
}

/// Compiles `src` with output redirected into this thread's capture buffer and
/// hands back the error report as a terminal would receive it, escape codes
/// and all.
///
/// `compile_diag` cannot answer for that: under a diagnostic sink the error
/// funnel records the plain message and unwinds before the coloured report is
/// built. The compile ends in that unwind either way, so it is caught here and
/// the buffer read afterwards.
fn compile_report(src: &str, filename: &str) -> String {
    CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
    let was_capturing = set_capturing(true);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = compile(
            String::from(src),
            filename,
            false,
            &crate::compiler::imports::ImportResolver::new(),
        );
    }));
    set_capturing(was_capturing);
    CAPTURED_OUTPUT.with(|o| o.take())
}

/// Compiles and runs `src` with output captured, and hands back what the
/// program printed.
///
/// Both profiles run it, and a release build has to print exactly what a
/// debug one does.
fn run_output(src: &str) -> String {
    let [debug, release] = [false, true].map(|optimize| {
        CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
        let was_capturing = set_capturing(true);
        let result = run_diag_profile(src, "out.cdl", optimize);
        set_capturing(was_capturing);
        let printed = CAPTURED_OUTPUT.with(|o| o.take());
        result.unwrap_or_else(|d| panic!("{}: {printed}", d.message));
        printed
    });
    assert_eq!(
        debug, release,
        "a release build prints what a debug build does"
    );
    native_agrees(
        src,
        "out.cdl",
        &crate::compiler::imports::ImportResolver::new(),
    );
    debug
}

/// Every diagnostic must carry a plain-text (ANSI-free) message, a non-empty
/// machine-readable code, and a well-formed byte span into the source.
fn assert_wellformed(d: &Diagnostic, src: &str) {
    assert!(!d.message.is_empty(), "empty message: {d:?}");
    assert!(!d.message.contains('\x1B'), "ANSI leaked into: {d:?}");
    assert!(!d.code.is_empty(), "empty code: {d:?}");
    assert!(d.span.start <= d.span.end, "inverted span: {d:?}");
    assert!(
        d.span.end <= src.len(),
        "span {:?} exceeds source len {} ({d:?})",
        d.span,
        src.len()
    );
}

#[test]
pub fn diagnostics_parser_error_span() {
    let src = "fn main() { let x = 1 }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.filename, "diag.kl");
    assert_eq!(d.message, "Missing semicolon");
    assert_eq!(d.code, "missing_semicolon");
    // The span points right after the last token before the missing ';'
    assert_eq!(d.span, 21..21);
}

/// `int` is 64 bits wide, so a literal past its range is a compile error at
/// the literal, naming the range it has to sit in.
#[test]
pub fn diagnostics_int_literal_past_the_range() {
    let src = "fn main() { print(9223372036854775809); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "int_literal_out_of_range");
    assert!(d.message.contains("-9223372036854775808"), "{}", d.message);
    assert!(d.message.contains("9223372036854775807"), "{}", d.message);
    assert_eq!(&src[d.span], "9223372036854775809");
}

/// A literal past what the number parser underneath can read reaches the same
/// error, rather than a different one from that parser.
#[test]
pub fn diagnostics_int_literal_past_64_bits() {
    let src = "fn main() { print(99999999999999999999999); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "int_literal_out_of_range");
    assert_eq!(&src[d.span], "99999999999999999999999");
}

/// A `float` literal whose value is past what double precision holds is a
/// compile error at the literal, the way an out-of-range `int` literal is. The
/// sign is a token of its own, so both ends of the range are refused.
#[test]
pub fn diagnostics_float_literal_past_the_range() {
    let src = "fn main() { print(1e400); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "float_literal_out_of_range");
    assert!(d.message.contains("float"), "{}", d.message);
    assert_eq!(&src[d.span], "1e400");

    let src = "fn main() { print(-1.8e308); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "float_literal_out_of_range");
    assert_eq!(&src[d.span], "1.8e308");
}

/// The largest `float` there is still reads, so the check refuses only what
/// overflows.
#[test]
pub fn the_largest_float_literal_is_read() {
    run_and_check_registers!(
        "fn main() { print(1.7976931348623157e308); }",
        f64::MAX.into()
    );
}

/// The literal one past the top of the range is read, because negating it is
/// the only way to write the smallest `int`.
#[test]
pub fn smallest_int_literal_is_read_through_its_sign() {
    run_and_check_registers!(
        "fn main() { print(-9223372036854775808); }",
        i64::MIN.into()
    );
}

/// The same literal without the sign in front of it is out of range like its
/// neighbours, rather than reading as the minimum.
#[test]
pub fn diagnostics_int_literal_one_past_the_top_needs_its_sign() {
    let src = "fn main() { print(9223372036854775808); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "int_literal_out_of_range");
    assert_eq!(&src[d.span], "9223372036854775808");
    run_and_check_registers!(
        "fn main() { print(- 9223372036854775808 + 1); }",
        (i64::MIN + 1).into()
    );
}

/// Two literals fold at parse time, and the fold wraps the way the runtime
/// does, so a debug build of the compiler agrees with a release one.
#[test]
pub fn folding_an_overflowing_constant_wraps() {
    run_and_check_registers!(
        "fn main() { print(9223372036854775807 + 1); }",
        i64::MIN.into()
    );
    run_and_check_registers!(
        "fn main() { print(-9223372036854775808 - 1); }",
        i64::MAX.into()
    );
    run_and_check_registers!("fn main() { print(4294967296 * 4294967296); }", 0.into());
}

/// An `int` holds 64 bits: a literal at the top of the range reads back as
/// itself rather than as whatever fits in a narrower one.
#[test]
pub fn int_literal_holds_the_whole_range() {
    run_and_check_registers!("fn main() { print(9223372036854775807); }", i64::MAX.into());
}

/// Arithmetic past 32 bits answers with the value, not with a wrap. The
/// operands reach the VM through a call, so the compiler computes nothing here
/// that the interpreter does not.
#[test]
pub fn arithmetic_past_32_bits() {
    run_and_check_registers!(
        "
        fn add(a, b) { return a + b; }
        fn main() { print(add(2147483647, 1)); }
        ",
        2_147_483_648_i64.into()
    );
    run_and_check_registers!(
        "
        fn mul(a, b) { return a * b; }
        fn main() { print(mul(65536, 65536)); }
        ",
        4_294_967_296_i64.into()
    );
    run_and_check_registers!(
        "
        fn div(a, b) { return a / b; }
        fn main() { print(div(9000000000, 3)); }
        ",
        3_000_000_000_i64.into()
    );
    run_and_check_registers!(
        "
        fn rem(a, b) { return a % b; }
        fn main() { print(rem(9000000001, 9000000000)); }
        ",
        1_i64.into()
    );
    run_and_check_registers!(
        "
        fn pow(a, b) { return a ^ b; }
        fn main() { print(pow(2, 62)); }
        ",
        4_611_686_018_427_387_904_i64.into()
    );
    run_and_check_registers!(
        "
        fn sub(a, b) { return a - b; }
        fn main() { print(sub(0, 9000000000)); }
        ",
        (-9_000_000_000_i64).into()
    );
}

/// Comparison reads all 64 bits, so two values that share their low 32 do not
/// compare equal.
#[test]
pub fn comparison_past_32_bits() {
    run_and_check_registers!(
        "
        fn cmp(a, b) { return a > b; }
        fn main() { print(cmp(4294967296, 2147483647)); }
        ",
        true.into()
    );
    run_and_check_registers!(
        "
        fn eq(a, b) { return a == b; }
        fn main() { print(eq(4294967296, 0)); }
        ",
        false.into()
    );
}

/// `int` and `str` carry the whole width in both directions.
#[test]
pub fn conversions_past_32_bits() {
    run_and_check_registers!(
        "fn main() { print(int(\"3000000000\")); }",
        3_000_000_000_i64.into()
    );
    run_and_check_registers!(
        "
        fn main() {
            let s = str(9223372036854775807);
            print(s == \"9223372036854775807\");
        }
        ",
        true.into()
    );
    run_and_check_registers!(
        "
        fn main() {
            print(int(4000000000.5));
        }
        ",
        4_000_000_000_i64.into()
    );
}

/// A range runs over bounds past 32 bits.
#[test]
pub fn range_over_large_bounds() {
    run_and_check_registers!(
        "
        fn main() {
            let total = 0;
            for i in 4294967296..4294967299 {
                total += i;
            }
            print(total);
        }
        ",
        12_884_901_891_i64.into()
    );
}

/// An index past what 32 bits could hold is out of bounds, and says so rather
/// than reading whatever the low half of it points at.
#[test]
pub fn index_past_32_bits_is_out_of_bounds() {
    let src = "fn main() { let a = [1, 2, 3]; print(a[4294967296]); }";
    let d = run_diag(src, "index.kl").unwrap_err();
    assert_eq!(d.code, "index_out_of_bounds");
    assert!(d.message.contains("4294967296"), "message: {:?}", d.message);
}

/// JSON carries an integer a double could not hold exactly, in and out.
#[test]
pub fn json_round_trips_a_large_integer() {
    run_and_check_registers!(
        "
        fn main() {
            let v = as_int(json_parse(\"9007199254740993\"));
            print(v);
        }
        ",
        9_007_199_254_740_993_i64.into()
    );
    run_and_check_registers!(
        "
        fn main() {
            let s = json_stringify(9007199254740993);
            print(s == \"9007199254740993\");
        }
        ",
        true.into()
    );
}

/// An exponent with no digits after it is an error that names what is
/// missing, rather than a number followed by a stray name.
#[test]
pub fn diagnostics_float_exponent_without_digits() {
    let src = "fn main() { print(1e); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "float_exponent_missing_digits");
    assert_eq!(&src[d.span], "1e");
}

/// A callee that holds no function is a compile error naming the type that
/// cannot be called.
#[test]
pub fn diagnostics_calling_a_value_that_is_not_a_function() {
    let src = "fn one() { return 1; }\nfn main() { print(one()(2)); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "type_not_callable");
    assert!(d.message.contains("int"), "{}", d.message);
}

#[test]
pub fn diagnostics_compile_error_span() {
    let src = "fn main() { let x = 1 + \"a\"; }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.filename, "diag.kl");
    assert_eq!(d.message, "Cannot perform operation int + string");
    // Compiler errors now carry a specific code, not the old blanket "compile_error".
    assert_eq!(d.code, "invalid_operation");
    // The span covers the whole offending operation
    assert_eq!(&src[d.span], "1 + \"a\"");
}

/// Ordering is per type, so a string next to an int is still a compile error,
/// and the note names the three types the operator does order.
#[test]
pub fn diagnostics_comparing_a_string_with_an_int() {
    let src = "fn main() { print(\"a\" < 1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation string < int");
    let report = strip_ansi(&compile_report(src, "diag.kl"));
    assert!(report.contains("string < string"), "{report}");
}

#[test]
pub fn diagnostics_unknown_variable_is_plain_text() {
    let src = "fn main() { print(cuont); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.message, "Cannot find variable cuont in this scope");
    assert_eq!(&src[d.span], "cuont");
}

#[test]
pub fn diagnostics_missing_main() {
    // Compiling a file with no `main` is not an error, which is what `candela
    // check` does with a library entry. Starting a program from one is, and the
    // report comes from there.
    let src = "fn foo() { return 1; }";
    compile_diag(src, "diag.kl").expect("a file without main compiles");
    let d = collect_diagnostic(|| {
        compile(
            String::from(src),
            "diag.kl",
            false,
            &crate::compiler::imports::ImportResolver::new(),
        )
        .require_main();
    })
    .unwrap_err();
    assert_eq!(
        d.message,
        "No main function: a program is entered through main"
    );
    assert_eq!(d.code, "no_main_function");
}

#[test]
pub fn diagnostics_runtime_error_code_and_span() {
    let src = "fn main() { let a = [1]; print(a[5]); }";
    let d = run_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.filename, "diag.kl");
    assert_eq!(d.code, "index_out_of_bounds");
    assert_wellformed(&d, src);
    assert!(
        d.message.contains('5') && d.message.contains('1'),
        "message: {:?}",
        d.message
    );
    assert!(src[d.span.clone()].contains("a[5]"), "span: {:?}", d.span);
}

#[test]
pub fn diagnostics_legacy_namespaced_import() {
    // The removed `import std::list;` form parses far enough to suggest the
    // exact quoted-path replacement.
    let src = "import std::list;\nfn main() { print(1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "legacy_namespaced_import");
    assert!(
        d.message.contains("import \"std/list\";"),
        "message: {:?}",
        d.message
    );
    assert_eq!(&src[d.span], "std::list");
}

#[test]
pub fn diagnostics_import_path_bad_extension() {
    // An import path either ends in `.cdl` or has no extension.
    let src = "import \"helpers.txt\";\nfn main() { print(1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "import_path_bad_extension");
}

#[test]
pub fn diagnostics_sink_is_scoped() {
    // A failed collection must not poison later compilations (or the CLI path)
    assert!(compile_diag("fn main() { print(1); }", "ok.kl").is_ok());
    assert!(compile_diag("fn main() { print(nope); }", "bad.kl").is_err());
    assert!(compile_diag("fn main() { print(2); }", "ok2.kl").is_ok());
}

// ---------------------------------------------------------------------------
// STRESS TESTS
//
// Drive thousands of malformed / garbage inputs through each error funnel and
// assert none of them panic with a Rust backtrace or exit the process: every
// failure must come back as a well-formed structured `Diagnostic`.
// ---------------------------------------------------------------------------

/// Tiny deterministic xorshift PRNG so the corpus is reproducible without
/// pulling in an `rand` dependency.
struct Rng(u64);
impl Rng {
    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    const fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
#[ignore = "stalls CI runners; run with --ignored"]
pub fn stress_parser_random_garbage() {
    // Random printable ASCII, random keyword/punctuation soup, unbalanced
    // brackets and quotes, and truncated valid programs.
    // NOTE: deliberately no i32-overflowing integer literal in this corpus.
    // Such a literal panics in the lexer (lexer.rs `panic!("Invalid float")`)
    // rather than routing through throw_parser_error, a pre-existing keel bug,
    // out of scope for this change. See the crate notes / PR description.
    let tokens = [
        "fn", "main", "let", "if", "else", "while", "for", "return", "match", "try", "catch",
        "struct", "(", ")", "{", "}", "[", "]", ",", ";", ":", "+", "-", "*", "/", "%", "=", "==",
        "\"", "\"abc", "1", "42", "x", "::", ".", "->", "true", "print",
    ];
    let mut rng = Rng(0x1234_5678_9abc_def0);
    let mut errors = 0usize;
    let mut ok = 0usize;
    for _ in 0..4000 {
        let kind = rng.below(3);
        let src = match kind {
            0 => {
                // random printable-ASCII byte soup
                let len = rng.below(40);
                (0..len)
                    .map(|_| char::from(32 + rng.below(95) as u8))
                    .collect::<String>()
            }
            1 => {
                // random token soup (unbalanced by construction)
                let len = rng.below(30);
                let mut s = String::new();
                for _ in 0..len {
                    s.push_str(tokens[rng.below(tokens.len())]);
                    s.push(' ');
                }
                s
            }
            _ => {
                // truncate a valid program at a random byte
                let base = "fn main() { let x = [1, 2, 3]; if x[0] == 1 { print(x); } }";
                let cut = rng.below(base.len() + 1);
                String::from(&base[..cut])
            }
        };
        match compile_diag(&src, "fuzz.kl") {
            Ok(()) => ok += 1,
            Err(d) => {
                assert_wellformed(&d, &src);
                errors += 1;
            }
        }
    }
    // The corpus is overwhelmingly invalid; make sure the error path was
    // exercised (and that at least some inputs still compiled cleanly).
    assert!(errors > 3000, "only {errors} errors from 4000 inputs");
    assert!(ok > 0, "no inputs compiled");
}

#[test]
pub fn stress_parser_unbalanced_and_huge() {
    let mut cases: Vec<String> = Vec::new();
    // Deeply nested but unclosed / closed delimiters.
    for depth in [1usize, 8, 64, 256, 1024] {
        cases.push(format!(
            "fn main() {{ let x = {}1{}",
            "(".repeat(depth),
            ")".repeat(depth)
        ));
        cases.push(format!("fn main() {{ let x = {}1;", "[".repeat(depth)));
        cases.push(format!("fn main() {{ {}", "{".repeat(depth)));
    }
    // Huge identifier and a huge (but valid, within-i32) numeric literal.
    // An i32-overflowing literal is intentionally omitted here; it hits a
    // pre-existing lexer panic rather than a structured error (see the corpus
    // note in stress_parser_random_garbage).
    cases.push(format!("fn main() {{ let {} = 1; }}", "a".repeat(50_000)));
    cases.push(format!(
        "fn main() {{ let x = {}00000000; }}",
        "0".repeat(50_000)
    ));
    // Unterminated string of growing size.
    for n in [1usize, 100, 10_000] {
        cases.push(format!("fn main() {{ let s = \"{}", "z".repeat(n)));
    }
    // Empty and whitespace-only inputs.
    cases.push(String::new());
    cases.push(String::from("   \n\t  "));
    cases.push(String::from(";;;;;;"));

    for src in &cases {
        // Must return a value (Ok or a well-formed Err), never panic/abort.
        if let Err(d) = compile_diag(src, "fuzz.kl") {
            assert_wellformed(&d, src);
        }
    }
}

#[test]
pub fn stress_compiler_semantic_errors() {
    // Each of these parses but fails during type/name/arity checking. Every one
    // must yield a well-formed compiler diagnostic rather than exiting, and must
    // carry a specific stable code, never the blanket
    // "compile_error" that every compiler error used to collapse to. The
    // (source, expected code) pairing pins the codes so they can't silently
    // drift.
    let cases = [
        ("fn main() { let x = 1 + \"a\"; }", "invalid_operation"), // type mismatch in op
        ("fn main() { print(undefined_var); }", "unknown_variable"), // undefined symbol
        ("fn main() { undefined_fn(); }", "unknown_function"),     // undefined function
        ("fn main() { let x = true - false; }", "invalid_operation"), // bad operator operands
        (
            "fn main() { let x = [1, \"a\"]; }",
            "array_element_type_mismatch",
        ), // heterogeneous array
        (
            "fn foo(a: int) { return a; } fn main() { foo(); }",
            "arity_mismatch",
        ), // too few args
        (
            "fn foo(a: int) { return a; } fn main() { foo(1, 2); }",
            "arity_mismatch",
        ), // too many args
        (
            "fn main() { return 1; } fn main() { return 2; }",
            "function_already_defined",
        ), // redefined
        (
            "fn foo(a: notatype) { return a; } fn main() { foo(1); }",
            "unknown_type",
        ), // unknown type annotation
        ("fn main() { let x = 1 / true; }", "invalid_operation"),  // bad division operands
    ];
    let mut distinct = std::collections::BTreeSet::new();
    for (src, expected_code) in cases {
        let d = compile_diag(src, "sem.kl")
            .err()
            .unwrap_or_else(|| panic!("expected a diagnostic for: {src}"));
        assert_wellformed(&d, src);
        // No compiler error may fall back to the old blanket code.
        assert_ne!(d.code, "compile_error", "blanket code for: {src}");
        assert_eq!(d.code, expected_code, "unexpected code for: {src}");
        distinct.insert(d.code.clone());
    }
    // The corpus deliberately spans many distinct error kinds; make sure the
    // codes differentiate them rather than all being equal.
    assert!(
        distinct.len() >= 6,
        "expected the compiler errors to be differentiated by code, saw only {distinct:?}"
    );
}

#[test]
pub fn genuine_panic_propagates_through_collect() {
    // A panic that signals a bug (not the internal FatalError unwind that
    // carries a Diagnostic) must not be captured as a diagnostic: it has to
    // propagate out of `collect_diagnostic` unchanged via `resume_unwind`.
    // Keep the test output clean by installing a no-op hook for the duration;
    // `collect_diagnostic` captures and forwards to it, then restores it.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(|| {
        let _ = collect_diagnostic(|| {
            panic!("genuine boom");
        });
    });
    std::panic::set_hook(previous);

    let payload = outcome.expect_err("collect_diagnostic must not swallow a genuine panic");
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    assert_eq!(message, "genuine boom");
}

#[test]
pub fn concurrent_collections_and_panics_do_not_wedge() {
    // The silencing hook is process-global: collecting threads install and
    // remove it while other threads run it by panicking. Neither side may wait
    // on a lock the other holds, so this must finish rather than park forever.
    // The no-op hook keeps the probe panics out of the test output.
    const ROUNDS: usize = 2000;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut threads = Vec::new();
    for _ in 0..2 {
        threads.push(std::thread::spawn(|| {
            for _ in 0..ROUNDS {
                assert!(compile_diag("fn main() { let x = ", "wedge.cdl").is_err());
            }
        }));
    }
    for _ in 0..16 {
        threads.push(std::thread::spawn(|| {
            for _ in 0..ROUNDS {
                let _ = std::panic::catch_unwind(|| panic!("hook probe"));
            }
        }));
    }
    for thread in threads {
        thread.join().expect("thread finished");
    }

    std::panic::set_hook(previous);
}

#[test]
pub fn stress_compiler_large_and_recursive() {
    use std::fmt::Write as _;
    // Large generated program (many statements + many functions).
    let mut big = String::from("fn main() {\n");
    for i in 0..5000 {
        writeln!(big, "    let v{i} = {i};").unwrap();
    }
    big.push_str("    print(1);\n}\n");
    for i in 0..500 {
        writeln!(big, "fn helper{i}(a: int) {{ return a + {i}; }}").unwrap();
    }
    assert!(
        run_diag(&big, "big.kl").is_ok(),
        "large valid program should run"
    );

    // Mutually recursive type inference must terminate (not loop / overflow).
    let mutual = "fn a() { return b(); } fn b() { return a(); } fn main() { print(1); }";
    let _ = compile_diag(mutual, "mutual.kl"); // Ok or Err, must not hang/abort.

    // Self-recursive function (keel's tested idiom uses unannotated params).
    let selfrec =
        "fn f(n) { if n == 0 { return 0; } return f(n - 1); } fn main() { print(f(10)); }";
    assert!(run_diag(selfrec, "rec.kl").is_ok());
}

#[test]
pub fn stress_runtime_errors() {
    // Each triggers a runtime fault that must surface as a structured diagnostic
    // with the expected stable code.
    let cases = [
        (
            "fn main() { let x = 1 / (1 - 1); print(x); }",
            "division_by_zero",
        ),
        (
            "fn main() { let x = 1 % (1 - 1); print(x); }",
            "modulo_by_zero",
        ),
        (
            "fn main() { let a = [1, 2, 3]; print(a[10]); }",
            "index_out_of_bounds",
        ),
        (
            "fn main() { let a = [1, 2, 3]; let i = 0 - 1; print(a[i]); }",
            "index_out_of_bounds",
        ),
    ];
    for (src, code) in cases {
        let d = run_diag(src, "rt.kl")
            .err()
            .unwrap_or_else(|| panic!("expected a runtime diagnostic for: {src}"));
        assert_wellformed(&d, src);
        assert_eq!(d.code, code, "for source: {src}");
    }
}

// ---------------------------------------------------------------------------
// OBJECT-ORIENTED METHOD SYNTAX (impl blocks)
//
// `impl Type { fn method(self, ...) { ... } }` lowers each method to a mangled
// per-type free function (`Type#method`) taking the receiver as argument 0.
// There is no runtime dispatch: the VM only ever sees ordinary function calls.
// ---------------------------------------------------------------------------

#[test]
pub fn method_basic_call() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        impl Point {
            fn sum(self) { return self.x + self.y; }
        }
        fn main() {
            let p = Point { x: 2, y: 3 };
            print(p.sum());
        }
        ",
        5.into()
    );
}

#[test]
pub fn method_with_extra_arg() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        impl Point {
            fn scaled_sum(self, factor) { return (self.x + self.y) * factor; }
        }
        fn main() {
            let p = Point { x: 2, y: 3 };
            print(p.scaled_sum(4));
        }
        ",
        20.into()
    );
}

#[test]
pub fn method_name_conflict_two_types() {
    // The heart of the feature: `Point.len` and `Str.len` mangle to distinct
    // symbols (`Point#len` vs `Str#len`) and each resolves by the receiver's
    // static type. If they collided the combined value would be wrong.
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        struct Str { chars: int }
        impl Point { fn len(self) { return self.x + self.y; } }
        impl Str { fn len(self) { return self.chars * 10; } }
        fn main() {
            let p = Point { x: 2, y: 3 };
            let s = Str { chars: 4 };
            print(p.len() * 1000 + s.len());
        }
        ",
        5040.into()
    );
}

#[test]
pub fn method_and_free_fn_coexist() {
    // A method named `area` and a free function named `area` live in separate
    // namespaces and both resolve.
    run_and_check_registers!(
        "
        struct Rect { w: int, h: int }
        fn area(a, b) { return a * b; }
        impl Rect { fn area(self) { return self.w * self.h; } }
        fn main() {
            let r = Rect { w: 3, h: 4 };
            print(area(2, 5) + r.area());
        }
        ",
        22.into()
    );
}

#[test]
pub fn method_chaining() {
    // Each result's static type drives the next resolution left-to-right:
    // inc() returns Counter, get() returns int.
    run_and_check_registers!(
        "
        struct Counter { n: int }
        impl Counter {
            fn inc(self) { return Counter { n: self.n + 1 }; }
            fn get(self) { return self.n; }
        }
        fn main() {
            let c = Counter { n: 0 };
            print(c.inc().inc().inc().get());
        }
        ",
        3.into()
    );
}

/// A method that calls itself through `self` is a recursive function, and is
/// compiled as one. Whether a body can reach itself is read from the calls it
/// makes, and a call on `self` names the method the receiver's own type
/// resolves it to.
#[test]
pub fn a_method_calls_itself() {
    run_and_check_registers!(
        "
        struct Box { n: int }
        impl Box {
            fn down(self, n) {
                if n <= 0 { return 0; }
                return n + self.down(n - 1);
            }
        }
        fn main() {
            print(Box { n: 1 }.down(4));
        }
        ",
        10.into()
    );
}

/// Two methods of one type that call each other are recursive together, the
/// way two free functions are.
#[test]
pub fn two_methods_call_each_other() {
    run_and_check_registers!(
        "
        struct Ping { n: int }
        impl Ping {
            fn even(self, n) {
                if n <= 0 { return 1; }
                return self.odd(n - 1);
            }
            fn odd(self, n) {
                if n <= 0 { return 2; }
                return self.even(n - 1);
            }
        }
        fn main() {
            print(Ping { n: 0 }.even(3));
        }
        ",
        2.into()
    );
}

/// A method of an instantiated generic type calls itself the same way: the
/// method is lowered against the instantiation, and the call on `self` names
/// that instantiation's method.
#[test]
pub fn a_generic_types_method_calls_itself() {
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        impl Cell<T> {
            fn sum(self, n) {
                if n <= 0 { return 0; }
                return n + self.sum(n - 1);
            }
        }
        fn main() {
            print(Cell<int> { value: 1 }.sum(4));
        }
        ",
        10.into()
    );
}

/// An enum's method recurses through `self` too, dispatching the same way a
/// struct's does.
#[test]
pub fn an_enums_method_calls_itself() {
    run_and_check_registers!(
        "
        enum Tag { A, B }
        impl Tag {
            fn count(self, n) {
                if n <= 0 { return 0; }
                return 1 + self.count(n - 1);
            }
        }
        fn main() {
            print(Tag::A.count(3));
        }
        ",
        3.into()
    );
}

#[test]
pub fn method_vs_field_disambiguation() {
    // `b.val` (field access, no parens) and `b.val()` (method call, parens) name
    // the same identifier but are distinguished purely by the call form.
    run_and_check_registers!(
        "
        struct Boxed { val: int }
        impl Boxed { fn val(self) { return self.val * 2; } }
        fn main() {
            let b = Boxed { val: 10 };
            print(b.val + b.val());
        }
        ",
        30.into()
    );
}

#[test]
pub fn method_alongside_builtin_len() {
    // A user struct method named `len` does not disturb the builtin string `len`.
    run_and_check_registers!(
        "
        struct Bag { count: int }
        impl Bag { fn len(self) { return self.count; } }
        fn main() {
            let bag = Bag { count: 7 };
            print(bag.len() + \"hello\".len());
        }
        ",
        12.into()
    );
}

#[test]
pub fn method_error_unknown() {
    let src = "
        struct P { x: int }
        fn main() { let p = P { x: 1 }; print(p.foo()); }
    ";
    let d = compile_diag(src, "m.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "no_such_method");
    assert!(
        d.message.contains("foo") && d.message.contains('P'),
        "message: {:?}",
        d.message
    );
}

#[test]
pub fn method_error_duplicate() {
    let src = "
        struct P { x: int }
        impl P { fn f(self) { return 1; } fn f(self) { return 2; } }
        fn main() { let p = P { x: 1 }; print(p.f()); }
    ";
    let d = compile_diag(src, "m.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "function_already_defined");
}

#[test]
pub fn method_on_builtin_string() {
    // An `impl string` block adds methods to string receivers, resolved by the
    // builtin type name (`string#shout`).
    run_and_check_registers!(
        "
        impl string {
            fn shout_len(self) { return (self.uppercase() + \"!\").len(); }
        }
        fn main() {
            print(\"hey\".shout_len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn method_on_builtin_list() {
    run_and_check_registers!(
        "
        impl list {
            fn second(self) { return self[1]; }
        }
        fn main() {
            print([7, 8, 9].second());
        }
        ",
        8.into()
    );
}

#[test]
pub fn method_on_builtin_map() {
    run_and_check_registers!(
        "
        impl map {
            fn get_or(self, k, default) {
                if self.contains(k) {
                    return self.get(k);
                }
                return default;
            }
        }
        fn main() {
            let m = {};
            m.insert(\"a\", 5);
            print(m.get_or(\"a\", 0) + m.get_or(\"b\", 30));
        }
        ",
        35.into()
    );
}

#[test]
pub fn method_on_builtin_int() {
    run_and_check_registers!(
        "
        impl int {
            fn doubled(self) { return self * 2; }
        }
        fn main() {
            let n = 21;
            print(n.doubled());
        }
        ",
        42.into()
    );
}

#[test]
pub fn method_on_builtin_float() {
    run_and_check_registers!(
        "
        impl float {
            fn halved(self) { return self / 2.0; }
        }
        fn main() {
            let f = 5.0;
            print(f.halved());
        }
        ",
        2.5.into()
    );
}

#[test]
pub fn method_on_builtin_with_type_parameter() {
    // A method on a builtin type takes type parameters on the same terms as a
    // struct method; the explicit type argument binds `U` at the call site.
    run_and_check_registers!(
        "
        impl list {
            fn tagged<U>(self, extra: U) -> U { return extra; }
        }
        fn main() {
            print([1, 2].tagged<int>(9) + [1, 2].tagged<string>(\"hi\").len());
        }
        ",
        11.into()
    );
}

#[test]
pub fn method_on_builtin_cannot_shadow_builtin() {
    // The builtin method table keeps precedence: an `impl list` method named
    // `len` never resolves; `[..].len()` stays the builtin length.
    run_and_check_registers!(
        "
        impl list {
            fn len(self) { return 999; }
        }
        fn main() {
            print([1, 2, 3].len());
        }
        ",
        3.into()
    );
}

#[test]
pub fn user_fn_named_like_a_file_builtin_is_typed_as_itself() {
    // `read` names the file library only through the `fs` path, so a program's
    // own `read` is the one every call site means. Inference used to answer
    // `string` off the bare name while the call compiled to the user function,
    // and the addition below then failed to type-check.
    run_and_check_registers!(
        "
        fn read(n: int) -> int { return n + 1; }

        fn main() {
            print(1 + read(1));
        }
        ",
        3.into()
    );
}

#[test]
pub fn user_fn_named_exists_is_typed_as_itself() {
    // Same seam as `read`, for the name whose file-library type is `bool`.
    run_and_check_registers!(
        "
        fn exists(n: int) -> int { return n * 2; }

        fn main() {
            print(1 + exists(3));
        }
        ",
        7.into()
    );
}

#[test]
pub fn user_fn_named_str_shadows_the_builtin() {
    // A function in scope wins over a built-in of the same name, in the type
    // checker and in lowering alike: `str` is the program's `str`, so the sum
    // is arithmetic rather than a string conversion.
    run_and_check_registers!(
        "
        fn str(n: int) -> int { return n * 2; }

        fn main() {
            print(1 + str(3));
        }
        ",
        7.into()
    );
}

#[test]
pub fn builtins_still_infer_when_nothing_shadows_them() {
    // The built-in table answers whenever the scope holds no function of the
    // name, so the conversions keep their own types.
    run_and_check_registers!(
        r#"
        fn main() {
            print(type(str(3)) + type(int("4")) + type(bool("true")) == "stringintbool");
        }
        "#,
        true.into()
    );
}

#[test]
pub fn fs_return_types_stay_on_the_fs_path() {
    // A user `read` and `fs::read` coexist: the path picks the family, so each
    // call gets the type of the function it names.
    run_and_check_registers!(
        r#"
        fn read(n: int) -> int { return n; }

        fn main() {
            print(type(read(1)) + type(fs::read("p")) == "intstring");
        }
        "#,
        true.into()
    );
}

#[test]
pub fn hof_named_function_reference() {
    run_and_check_registers!(
        "
        fn double(x) { return x * 2; }
        fn apply(f, n) { return f(n); }
        fn main() { print(apply(double, 21)); }
        ",
        42.into()
    );
}

#[test]
pub fn hof_anonymous_function() {
    run_and_check_registers!(
        "
        fn apply(f, n) { return f(n); }
        fn main() { print(apply(fn(x) { return x * x; }, 9)); }
        ",
        81.into()
    );
}

/// A closure that takes nothing is the shape of a thunk, and it is written
/// and called like any other.
#[test]
pub fn zero_parameter_closure_through_a_variable() {
    run_and_check_registers!(
        "
        fn main() {
            let answer = fn() { return 42; };
            print(answer());
        }
        ",
        42.into()
    );
}

/// A closure is an ordinary value, so the one a call hands back is called
/// where it stands rather than through a `let`.
#[test]
pub fn call_the_value_a_call_returns() {
    run_and_check_registers!(
        "
        fn pick() { return fn(x) { return x * 2; }; }
        fn main() { print(pick()(21)); }
        ",
        42.into()
    );
}

/// The same for an element of a list of closures.
#[test]
pub fn call_an_element_of_a_list_of_functions() {
    run_and_check_registers!(
        "
        fn main() {
            let fs = [fn(x) { return x + 1; }];
            print(fs[0](41));
        }
        ",
        42.into()
    );
}

/// The same closure handed to a higher-order function, which calls it with no
/// arguments the way it received it.
#[test]
pub fn zero_parameter_closure_through_a_hof() {
    run_and_check_registers!(
        "
        fn twice(f) { return f() + f(); }
        fn main() { print(twice(fn() { return 21; })); }
        ",
        42.into()
    );
}

/// And for the two chained: a call, an index into what it returned, then a
/// call of the element.
#[test]
pub fn call_an_element_of_the_list_a_call_returns() {
    run_and_check_registers!(
        "
        fn make() { return [fn(x) { return x * 2; }]; }
        fn main() { print(make()[0](21)); }
        ",
        42.into()
    );
}

#[test]
pub fn hof_specialization_is_per_function() {
    // The same higher-order function called with two different functions must
    // specialize separately: 20 + 30 = 50 (not 40, which a collapsed
    // specialization would produce by reusing the first function).
    run_and_check_registers!(
        "
        fn double(x) { return x * 2; }
        fn triple(x) { return x * 3; }
        fn apply(f, n) { return f(n); }
        fn main() { print(apply(double, 10) + apply(triple, 10)); }
        ",
        50.into()
    );
}

#[test]
pub fn hof_map_over_array() {
    run_and_check_registers!(
        "
        fn inc(x) { return x + 1; }
        fn map(arr, f) { let o = []; for x in arr { o.push(f(x)); } return o; }
        fn sum(arr) { let s = 0; for x in arr { s = s + x; } return s; }
        fn main() { print(sum(map([1, 2, 3], inc))); }
        ",
        9.into()
    );
}

// ---- native enums ----

#[test]
pub fn enum_nullary_match() {
    run_and_check_registers!(
        "
        enum Color { Red, Green, Blue }
        fn main() {
            let c = Color::Green;
            let n = 0;
            match c {
                Red => { n = 1; }
                Green => { n = 2; }
                Blue => { n = 3; }
            }
            print(n);
        }
        ",
        2.into()
    );
}

#[test]
pub fn enum_payload_binding() {
    run_and_check_registers!(
        "
        enum Shape { Circle(int), Rect(int, int), Unit }
        fn main() {
            let s = Shape::Rect(3, 4);
            let a = 0;
            match s {
                Circle(r) => { a = r; }
                Rect(w, h) => { a = w * h; }
                Unit => { a = -1; }
            }
            print(a);
        }
        ",
        12.into()
    );
}

#[test]
pub fn enum_match_wildcard() {
    run_and_check_registers!(
        "
        enum Dir { North, South, East, West }
        fn main() {
            let d = Dir::West;
            let n = 0;
            match d {
                North => { n = 1; }
                _ => { n = 9; }
            }
            print(n);
        }
        ",
        9.into()
    );
}

/// The compiler's debug register dump formats every register before the
/// program runs, and an enum construction's destination register is only a
/// placeholder until `CloneEnum` writes it. The placeholder used to name
/// object-pool slot 0, so the dump read the first word of whatever object the
/// program built first as this enum's variant tag: an array of ints gave a tag
/// past the last variant, and an array of enums gave a word that is no tag at
/// all. Both cases dump here, because `run_and_check_registers!` compiles with
/// the dump on.
#[test]
pub fn enum_in_an_array_dumps_its_registers() {
    run_and_check_registers!(
        "
        enum Value { Int(int), Null }
        fn main() {
            let xs = [Value::Int(3)];
            match xs[0] {
                Int(n) => { print(n); }
                Null => { print(-1); }
            }
        }
        ",
        3.into()
    );
}

/// The same dump, with an array of plain ints built before the enum, which is
/// what put a tag past the last variant in front of the unchecked variant
/// lookup.
#[test]
pub fn enum_after_an_int_array_dumps_its_registers() {
    run_and_check_registers!(
        "
        enum Value { Int(int), Null }
        fn main() {
            let seen = [42];
            let v = Value::Int(seen[0]);
            match v {
                Int(n) => { print(n); }
                Null => { print(-1); }
            }
        }
        ",
        42.into()
    );
}

/// Every register of `src`, formatted the way the debug dump formats it. A
/// construction's destination register is a placeholder until the instruction
/// that fills it runs, so this is what the dump prints for a register before
/// the program has written it.
fn dumped_registers(src: &str, filename: &str) -> Vec<String> {
    let out = compile(
        String::from(src),
        filename,
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    out.registers
        .iter()
        .map(|data| {
            data.format(
                &out.pools.objs,
                &out.pools.strings,
                &out.pools.maps,
                &out.structs,
                &out.enums,
                true,
            )
            .to_string()
        })
        .collect()
}

/// A program that builds an array, a struct and a map inside a function called
/// twice, after a program-first array and a program-first map in `main`. The
/// three constructions in the function are the ones that go through a template
/// register and a placeholder destination.
const OBJECTS_AFTER_OTHER_OBJECTS: &str = "
        struct Point { x: int, y: int }
        fn build(n) {
            let xs = [10, 20];
            let p = Point{ x: n, y: 2 };
            let m = { \"k\": 7 };
            return xs.len() + p.x + m.get(\"k\");
        }
        fn main() {
            let first = [\"alpha\", \"beta\"];
            let earlier = { \"zz\": 99 };
            print(first.len() + earlier.get(\"zz\") + build(1) + build(2));
        }
        ";

/// What the dump prints for an enum construction's destination register: the
/// variant the register is about to hold, not the fallback `Data::format`
/// gives an entry that carries no readable variant tag.
#[test]
pub fn an_enum_destination_register_dumps_its_variant() {
    let dumped = dumped_registers(
        "
        enum Value { Int(int), Null }
        fn main() {
            let xs = [Value::Int(3)];
            print(7);
        }
        ",
        "enum_dump.cdl",
    );
    assert!(dumped.iter().any(|s| s == "Int(3)"), "{dumped:?}");
    assert!(!dumped.iter().any(|s| s == "enum"), "{dumped:?}");
}

/// An array literal's destination register is a placeholder until `CloneArray`
/// or `EmptyArray` fills it. The placeholder used to name object-pool slot 0,
/// which belongs to whatever object the program built first, so the dump
/// showed that object's elements in a register about to hold this array. The
/// template and the destination both name this literal's entry now, and the
/// first array is named by the one register that holds it.
#[test]
pub fn an_array_destination_register_dumps_its_own_elements() {
    let dumped = dumped_registers(OBJECTS_AFTER_OTHER_OBJECTS, "array_dump.cdl");
    assert_eq!(
        dumped.iter().filter(|s| s.as_str() == "[10,20]").count(),
        2,
        "{dumped:?}"
    );
    assert_eq!(
        dumped
            .iter()
            .filter(|s| s.as_str() == "[\"alpha\",\"beta\"]")
            .count(),
        1,
        "{dumped:?}"
    );
}

/// The same for a struct literal, whose destination register `CloneStruct`
/// fills. A field the literal sets from a value the compiler does not have is
/// null in the template, which is what the register is about to hold before
/// `SetFieldStruct` writes it.
#[test]
pub fn a_struct_destination_register_dumps_its_own_fields() {
    let dumped = dumped_registers(OBJECTS_AFTER_OTHER_OBJECTS, "struct_dump.cdl");
    assert_eq!(
        dumped
            .iter()
            .filter(|s| s.as_str() == "Point {x:null,y:2}")
            .count(),
        2,
        "{dumped:?}"
    );
}

/// The same for a map literal, whose destination register `CloneMap` fills.
/// The placeholder named map-pool slot 0, so the dump showed the first map the
/// program built.
#[test]
pub fn a_map_destination_register_dumps_its_own_entries() {
    let dumped = dumped_registers(OBJECTS_AFTER_OTHER_OBJECTS, "map_dump.cdl");
    assert_eq!(
        dumped.iter().filter(|s| s.as_str() == "{\"k\":7}").count(),
        2,
        "{dumped:?}"
    );
    assert_eq!(
        dumped
            .iter()
            .filter(|s| s.as_str() == "{\"zz\":99}")
            .count(),
        1,
        "{dumped:?}"
    );
}

/// The array collector's root scan walks the whole register file and follows
/// every register whose tag is an object, so a placeholder aimed at slot 0 kept
/// the program's first object alive for as long as its register went unwritten.
/// Reading the register file the same way, slot 0 of each pool is named by the
/// single register that holds the object it belongs to. The scan itself is
/// internal to the runtime, so this asserts the root set the compiler hands it
/// rather than a collection.
#[test]
pub fn pool_slot_zero_is_named_by_one_register() {
    let out = compile(
        String::from(OBJECTS_AFTER_OTHER_OBJECTS),
        "slot_zero.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    let objects = out
        .registers
        .iter()
        .filter(|d| (d.is_array() || d.is_struct() || d.is_enum()) && d.as_array() == 0)
        .count();
    let maps = out
        .registers
        .iter()
        .filter(|d| d.is_map() && d.as_map() == 0)
        .count();
    assert_eq!(objects, 1, "object-pool slot 0 has {objects} roots");
    assert_eq!(maps, 1, "map-pool slot 0 has {maps} roots");
}

/// The same program runs to the right answer with the dump on, which is how
/// `run_and_check_registers!` compiles: 2 + 99 + (2 + 1 + 7) + (2 + 2 + 7).
#[test]
pub fn objects_built_after_other_objects_run() {
    run_and_check_registers!(OBJECTS_AFTER_OTHER_OBJECTS, 122.into());
}

/// A bare parameter takes the type its call site passes, so a call that hands
/// it an enum value gets the variant-matching lowering with its payload bound.
#[test]
pub fn enum_match_through_an_untyped_parameter() {
    run_and_check_registers!(
        "
        enum Value { Int(int), Null }
        fn pick(v) {
            let rc = 0;
            match v {
                Int(n) => { rc = n; }
                Null => { rc = -1; }
            }
            return rc;
        }
        fn main() {
            print(pick(Value::Int(3)));
        }
        ",
        3.into()
    );
}

/// An empty array literal has no element type, so an element read from one is
/// null and the arms have no enum to dispatch on. The equality lowering used to
/// take this silently and compile `Int(n)` as a variant construction, which
/// left `n` unbound in the arm body.
#[test]
pub fn variant_patterns_need_an_enum_scrutinee() {
    let src = "enum Value { Int(int), Null }
fn pick(v) {
    let rc = 0;
    match v { Int(n) => { rc = n; } Null => { rc = -1; } }
    return rc;
}
fn main() { let xs = []; print(pick(xs[0])); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "match_not_enum");
    assert!(d.message.contains("variants of enum Value"));
    assert!(d.message.contains("the matched value is null"));
}

/// A qualified arm pattern used to be reduced to its last segment and looked
/// up in the scrutinee's enum, so a variant name two enums share matched
/// whichever enum the pattern named. The qualifier resolves now, and an enum
/// that is not the scrutinee's is reported.
#[test]
pub fn a_pattern_qualified_with_another_enum_is_reported() {
    let src = "enum Shape { Circle(int), Empty }
enum Other { Circle(int) }
fn main() {
    let s = Shape::Circle(5);
    match s { Other::Circle(r) => { print(r); } Empty => { print(0); } }
}";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "pattern_enum_mismatch");
    assert!(d.message.contains("variant of enum Other"), "{d:?}");
    assert!(d.message.contains("this match is on Shape"), "{d:?}");
}

/// The same check across a module alias. A path through the wrong alias reaches
/// a different enum, which the reduction to the last segment hid as well.
#[test]
pub fn a_pattern_qualified_with_a_wrong_alias_is_reported() {
    let dir = std::env::temp_dir().join("candela_pattern_alias_test");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(
        dir.join("shapes.cdl"),
        "enum Shape { Circle(int), Empty }\n",
    )
    .unwrap();
    std::fs::write(dir.join("others.cdl"), "enum Other { Circle(int) }\n").unwrap();
    let main = dir.join("wrong_alias.cdl");
    let src = "import \"./shapes.cdl\" as sh;
import \"./others.cdl\" as ot;
fn main() {
    let s = sh::Shape::Circle(5);
    match s { ot::Other::Circle(r) => { print(r); } Empty => { print(0); } }
}";
    std::fs::write(&main, src).unwrap();
    let filename = main.to_str().unwrap().to_owned();
    let d = collect_diagnostic(|| {
        let _ = compile(
            String::from(src),
            &filename,
            false,
            &crate::compiler::imports::ImportResolver::new(),
        );
    })
    .unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "pattern_enum_mismatch");
    assert!(d.message.contains("variant of enum Other"), "{d:?}");
    assert!(d.message.contains("this match is on Shape"), "{d:?}");
}

/// A generic enum's arms resolve through the instantiation the pattern names,
/// so a pattern written at another type argument is a different enum and is
/// reported the same way.
#[test]
pub fn a_pattern_at_another_instantiation_is_reported() {
    let src = "enum Slot<T> { Full(T), Empty }
fn main() {
    let s = Slot<int>::Full(7);
    match s { Slot<string>::Full(v) => { print(v); } Empty => { print(0); } }
}";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "pattern_enum_mismatch");
    assert!(d.message.contains("enum Slot<string>"), "{d:?}");
    assert!(d.message.contains("this match is on Slot<int>"), "{d:?}");
}

/// Every qualified form that does name the scrutinee's enum still matches: the
/// enum name on its own, and a generic enum written with the instantiation the
/// value has.
#[test]
pub fn a_pattern_qualified_with_the_scrutinee_enum_matches() {
    run_and_check_registers!(
        "
        enum Shape { Circle(int), Empty }
        enum Slot<T> { Full(T), Empty }
        fn main() {
            let s = Shape::Circle(5);
            let total = 0;
            match s {
                Shape::Circle(r) => { total = total + r; }
                Shape::Empty => { total = total - 1; }
            }
            let slot = Slot<int>::Full(4);
            match slot {
                Slot<int>::Full(v) => { total = total + v; }
                Empty => { total = total - 1; }
            }
            print(total);
        }
        ",
        9.into()
    );
}

/// A declared array parameter pins the element type an empty literal leaves
/// open, so the body compiles at that element type and can match on its
/// elements. Both calls land on the one specialisation the declaration fixes.
#[test]
pub fn a_declared_array_parameter_pins_an_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn total(xs: Value[]) -> int {
            let sum = 0;
            let i = 0;
            while i < xs.len() {
                match xs[i] {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
                i = i + 1;
            }
            return sum;
        }
        fn main() {
            print(total([]) + total([Value::Num(4), Value::Text(\"ab\")]));
        }
        ",
        6.into()
    );
}

/// The pinning reaches an empty literal nested inside the argument: an outer
/// array of arrays fills in the inner element type from the declaration too.
#[test]
pub fn a_declared_array_parameter_pins_a_nested_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn total(xs: Value[]) -> int {
            let sum = 0;
            let i = 0;
            while i < xs.len() {
                match xs[i] {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
                i = i + 1;
            }
            return sum;
        }
        fn deep(rows: Value[][]) -> int {
            let sum = 0;
            let i = 0;
            while i < rows.len() {
                sum = sum + total(rows[i]);
                i = i + 1;
            }
            return sum;
        }
        fn main() {
            print(deep([[], [Value::Num(7)]]) + deep([]));
        }
        ",
        7.into()
    );
}

/// A declared `-> T[]` says what a `return []` hands back, so the call site
/// keeps the element type. The consumer here takes a bare parameter, which
/// specialises on whatever the call passes: it can only match on variants if
/// the declared return type reached it.
#[test]
pub fn a_declared_array_return_pins_an_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn empty() -> Value[] {
            return [];
        }
        fn total(xs) {
            let sum = 0;
            let i = 0;
            while i < xs.len() {
                match xs[i] {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
                i = i + 1;
            }
            return sum;
        }
        fn main() {
            print(total(empty()) + 5);
        }
        ",
        5.into()
    );
}

/// A generic `-> T[]` pins the same way once the call names its type argument:
/// the annotation is resolved with `T` bound, so `empty<Value>()` hands the
/// consumer elements of `Value` rather than elements of no type.
#[test]
pub fn a_generic_array_return_pins_an_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn empty<T>() -> T[] {
            return [];
        }
        fn total(xs) {
            let sum = 0;
            let i = 0;
            while i < xs.len() {
                match xs[i] {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
                i = i + 1;
            }
            return sum;
        }
        fn main() {
            print(total(empty<Value>()) + 5);
        }
        ",
        5.into()
    );
}

/// Only an open element type is filled in. A literal that does name its element
/// type is still checked against the declaration, so the wrong one is refused.
#[test]
pub fn a_wrong_array_element_type_is_still_refused() {
    let src = "enum Value { Num(int), Text(string) }
fn total(xs: Value[]) -> int { return xs.len(); }
fn main() { print(total([1, 2, 3])); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
}

/// A declared map parameter pins the value type an empty literal leaves open,
/// so the body compiles at that value type and can match on what a key holds.
/// The empty call comes first, which is the order that compiles the open
/// specialisation.
#[test]
pub fn a_declared_map_parameter_pins_an_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn total(m: {string: Value}) -> int {
            let sum = 0;
            for k in m {
                match m.get(k) {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
            }
            return sum;
        }
        fn main() {
            print(total({}) + total({\"a\": Value::Num(4), \"b\": Value::Text(\"ab\")}));
        }
        ",
        6.into()
    );
}

/// The key half of a map's type is pinned as well, so a body that matches on
/// the keys of a declared `{Key: int}` compiles when the literal is empty. Map
/// keys have to be literals, so an enum-keyed map is only ever written empty.
#[test]
pub fn a_declared_map_parameter_pins_an_empty_literal_key() {
    run_and_check_registers!(
        "
        enum Key { Num(int), Text(string) }
        fn total(m: {Key: int}) -> int {
            let sum = 7;
            for k in m {
                match k {
                    Key::Num(n) => { sum = sum + n; }
                    Key::Text(t) => { sum = sum + t.len(); }
                }
            }
            return sum;
        }
        fn main() {
            print(total({}));
        }
        ",
        7.into()
    );
}

/// The pinning reaches an empty map nested inside the argument: an array of
/// maps fills in the inner value type from the declaration too.
#[test]
pub fn a_declared_map_parameter_pins_a_nested_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn total(m: {string: Value}) -> int {
            let sum = 0;
            for k in m {
                match m.get(k) {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
            }
            return sum;
        }
        fn deep(rows: {string: Value}[]) -> int {
            let sum = 0;
            let i = 0;
            while i < rows.len() {
                sum = sum + total(rows[i]);
                i = i + 1;
            }
            return sum;
        }
        fn main() {
            print(deep([{}, {\"a\": Value::Num(7)}]));
        }
        ",
        7.into()
    );
}

/// A declared `-> {K: V}` says what a `return {}` hands back, so the call site
/// keeps the value type. The consumer takes a bare parameter, which specialises
/// on whatever the call passes: it can only match on variants if the declared
/// return type reached it.
#[test]
pub fn a_declared_map_return_pins_an_empty_literal() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn empty() -> {string: Value} {
            return {};
        }
        fn total(m) {
            let sum = 5;
            for k in m {
                match m.get(k) {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
            }
            return sum;
        }
        fn main() {
            print(total(empty()));
        }
        ",
        5.into()
    );
}

/// Only an open position is filled. A map literal that does name its value type
/// is still checked against the declaration, so the wrong one is refused.
#[test]
pub fn a_wrong_map_value_type_is_still_refused() {
    let src = "enum Value { Num(int), Text(string) }
fn total(m: {string: Value}) -> int { return m.len(); }
fn main() { print(total({\"a\": 1})); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
}

/// A `let` takes no annotation, so an empty map literal holds nothing until
/// something says what it holds. Handing it to a parameter that declares its
/// types is such a statement: the binding takes the declared types, so what the
/// caller reads back out of the map has the value type the callee put in it.
#[test]
pub fn a_typed_parameter_pins_an_empty_map_binding() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn fill(m: {string: P}) {
            m.insert(\"a\", P { x: 1 });
        }
        fn main() {
            let m = {};
            fill(m);
            print(m.get(\"a\").x);
        }
        ",
        1.into()
    );
}

/// The same for an empty array literal: the declared element type reaches the
/// caller's binding, so indexing it yields an element of that type.
#[test]
pub fn a_typed_parameter_pins_an_empty_array_binding() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn fill(xs: P[]) {
            xs.push(P { x: 2 });
        }
        fn main() {
            let xs = [];
            fill(xs);
            print(xs[0].x);
        }
        ",
        2.into()
    );
}

/// The declared type is taken whole, so a binding pinned from a nested
/// declaration reads back through both levels.
#[test]
pub fn a_typed_parameter_pins_a_nested_empty_map_binding() {
    run_and_check_registers!(
        "
        fn fill(m: {string: int[]}) {
            m.insert(\"a\", [7]);
        }
        fn main() {
            let m = {};
            fill(m);
            print(m.get(\"a\")[0]);
        }
        ",
        7.into()
    );
}

/// Two parameters that declare the same type pin the binding to it once. The
/// second call passes a binding that already names its element type and is
/// checked against the declaration like any other argument.
#[test]
pub fn two_agreeing_parameters_pin_one_empty_binding() {
    run_and_check_registers!(
        "
        fn fill(xs: int[]) {
            xs.push(1);
        }
        fn add(xs: int[]) {
            xs.push(2);
        }
        fn main() {
            let xs = [];
            fill(xs);
            add(xs);
            print(xs[0] + xs[1]);
        }
        ",
        3.into()
    );
}

/// The first call pins the binding, so a later call that declares another type
/// is the ordinary argument mismatch, reported where that call passes it.
#[test]
pub fn a_second_disagreeing_parameter_is_reported() {
    let src = "fn ints(m: {string: int}) { m.insert(\"a\", 1); }
fn texts(m: {string: string}) { m.insert(\"b\", \"x\"); }
fn main() {
    let m = {};
    ints(m);
    texts(m);
}";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
    let second_call = src.find("texts(m);").unwrap();
    assert!(
        d.span.start >= second_call,
        "reported before the call: {d:?}"
    );
}

/// The pin outlives the call that made it: a match written after it dispatches
/// on the enum the parameter declared, and its payload binder has a type.
#[test]
pub fn a_pinned_binding_matches_on_an_enum_payload() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        fn fill(m: {string: Value}) {
            m.insert(\"a\", Value::Text(\"abcd\"));
        }
        fn main() {
            let m = {};
            fill(m);
            let sum = 0;
            match m.get(\"a\") {
                Value::Num(n) => { sum = n; }
                Value::Text(t) => { sum = t.len(); }
            }
            print(sum);
        }
        ",
        4.into()
    );
}

/// A diagnostic names a user type the way the program declared it. A
/// `DataType` carries only an enum's id, so `Display` prints the bare word
/// `enum` and a program with two enums reports the same word for both; the
/// message has to read `Value[]`.
#[test]
pub fn an_argument_type_mismatch_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
enum Other { A }
fn total(xs: Value[]) -> int { return xs.len(); }
fn main() { print(total([1, 2, 3])); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
    assert!(d.message.contains("Value[]"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// The return-type mismatch names it too, in the message and in the note that
/// repeats what the function is declared to return.
#[test]
pub fn a_return_type_mismatch_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn make() -> Value[] { return [1]; }
fn main() { print(make().len()); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_type");
    assert!(d.message.contains("Value[]"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// So does the field-type mismatch, for the declared type and for what was
/// given.
#[test]
pub fn a_struct_field_type_mismatch_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
struct Holder { rows: Value[] }
fn main() { print(Holder{ rows: [1] }.rows.len()); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "struct_field_type_mismatch");
    assert!(d.message.contains("Value[]"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// The operator error names it in the message and on both operand labels.
#[test]
pub fn an_operator_error_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn main() { print(Value::Num(1) + 1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert!(d.message.contains("Value"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// So does the error for a type that cannot be indexed.
#[test]
pub fn an_unindexable_type_error_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn main() { let v = Value::Num(1); print(v[0]); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "type_not_indexable");
    assert!(d.message.contains("Value"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// Iterating one is the same message with a different verb, so it names the
/// type too.
#[test]
pub fn a_non_iterable_type_error_names_the_struct() {
    let src = "struct Point { x: int }
fn main() { let p = Point{ x: 1 }; for q in p { print(q); } }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "type_not_iterable");
    assert!(d.message.contains("Point"), "{}", d.message);
    assert!(!d.message.contains("struct"), "{}", d.message);
}

/// A collection literal reports what it already holds against the element that
/// does not fit, and both are named.
#[test]
pub fn an_array_literal_type_error_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn main() { print([Value::Num(1), 2]); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "array_element_type_mismatch");
    assert!(d.message.contains("Value"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

#[test]
pub fn a_map_literal_type_error_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn main() { print({\"a\": Value::Num(1), \"b\": 2}); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "map_entry_type_mismatch");
    assert!(d.message.contains("Value"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// Writing through an index names the list's own type, not the word `enum[]`.
#[test]
pub fn an_index_assignment_type_error_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn main() { let xs = [Value::Num(1)]; xs[0] = 2; print(xs.len()); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "cannot_push_type_to_array");
    assert!(d.message.contains("Value[]"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// A built-in's argument check reports the list of types it accepts. For a
/// method on a list of a user type that list is the user type, so it is named
/// on the expected side as well as the received one.
#[test]
pub fn a_builtin_argument_error_names_the_enum() {
    let src = "enum Value { Num(int), Text(string) }
fn main() { let xs = [Value::Num(1)]; xs.push(2); print(xs.len()); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
    assert!(d.message.contains("Value"), "{}", d.message);
    assert!(!d.message.contains("enum"), "{}", d.message);
}

/// The match-on-a-non-enum report names what was matched, so a struct
/// scrutinee reads `Point` rather than the word `struct`.
#[test]
pub fn a_match_on_a_non_enum_names_the_struct() {
    let src = "enum Value { Num(int), Text(string) }
struct Point { x: int }
fn main() {
    let p = Point{ x: 1 };
    match p { Value::Num(n) => { print(n); } _ => { print(0); } }
}";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "match_not_enum");
    assert!(d.message.contains("Point"), "{}", d.message);
    // The enum the arms name is still called out; the scrutinee is no longer
    // the bare word `struct`.
    assert!(!d.message.contains("struct"), "{}", d.message);
}

#[test]
pub fn a_range_type_error_names_the_struct() {
    let src = "struct Point { x: int }
fn main() { let p = Point{ x: 1 }; for i in p..3 { print(i); } }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "range_element_type_mismatch");
    assert!(d.message.contains("Point"), "{}", d.message);
    assert!(!d.message.contains("struct"), "{}", d.message);
}

/// A struct field needs no pinning: reading a field takes the type the struct
/// declares for it, not the type of the value that was stored, so an empty map
/// in a `{string: Value}` field is read back as holding `Value`. Arrays behave
/// the same way, which is why neither field case goes through the argument
/// pinning.
#[test]
pub fn a_struct_field_declares_what_an_empty_literal_holds() {
    run_and_check_registers!(
        "
        enum Value { Num(int), Text(string) }
        struct Holder { rows: {string: Value}, names: Value[] }
        fn total(h: Holder) -> int {
            let sum = 3;
            for k in h.rows {
                match h.rows.get(k) {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
            }
            let i = 0;
            while i < h.names.len() {
                match h.names[i] {
                    Value::Num(n) => { sum = sum + n; }
                    Value::Text(t) => { sum = sum + t.len(); }
                }
                i = i + 1;
            }
            return sum;
        }
        fn main() {
            print(total(Holder{ rows: {}, names: [] }));
        }
        ",
        3.into()
    );
}

/// Literal arms stay an equality chain whatever the scrutinee's type is, so a
/// match on a value the compiler cannot type still compiles and runs.
#[test]
pub fn literal_arms_match_an_untyped_value() {
    run_and_check_registers!(
        "
        fn pick(v) {
            let rc = 0;
            match v {
                1 => { rc = 10; }
                2 => { rc = 20; }
                _ => { rc = -1; }
            }
            return rc;
        }
        fn main() {
            let xs = [];
            print(pick(2) + pick(xs.len()));
        }
        ",
        19.into()
    );
}

#[test]
pub fn enum_equality() {
    run_and_check_registers!(
        "
        enum Color { Red, Green, Blue }
        fn main() {
            let a = Color::Red;
            let b = Color::Red;
            print(a == b);
        }
        ",
        true.into()
    );
}

#[test]
pub fn enum_inequality_variants() {
    run_and_check_registers!(
        "
        enum Color { Red, Green, Blue }
        fn main() {
            let a = Color::Red;
            let b = Color::Blue;
            print(a == b);
        }
        ",
        false.into()
    );
}

#[test]
pub fn enum_payload_equality() {
    run_and_check_registers!(
        "
        enum Box { Val(int), Empty }
        fn main() {
            let a = Box::Val(7);
            let b = Box::Val(7);
            let c = Box::Val(8);
            print((a == b) && (a != c));
        }
        ",
        true.into()
    );
}

#[test]
pub fn enum_returned_from_fn() {
    run_and_check_registers!(
        "
        enum Opt { Some(int), None }
        fn wrap(x) { return Opt::Some(x); }
        fn main() {
            let o = wrap(42);
            let n = 0;
            match o {
                Some(v) => { n = v; }
                None => { n = -1; }
            }
            print(n);
        }
        ",
        42.into()
    );
}

/// An enum construction inside a loop keeps its object-pool template register
/// for the life of the program. The template is a constant register, so once a
/// caller's variable is allowed to reuse it, the second call to the function
/// clones whatever the caller left there instead of the variant.
#[test]
pub fn enum_built_in_a_loop_survives_a_second_call() {
    run_and_check_registers!(
        "
        enum Value { Int(int), Text(string) }
        fn build(n) {
            let cells = [];
            for i in range(n) { cells.push(Value::Int(i)); }
            return cells;
        }
        fn main() {
            let a = build(4);
            let b = build(4);
            let total = 0;
            for c in b {
                match c {
                    Int(v) => { total = total + v; }
                    Text(s) => { total = total - 1; }
                }
            }
            print(total);
        }
        ",
        6.into()
    );
}

#[test]
pub fn enum_any_payload() {
    run_and_check_registers!(
        "
        enum Any1 { Wrap(any), Nil }
        fn main() {
            let a = Any1::Wrap(\"hello\");
            let out = \"none\";
            match a {
                Wrap(v) => { out = v; }
                Nil => { out = \"nil\"; }
            }
            print(out);
        }
        ",
        crate::data::Data::small_str("hello")
    );
}

// ---------------------------------------------------------------------------
// ANY / DOWNCAST, JSON, AND MAP/SET PRIMITIVES
// ---------------------------------------------------------------------------

#[test]
pub fn any_downcast_int_arithmetic() {
    // A value pulled out of an `any` payload downcasts to a concrete int and is
    // then usable in arithmetic; the ergonomic gap the enum pass left open.
    run_and_check_registers!(
        "
        enum Box1 { Val(any) }
        fn main() {
            let b = Box1::Val(21);
            let n = 0;
            match b { Val(v) => { n = as_int(v) * 2; } }
            print(n);
        }
        ",
        42.into()
    );
}

#[test]
pub fn any_type_tests() {
    run_and_check_registers!(
        "
        fn main() {
            let j = json_parse(\"{\\\"n\\\": 5}\");
            print(is_map(j));
        }
        ",
        true.into()
    );
}

#[test]
pub fn any_is_int_true() {
    run_and_check_registers!(
        "
        fn main() {
            let v = json_parse(\"7\");
            print(is_int(v));
        }
        ",
        true.into()
    );
}

#[test]
pub fn any_is_int_false_on_string() {
    run_and_check_registers!(
        "
        fn main() {
            let v = json_parse(\"\\\"x\\\"\");
            print(is_int(v));
        }
        ",
        false.into()
    );
}

#[test]
pub fn any_bad_downcast_is_catchable() {
    // A downcast to the wrong type raises a catchable error rather than
    // producing a garbage value.
    run_and_check_registers!(
        "
        fn main() {
            let r = 0;
            try {
                let x = as_int(json_parse(\"\\\"str\\\"\"));
                r = 1;
            } catch e {
                r = 2;
            }
            print(r);
        }
        ",
        2.into()
    );
}

#[test]
pub fn any_as_str() {
    run_and_check_registers!(
        "
        fn main() {
            print(as_str(json_parse(\"\\\"hi\\\"\")));
        }
        ",
        crate::data::Data::small_str("hi")
    );
}

#[test]
pub fn any_as_bool() {
    run_and_check_registers!(
        "
        fn main() {
            print(as_bool(json_parse(\"true\")));
        }
        ",
        true.into()
    );
}

#[test]
pub fn json_parse_scalar_int() {
    run_and_check_registers!(
        "
        fn main() {
            print(as_int(json_parse(\"42\")) + 1);
        }
        ",
        43.into()
    );
}

#[test]
pub fn json_parse_object_field() {
    run_and_check_registers!(
        "
        fn main() {
            let obj = as_map(json_parse(\"{\\\"x\\\": 10, \\\"y\\\": 20}\"));
            print(as_int(obj.get(\"x\")) + as_int(obj.get(\"y\")));
        }
        ",
        30.into()
    );
}

#[test]
pub fn json_parse_nested_array() {
    run_and_check_registers!(
        "
        fn main() {
            let obj = as_map(json_parse(\"{\\\"nums\\\": [1, 2, 3, 4]}\"));
            let arr = as_list(obj.get(\"nums\"));
            print(arr.len());
        }
        ",
        4.into()
    );
}

/// Runs `contents` in the chosen profile and answers the pools it leaves.
fn pools_after_run(contents: &str, optimize: bool) -> candela_vm::rt::Pools {
    let filename = "test.kl";
    let out = crate::compiler::compile_profile(
        String::from(contents),
        filename,
        true,
        &crate::compiler::imports::ImportResolver::new(),
        optimize,
    );
    let mut pools = out.pools;
    run_vm(
        &out.instructions,
        &mut RegisterFile(out.registers),
        &mut pools,
        &crate::errors::ErrorCtx {
            instr_src: &out.instr_src,
            sources: &[Source {
                filename: filename.into(),
                contents: String::from(contents),
            }],
        },
        &out.callsite_registers,
        &[],
        &[],
        &[],
        out.allocated_arg_count,
        out.allocated_call_depth,
        &[],
        &[],
        0,
    );
    pools
}

#[test]
pub fn json_parse_in_a_loop_keeps_the_pools_bounded() {
    // Each parse leaves its lists and maps behind as garbage. They used to be
    // pushed onto the end of their pools without a look at the free slots or
    // the collector, so every parse grew the pools and none ever started a
    // collection to take the slots back.
    let contents = "
        fn main() {
            let total = 0;
            for i in 0..3000 {
                let v = as_list(json_parse(\"[[1, 2], {\\\"a\\\": 3}]\"));
                total = total + as_int(as_list(v[0])[1]);
            }
            print(total);
        }
    ";
    for optimize in [false, true] {
        let stats = pools_after_run(contents, optimize).gc_stats();
        assert!(stats.cycles > 0, "the parses never started a collection");
        assert!(stats.arrays.len < 512, "{stats:?}");
        assert!(stats.maps.len < 512, "{stats:?}");
    }
}

#[test]
pub fn json_parse_survives_a_collection_mid_parse() {
    // The document holds thousands of lists and maps, so parsing it starts a
    // collection and carries it through marking and sweeping before the
    // parse ends. Everything the parse built so far has to survive that: every
    // entry matches the values the program built the document from, and the
    // answer stringifies back to the document.
    let src = "
        fn main() {
            let doc = \"[\";
            let ids = [];
            let names = [];
            for i in 0..2000 {
                if i > 0 { doc = doc + \",\"; }
                let name = \"an entry named after number \" + str(i);
                doc = doc + \"{\\\"id\\\":\" + str(i) + \",\\\"name\\\":\\\"\" + name + \"\\\",\\\"tags\\\":[\" + str(i) + \",\\\"\" + name + \" again\\\"]}\";
                ids.push(i);
                names.push(name);
            }
            doc = doc + \"]\";
            let bad = 0;
            for round in 0..3 {
                let parsed = as_list(json_parse(doc));
                if json_stringify(parsed) != doc { bad += 1; }
                if parsed.len() != ids.len() { bad += 1; }
                for i in 0..ids.len() {
                    let got = as_map(parsed[i]);
                    if as_int(got.get(\"id\")) != ids[i] { bad += 1; }
                    if as_str(got.get(\"name\")) != names[i] { bad += 1; }
                    let tags = as_list(got.get(\"tags\"));
                    if as_int(tags[0]) != ids[i] { bad += 1; }
                    if as_str(tags[1]) != names[i] + \" again\" { bad += 1; }
                }
            }
            print(bad);
        }
    ";
    assert_eq!(run_output(src), "0\n");
    for optimize in [false, true] {
        let stats = pools_after_run(src, optimize).gc_stats();
        assert!(stats.cycles > 0, "the parses never started a collection");
    }
}

#[test]
pub fn split_survives_a_collection_mid_split() {
    // Each part too long to sit inside a value takes a pool slot through the
    // collector, so a split into thousands of them runs collection work
    // before the list of parts is done. The parts made so far stay whole.
    assert_eq!(
        run_output(
            "
            fn main() {
                let text = \"\";
                for i in 0..3000 {
                    if i > 0 { text = text + \";\"; }
                    text = text + \"part number \" + str(i);
                }
                let bad = 0;
                for round in 0..3 {
                    let parts = text.split(\";\");
                    if parts.len() != 3000 { bad += 1; }
                    for i in 0..parts.len() {
                        if parts[i] != \"part number \" + str(i) { bad += 1; }
                    }
                }
                print(bad);
            }
            "
        ),
        "0\n"
    );
}

#[test]
pub fn gc_state_persists_across_runs() {
    // The pools outlive one run of the interpreter, so what the collector
    // knows about them has to as well. A second run over the same pools
    // reuses the slots the first run's collection freed and keeps the raised
    // threshold, instead of collecting again on its first allocation.
    let filename = "test.kl";
    let contents = "
        fn main() {
            let i = 0;
            while i < 300 {
                let t = [i];
                i = i + 1;
            }
        }
    ";
    let out = compile(
        String::from(contents),
        filename,
        true,
        &crate::compiler::imports::ImportResolver::new(),
    );
    let mut pools = out.pools;
    let mut reg = RegisterFile(out.registers);
    let err_ctx = crate::errors::ErrorCtx {
        instr_src: &out.instr_src,
        sources: &[Source {
            filename: filename.into(),
            contents: String::from(contents),
        }],
    };
    for _ in 0..2 {
        run_vm(
            &out.instructions,
            &mut reg,
            &mut pools,
            &err_ctx,
            &out.callsite_registers,
            &[],
            &[],
            &[],
            out.allocated_arg_count,
            out.allocated_call_depth,
            &[],
            &[],
            0,
        );
    }
    // Under gc-torture every allocation runs the collector, so the count
    // says nothing about thresholds there.
    if !candela_vm::rt::GcState::TORTURE {
        assert_eq!(pools.gc.cycles(), 1);
    }
    assert!(pools.objs.len() < 512, "pool grew to {}", pools.objs.len());
}

#[test]
pub fn idle_collection_stays_within_its_budget() {
    // A host collecting between frames asks for a slice of work and has to
    // get about that much: the budget, plus the one-off read of the roots when
    // a cycle begins, plus at most one mark word's worth of freed slots.
    let filename = "test.kl";
    let contents = "
        struct Row { label: string, cells: int[] }
        fn main() {
            let keep = [];
            for i in 0..7600 {
                keep.push(Row { label: \"a row label long enough to pool \" + str(i), cells: [i] });
                let garbage = [i, i];
            }
        }
    ";
    let out = compile(
        String::from(contents),
        filename,
        true,
        &crate::compiler::imports::ImportResolver::new(),
    );
    let mut pools = out.pools;
    let mut reg = RegisterFile(out.registers);
    let err_ctx = crate::errors::ErrorCtx {
        instr_src: &out.instr_src,
        sources: &[Source {
            filename: filename.into(),
            contents: String::from(contents),
        }],
    };
    run_vm(
        &out.instructions,
        &mut reg,
        &mut pools,
        &err_ctx,
        &out.callsite_registers,
        &[],
        &[],
        &[],
        out.allocated_arg_count,
        out.allocated_call_depth,
        &[],
        &[],
        0,
    );
    let budget = 50;
    let cycles = pools.gc_stats().cycles;
    let mut calls = 0;
    loop {
        let before = pools.gc_stats();
        let free = before.arrays.free + before.maps.free + before.strings.free;
        let slots = before.arrays.len + before.maps.len + before.strings.len;
        // What beginning a cycle may spend reading roots and free slots. The
        // sweep can overrun by one mark word, which covers 64 slots.
        let begin = ((free + reg.0.len() + slots / 64) / 64 + 1) as u64;
        let done = pools.collect(&reg.0, budget);
        let spent = pools.gc_stats().units - before.units;
        assert!(
            spent <= u64::from(budget) + 65 + begin,
            "a slice of budget {budget} did {spent} units"
        );
        calls += 1;
        if done {
            break;
        }
    }
    assert!(calls > 1, "a budget of {budget} finished the whole cycle");
    assert!(pools.gc_stats().cycles > cycles || candela_vm::rt::GcState::TORTURE);
}

#[test]
pub fn strings_past_slot_65536_survive_a_collection() {
    // A string's pool slot is as wide as the value that names it. A collection
    // that freed slot 70000 under a narrower id handed the next string a slot
    // a live string still held, and that string read back as the newcomer.
    run_and_check_registers!(
        r#"
        fn main() {
            let keep = [];
            for i in 0..70000 {
                keep.push("longstring" + str(i));
            }
            for j in 0..200000 {
                let t = "temporary" + str(j);
            }
            let bad = 0;
            for i in 0..70000 {
                if keep[i] != "longstring" + str(i) { bad += 1; }
            }
            print(bad);
        }
        "#,
        0.into()
    );
}

#[test]
pub fn partitioned_lists_survive_a_collection_during_the_partition() {
    // Partitioning makes one list per run between separators. A collection
    // that ran while the later ones were being made freed the earlier ones,
    // and the lists made after it reused their slots.
    run_and_check_registers!(
        "
        fn main() {
            let src = [];
            for i in 0..600 {
                src.push(i);
                src.push(-1);
            }
            let parts = src.partition(-1);
            let bad = 0;
            for i in 0..600 {
                if parts[i].len() != 1 || parts[i][0] != i { bad += 1; }
            }
            print(bad);
        }
        ",
        0.into()
    );
}

#[test]
pub fn an_interned_key_never_names_a_freed_string() {
    // Parsing json interns object keys by comparing text across the string
    // pool. A freed slot kept its text, so a key equal to a dead string was
    // given that slot, and the next string allocated there overwrote the key.
    run_and_check_registers!(
        r#"
        fn main() {
            let i = 0;
            while i < 300 {
                let t = "garbage_string_" + str(i);
                i += 1;
            }
            let doc = as_map(json_parse("{\"garbage_string_5\": 1}"));
            let j = 0;
            while j < 300 {
                let t = "reuse_every_slot_" + str(j);
                j += 1;
            }
            let k = as_str(doc.keys()[0]);
            print(k == "garbage_string_" + str(5));
        }
        "#,
        crate::data::TRUE
    );
}

// The collector marks while the program runs, so a value can move between
// objects during a cycle. These programs move values around while they
// allocate; under the gc-torture feature every allocation runs a unit of
// marking, and any value the barriers miss is caught by the trace check at the
// end of each mark phase or read back wrong here.

#[test]
pub fn a_value_moved_between_lists_during_marking_survives() {
    run_and_check_registers!(
        "
        fn main() {
            let a = [[0]];
            let b = [[1000]];
            for i in 1..200 {
                a.push([i]);
                b.push([i + 1000]);
            }
            for round in 0..3000 {
                let k = round % 200;
                let moved = b[k];
                b[k] = [round];
                a[(k + 37) % 200] = moved;
                let junk = [round, round, round];
            }
            let bad = 0;
            for i in 0..200 {
                if a[i].len() != 1 { bad += 1; }
                if b[i].len() != 1 { bad += 1; }
            }
            print(bad);
        }
        ",
        0.into()
    );
}

#[test]
pub fn a_value_moved_out_of_a_struct_field_during_marking_survives() {
    run_and_check_registers!(
        "
        struct Holder {
            item: int[],
        }

        fn main() {
            let holders = [Holder { item: [0] }];
            for i in 1..100 {
                holders.push(Holder { item: [i] });
            }
            for round in 0..3000 {
                let from = holders[round % 100];
                let to = holders[(round * 7 + 3) % 100];
                let moved = from.item;
                from.item = [round];
                to.item = moved;
                let junk = [round, round, round];
            }
            let bad = 0;
            for h in holders {
                if h.item.len() != 1 { bad += 1; }
            }
            print(bad);
        }
        ",
        0.into()
    );
}

#[test]
pub fn a_captured_variable_reassigned_during_marking_survives() {
    run_and_check_registers!(
        "
        fn main() {
            let current = [0];
            let read = fn() { return current; };
            let bad = 0;
            for round in 1..3000 {
                let previous = read();
                current = [round];
                let junk = [round, round, round];
                if previous.len() != 1 || read()[0] != round { bad += 1; }
            }
            print(bad);
        }
        ",
        0.into()
    );
}

#[test]
pub fn map_values_moved_and_removed_during_marking_survive() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {0: [0]};
            for i in 1..100 {
                m.insert(i, [i]);
            }
            for round in 0..3000 {
                let k = round % 100;
                let moved = m.get(k);
                m.insert(k, [round]);
                m.insert(1000 + round % 7, moved);
                m.remove(1000 + (round + 3) % 7);
                let junk = [round, round, round];
            }
            let bad = 0;
            for v in m.values() {
                if v.len() != 1 { bad += 1; }
            }
            print(bad);
        }
        ",
        0.into()
    );
}

#[test]
pub fn a_long_list_permuted_during_marking_keeps_its_strings() {
    run_and_check_registers!(
        r#"
        fn main() {
            let words = ["entry number 0"];
            for i in 1..300 {
                words.push("entry number " + str(i));
            }
            for round in 0..400 {
                if round % 3 == 0 {
                    words.sort();
                } else if round % 3 == 1 {
                    words.reverse();
                } else {
                    let gone = words[round % 300];
                    words.remove(round % 300);
                    words.push(gone);
                }
                let junk = "temporary string " + str(round);
            }
            let bad = 0;
            for w in words {
                if !w.starts_with("entry number ") { bad += 1; }
            }
            print(bad + words.len());
        }
        "#,
        300.into()
    );
}

#[test]
pub fn a_json_key_interned_during_marking_keeps_its_text() {
    run_and_check_registers!(
        r#"
        fn main() {
            let bad = 0;
            for round in 0..300 {
                let dead = "interned key " + str(round);
                let doc = as_map(json_parse("{\"interned key " + str(round) + "\": [1, 2]}"));
                let junk = "unrelated filler " + str(round);
                let k = as_str(doc.keys()[0]);
                if k != "interned key " + str(round) { bad += 1; }
            }
            print(bad);
        }
        "#,
        0.into()
    );
}

#[test]
pub fn parsed_array_survives_array_gc() {
    // An array a parsed document hangs off its root map is reachable only
    // through that map. The array collector has to follow a map register to
    // see it, or the sweep hands its pool slot to the next allocation and the
    // script reads whatever that wrote.
    let doc = (0..300)
        .map(|i| format!("[{i}]"))
        .collect::<Vec<_>>()
        .join(",");
    run_and_check_registers!(
        &format!(
            r#"
            fn libs_len(body) {{
                let root = as_map(json_parse(body));
                let s = "a,b,c";
                let i = 0;
                while i < 50 {{
                    let parts = s.split(",");
                    i = i + 1;
                }}
                return as_list(root.get("libraries")).len();
            }}

            fn main() {{
                print(libs_len("{{\"libraries\": [{doc}], \"other\": \"x\"}}"));
            }}
        "#
        ),
        300.into()
    );
}

#[test]
pub fn json_roundtrip_preserves_int() {
    run_and_check_registers!(
        "
        fn main() {
            let obj = as_map(json_parse(json_stringify(json_parse(\"{\\\"a\\\": 7}\"))));
            print(as_int(obj.get(\"a\")));
        }
        ",
        7.into()
    );
}

#[test]
pub fn json_roundtrip_preserves_float() {
    run_and_check_registers!(
        "
        fn main() {
            let v = json_parse(json_stringify(json_parse(\"2.5\")));
            print(as_float(v));
        }
        ",
        crate::data::Data::float(2.5)
    );
}

#[test]
pub fn map_empty_literal_and_len() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {};
            m.insert(\"a\", 1);
            m.insert(\"b\", 2);
            print(m.len());
        }
        ",
        2.into()
    );
}

#[test]
pub fn map_contains() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {};
            m.insert(\"a\", 1);
            print(m.contains(\"a\"));
        }
        ",
        true.into()
    );
}

#[test]
pub fn map_contains_absent() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {\"a\": 1};
            print(m.contains(\"z\"));
        }
        ",
        false.into()
    );
}

#[test]
pub fn map_remove_drops_the_entry() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {\"a\": 1, \"b\": 2};
            m.remove(\"a\");
            print(m.len());
        }
        ",
        1.into()
    );
}

#[test]
pub fn map_remove_leaves_the_key_absent() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {\"a\": 1, \"b\": 2};
            m.remove(\"a\");
            print(m.contains(\"a\"));
        }
        ",
        false.into()
    );
}

#[test]
pub fn map_remove_absent_key_changes_nothing() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {\"a\": 1};
            m.remove(\"z\");
            print(m.len());
        }
        ",
        1.into()
    );
}

#[test]
pub fn map_remove_takes_a_typed_key() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {};
            m.insert(1, 10);
            m.insert(2, 20);
            m.remove(1);
            print(m.get(2));
        }
        ",
        20.into()
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn remove_rejects_a_receiver_that_is_neither_list_nor_map() {
    run!(
        "
        fn main() {
            let s = \"abc\";
            s.remove(0);
        }
        "
    );
}

#[test]
pub fn map_keys_values_len() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {};
            m.insert(1, 10);
            m.insert(2, 20);
            m.insert(3, 30);
            print(m.keys().len() + m.values().len());
        }
        ",
        6.into()
    );
}

#[test]
pub fn map_iteration_over_keys() {
    run_and_check_registers!(
        "
        fn main() {
            let m = {};
            m.insert(1, 10);
            m.insert(2, 20);
            let s = 0;
            for k in m {
                s += m.get(k);
            }
            print(s);
        }
        ",
        30.into()
    );
}

#[test]
pub fn map_downcast_takes_mixed_value_types() {
    run_and_check_registers!(
        "
        fn main() {
            let m = as_map(json_parse(\"{\\\"a\\\": 1}\"));
            m.insert(\"b\", 2);
            m.insert(\"c\", \"three\");
            print(m.len());
        }
        ",
        3.into()
    );
}

#[test]
pub fn map_downcast_entries_stay_dynamic() {
    run_and_check_registers!(
        "
        fn main() {
            let m = as_map(json_parse(\"{\\\"a\\\": 1, \\\"b\\\": \\\"two\\\"}\"));
            print(as_int(m.get(\"a\")) + as_str(m.get(\"b\")).len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn map_downcast_takes_mixed_key_types() {
    run_and_check_registers!(
        "
        fn main() {
            let m = as_map(json_parse(\"{\\\"a\\\": 1}\"));
            m.insert(\"b\", 2);
            m.insert(7, 3);
            print(m.len());
        }
        ",
        3.into()
    );
}

#[test]
pub fn list_downcast_takes_mixed_element_types() {
    run_and_check_registers!(
        "
        fn main() {
            let l = as_list(json_parse(\"[1]\"));
            l.push(2);
            l.push(\"three\");
            print(l.len());
        }
        ",
        3.into()
    );
}

#[test]
pub fn list_downcast_elements_stay_dynamic() {
    run_and_check_registers!(
        "
        fn main() {
            let l = as_list(json_parse(\"[1, \\\"two\\\"]\"));
            print(as_int(l[0]) + as_str(l[1]).len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn empty_map_literal_pins_its_types_on_the_first_insert() {
    let src = "fn main() { let m = {}; m.insert(\"a\", 1); m.insert(\"b\", \"s\"); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
    assert_eq!(
        d.message,
        "Function insert expects this argument to be of type int, but this expression's type is string"
    );
}

#[test]
pub fn empty_array_literal_pins_its_type_on_the_first_push() {
    let src = "fn main() { let a = []; a.push(1); a.push(\"s\"); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
    assert_eq!(
        d.message,
        "Function push expects this argument to be of type int, but this expression's type is string"
    );
}

/// A pin applies to a read written in a `return` the same as to one written
/// anywhere else. The return-type walk runs over the body before the compile
/// walk lowers it, so it applies the pins the body's statements perform.
#[test]
pub fn a_pinned_map_reads_back_in_a_return() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn wrap() {
            let m = {};
            m.insert(\"a\", P { x: 5 });
            return m.get(\"a\").x;
        }
        fn main() {
            print(wrap());
        }
        ",
        5.into()
    );
}

#[test]
pub fn a_pinned_array_reads_back_in_a_return() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn wrap() {
            let xs = [];
            xs.push(P { x: 6 });
            return xs[0].x;
        }
        fn main() {
            print(wrap());
        }
        ",
        6.into()
    );
}

/// A declared return type walks the body the same way, so the pin has to hold
/// there too.
#[test]
pub fn a_pinned_map_reads_back_in_an_annotated_return() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn wrap() -> int {
            let m = {};
            m.insert(\"a\", P { x: 7 });
            return m.get(\"a\").x;
        }
        fn main() {
            print(wrap());
        }
        ",
        7.into()
    );
}

/// The pin a call applies to its argument reaches a `return` as well: the
/// parameter's declared type is what says what the caller's empty map holds.
#[test]
pub fn a_parameter_pinned_map_reads_back_in_a_return() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn fill(m: {string: P}) {
            m.insert(\"a\", P { x: 8 });
        }
        fn wrap() {
            let m = {};
            fill(m);
            return m.get(\"a\").x;
        }
        fn main() {
            print(wrap());
        }
        ",
        8.into()
    );
}

#[test]
pub fn a_parameter_pinned_array_reads_back_in_a_return() {
    run_and_check_registers!(
        "
        struct P { x: int }
        fn fill(xs: P[]) {
            xs.push(P { x: 9 });
        }
        fn wrap() {
            let xs = [];
            fill(xs);
            return xs[0].x;
        }
        fn main() {
            print(wrap());
        }
        ",
        9.into()
    );
}

// ---------------------------------------------------------------------------
// TYPED PARAMETERS AND RETURN ANNOTATIONS
// ---------------------------------------------------------------------------

#[test]
pub fn annotated_params_accept_matching_int() {
    run_and_check_registers!(
        "
        fn add(a: int, b: int) {
            return a + b;
        }
        fn main() {
            print(add(1, 2));
        }
        ",
        3.into()
    );
}

#[test]
pub fn annotated_params_accept_matching_float() {
    run_and_check_registers!(
        "
        fn scale(x: float, factor: float) {
            return x * factor;
        }
        fn main() {
            print(scale(1.5, 2.0));
        }
        ",
        3.0.into()
    );
}

#[test]
pub fn annotated_params_accept_matching_string() {
    run_and_check_registers!(
        "
        fn join(a: string, b: string) {
            return a + b;
        }
        fn main() {
            print(join(\"ab\", \"cd\").len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn annotated_params_accept_matching_bool() {
    run_and_check_registers!(
        "
        fn both(a: bool, b: bool) {
            return a && b;
        }
        fn main() {
            print(both(true, true));
        }
        ",
        true.into()
    );
}

#[test]
pub fn annotated_array_param_accepts_matching_array() {
    run_and_check_registers!(
        "
        fn total(xs: int[]) {
            let s = 0;
            for x in xs { s += x; }
            return s;
        }
        fn main() {
            print(total([1, 2, 3]));
        }
        ",
        6.into()
    );
}

#[test]
pub fn annotated_param_any_stays_dynamic() {
    run_and_check_registers!(
        "
        fn pick(x: any) {
            return x;
        }
        fn main() {
            print(pick(4));
        }
        ",
        4.into()
    );
}

#[test]
pub fn unannotated_params_still_specialise_per_call() {
    run_and_check_registers!(
        "
        fn same(x) {
            return x;
        }
        fn main() {
            print(same(\"a\").len() + same(2));
        }
        ",
        3.into()
    );
}

#[test]
pub fn annotated_param_rejects_mismatch() {
    let src = "fn add(a: int, b: int) { return a + b; }
fn main() { print(add(1, \"two\")); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "argument_type_mismatch");
    assert_eq!(
        d.message,
        "Function add expects this argument's type to be int, but this expression's type is string"
    );
}

#[test]
pub fn annotated_param_rejects_float_for_int() {
    let src = "fn twice(n: int) { return n * 2; }
fn main() { print(twice(1.5)); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "argument_type_mismatch");
    assert!(d.message.contains("this expression's type is float"));
}

#[test]
pub fn annotated_method_param_rejects_mismatch() {
    let src = "struct Rect { w: int, h: int }
impl Rect {
    fn grow(self, by: int) { return self.w + by; }
}
fn main() { let r = Rect { w: 1, h: 2 }; print(r.grow(\"x\")); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "argument_type_mismatch");
}

#[test]
pub fn method_param_annotation_accepts_match() {
    run_and_check_registers!(
        "
        struct Rect { w: int, h: int }
        impl Rect {
            fn grow(self, by: int) { return self.w + by; }
        }
        fn main() {
            let r = Rect { w: 1, h: 2 };
            print(r.grow(4));
        }
        ",
        5.into()
    );
}

#[test]
pub fn return_annotation_accepts_matching_type() {
    run_and_check_registers!(
        "
        fn add(a: int, b: int) -> int {
            return a + b;
        }
        fn main() {
            print(add(2, 3));
        }
        ",
        5.into()
    );
}

#[test]
pub fn return_annotation_rejects_mismatch() {
    let src = "fn label(n: int) -> int { return \"x\"; }
fn main() { print(label(1)); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_type");
    assert!(d.message.contains("expected int"));
    assert!(d.message.contains("type is string"));
}

// `null` is a keyword, not an identifier, so the type grammar has no way to
// spell it; a function that returns nothing leaves the annotation off.
#[test]
pub fn unannotated_return_still_infers() {
    run_and_check_registers!(
        "
        fn shout(n: int) {
            print(n);
        }
        fn main() {
            shout(9);
        }
        ",
        9.into()
    );
}

#[test]
pub fn return_annotation_rejects_missing_return() {
    let src = "fn broken(n: int) -> int { print(n); }
fn main() { broken(1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "invalid_type");
}

#[test]
pub fn method_return_annotation_is_checked() {
    let src = "struct Rect { w: int, h: int }
impl Rect {
    fn area(self) -> string { return self.w * self.h; }
}
fn main() { let r = Rect { w: 2, h: 3 }; print(r.area()); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "invalid_type");
}

#[test]
pub fn method_return_annotation_accepts_match() {
    run_and_check_registers!(
        "
        struct Rect { w: int, h: int }
        impl Rect {
            fn area(self) -> int { return self.w * self.h; }
        }
        fn main() {
            let r = Rect { w: 2, h: 3 };
            print(r.area());
        }
        ",
        6.into()
    );
}

#[test]
pub fn nested_fn_reports_a_parse_error() {
    let src = "fn main() { fn helper() { return 1; } print(helper()); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "nested_function_declaration");
    assert!(d.message.contains("top level"));
}

// ---------------------------------------------------------------------------
// NON-BOOL CONDITIONS
//
// A condition is a bool. Every other known type is reported at the condition,
// and a condition typed `any` is checked when it runs.
// ---------------------------------------------------------------------------

#[test]
pub fn if_rejects_a_non_bool_condition() {
    for condition in ["0", "null", "\"s\""] {
        let src = format!(
            "
        fn main() {{
            if {condition} {{ print(1); }} else {{ print(2); }}
        }}
        "
        );
        let d = compile_diag(&src, "diag.kl").unwrap_err();
        assert_wellformed(&d, &src);
        assert_eq!(d.code, "non_bool_condition", "{}", d.message);
    }
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn else_if_rejects_non_bool_condition() {
    run!(
        "
        fn main() {
            if false { print(0); } else if 3 { print(1); } else { print(2); }
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn while_rejects_non_bool_condition() {
    run!(
        "
        fn main() {
            let n = 0;
            while 1 {
                n += 1;
                break;
            }
            print(n);
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn inline_if_rejects_non_bool_condition() {
    run!(
        "
        fn main() {
            let x = if 1 { 7 } else { 8 };
            print(x);
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn bool_operand_of_and_rejects_non_bool() {
    run!(
        "
        fn main() {
            if true && 1 { print(1); }
        }
        "
    );
}

#[test]
#[should_panic(expected = "explicit panic")]
pub fn specialised_body_rejects_non_bool_condition() {
    // The closure returns an int, so the condition in the body compiled for it
    // is an int. A comparator written this way is what #134 was found through.
    run!(
        "
        fn pick(f) {
            if f(2, 1) {
                return \"a\";
            }
            return \"b\";
        }

        fn main() {
            print(pick(fn(a, b) { return a - b; }));
        }
        "
    );
}

#[test]
pub fn any_condition_raises_on_a_non_bool() {
    // Nothing pins the type of a parsed json value, so the check happens when
    // the condition runs and raises a catchable `bad_downcast`.
    run_and_check_registers!(
        "
        fn main() {
            let r = 0;
            try {
                if json_parse(\"1\") {
                    r = 1;
                }
            } catch e {
                r = 2;
            }
            print(r);
        }
        ",
        2.into()
    );
}

#[test]
pub fn any_condition_takes_a_bool() {
    run_and_check_registers!(
        "
        fn main() {
            let r = 0;
            if json_parse(\"true\") {
                r = 1;
            }
            print(r);
        }
        ",
        1.into()
    );
}

#[test]
pub fn a_false_condition_still_takes_the_else_branch() {
    run_and_check_registers!(
        "
        fn main() {
            if false { print(1); } else { print(2); }
        }
        ",
        2.into()
    );
}

#[test]
pub fn bool_conditions_still_compile() {
    run_and_check_registers!(
        "
        fn main() {
            let n = 3;
            if n > 2 { print(1); } else { print(0); }
        }
        ",
        1.into()
    );
}

// ---------------------------------------------------------------------------
// EQUALITY ACROSS TYPES
//
// Comparing a string with a value of another type compiles and is unequal. The
// string-comparison instruction is chosen whenever either side is statically a
// string, so the run-time guard is what keeps a mismatched pair meaningful.
// ---------------------------------------------------------------------------

#[test]
pub fn eq_string_against_int_is_false() {
    run_and_check_registers!(
        "
        fn main() {
            print(\"a\" == 1);
        }
        ",
        false.into()
    );
}

#[test]
pub fn neq_int_against_string_is_true() {
    run_and_check_registers!(
        "
        fn main() {
            print(1 != \"a\");
        }
        ",
        true.into()
    );
}

#[test]
pub fn eq_string_against_bool_is_false() {
    run_and_check_registers!(
        "
        fn main() {
            print(\"a\" == true);
        }
        ",
        false.into()
    );
}

#[test]
pub fn eq_string_against_null_is_false() {
    run_and_check_registers!(
        "
        fn main() {
            print(\"a\" == null);
        }
        ",
        false.into()
    );
}

#[test]
pub fn digit_string_is_not_equal_to_the_number() {
    run_and_check_registers!(
        "
        fn main() {
            print(\"5\" == 5);
        }
        ",
        false.into()
    );
}

#[test]
pub fn mixed_equality_as_a_condition_takes_the_else_branch() {
    run_and_check_registers!(
        "
        fn main() {
            if \"a\" == 1 { print(1); } else { print(2); }
        }
        ",
        2.into()
    );
}

#[test]
pub fn mixed_inequality_as_a_condition_takes_the_true_branch() {
    run_and_check_registers!(
        "
        fn main() {
            if \"a\" != 1 { print(1); } else { print(2); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn mixed_equality_through_an_untyped_parameter() {
    // `v` is whatever the call site passes, so the string comparison the
    // compiler picks meets a non-string operand only at run time.
    run_and_check_registers!(
        "
        fn same_as_text(v) { return v == \"5\"; }
        fn main() { print(same_as_text(5)); }
        ",
        false.into()
    );
}

#[test]
pub fn matching_strings_through_an_untyped_parameter() {
    run_and_check_registers!(
        "
        fn same_as_text(v) { return v == \"5\"; }
        fn main() { print(same_as_text(\"5\")); }
        ",
        true.into()
    );
}

#[test]
pub fn eq_on_matching_strings_still_works() {
    run_and_check_registers!(
        "
        fn main() {
            print(\"a\" == \"a\");
        }
        ",
        true.into()
    );
}

#[test]
pub fn eq_on_matching_ints_still_works() {
    run_and_check_registers!(
        "
        fn main() {
            print(2 != 3);
        }
        ",
        true.into()
    );
}

// ---------------------------------------------------------------------------
// UNARY ! REPORTS ITS OWN SYMBOL
// ---------------------------------------------------------------------------

#[test]
pub fn bool_neg_names_the_bang_operator() {
    let src = "fn main() { let n = 1; print(!n); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation ! int");
}

// ---------------------------------------------------------------------------
// CONSTANT FOLDING MATCHES THE TYPE CHECKER
// ---------------------------------------------------------------------------

#[test]
pub fn folded_pow_rejects_mixed_operands() {
    let src = "fn main() { print(2.0 ^ 3); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation float ^ int");
}

#[test]
pub fn unfolded_pow_rejects_mixed_operands() {
    let src = "fn main() { let f = 2.0; print(f ^ 3); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation float ^ int");
}

#[test]
pub fn folded_pow_accepts_matching_floats() {
    run_and_check_registers!(
        "
        fn main() {
            print(2.0 ^ 3.0);
        }
        ",
        8.0.into()
    );
}

/// A bitwise operator takes two `int` operands, and any other pair is reported
/// the way a wrong pair of arithmetic operands is.
#[test]
pub fn bitwise_rejects_a_float_operand() {
    let src = "fn main() { print(1.5 & 1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation float & int");
}

#[test]
pub fn bitwise_rejects_a_string_operand() {
    let src = "fn main() { print(\"a\" | 2); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation string | int");
}

#[test]
pub fn bitwise_rejects_an_any_operand() {
    let src = "struct Wrap { v: any } fn main() { let w = Wrap { v: 3 }; print(w.v & 1); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation any & int");
}

/// `~` names only its operand, the way `!` and prefix `-` do.
#[test]
pub fn bit_not_names_the_tilde_operator() {
    let src = "fn main() { print(~1.5); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation ~ float");
}

#[test]
pub fn shift_rejects_a_float_count() {
    let src = "fn main() { let x = 1; print(x << 2.0); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation int << float");
}

/// An `int` holds 64 bits, so a count of 64 or more, or a negative one, names
/// no result. A count written as a literal is refused before the program runs,
/// whether the value being shifted is a literal too or not.
#[test]
pub fn a_literal_shift_count_past_the_width_is_a_compile_error() {
    for src in [
        "fn main() { print(1 << 64); }",
        "fn main() { print(1 >> 64); }",
        "fn main() { print(1 << -1); }",
        "fn main() { let x = 1; print(x << 64); }",
        "fn main() { let x = 1; print(x >> -1); }",
        "fn main() { let x = 1; x <<= 64; print(x); }",
    ] {
        let d = compile_diag(src, "diag.kl").unwrap_err();
        assert_wellformed(&d, src);
        assert_eq!(d.code, "shift_count_out_of_range", "{src}");
        assert!(d.message.contains("64 bits wide"), "{}", d.message);
    }
}

/// The widest shift there is still compiles, so the check refuses only what is
/// past the end.
#[test]
pub fn a_shift_by_63_is_in_range() {
    run_and_check_registers!("fn main() { print(1 << 63); }", i64::MIN.into());
    run_and_check_registers!("fn main() { print(1 >> 0); }", 1.into());
}

/// A count only known while the program runs raises instead, and the error is
/// catchable like division by zero.
#[test]
pub fn a_runtime_shift_count_past_the_width_raises() {
    let src = "fn main() { let x = 1; let n = 64; print(x << n); }";
    let d = run_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "shift_count_out_of_range");

    let src = "fn main() { let x = 1; let n = -1; print(x >> n); }";
    let d = run_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "shift_count_out_of_range");
}

#[test]
pub fn a_shift_count_out_of_range_is_catchable() {
    run_and_check_registers!(
        "
        fn main() {
            let x = 1;
            let n = 64;
            try {
                print(x << n);
            } catch \"shift_count_out_of_range\" {
                print(99);
            }
        }
        ",
        99.into()
    );
}

#[test]
pub fn division_by_literal_zero_is_a_parse_error() {
    let src = "fn main() { let n = 4; print(n / 0); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "division_by_zero");
}

#[test]
pub fn float_division_by_literal_zero_follows_ieee() {
    run_and_check_registers!(
        "
        fn main() {
            print(1.0 / 0.0 > 0.0);
        }
        ",
        true.into()
    );
}

#[test]
pub fn float_remainder_by_literal_zero_follows_ieee() {
    run_and_check_registers!(
        "
        fn main() {
            print(1.0 % 0.0 > 0.0);
        }
        ",
        false.into()
    );
}

#[test]
pub fn remainder_by_literal_int_zero_is_a_parse_error() {
    let src = "fn main() { let n = 4; print(n % 0); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "modulo_by_zero");
}

#[test]
pub fn float_literal_divided_by_int_zero_is_a_type_error() {
    let src = "fn main() { print(2.0 / 0); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation float / int");
}

// ---------------------------------------------------------------------------
// SHORT-CIRCUIT EVALUATION EVERYWHERE
// ---------------------------------------------------------------------------

#[test]
pub fn and_short_circuits_in_a_let() {
    // `bump` raises when it runs, so reaching it fails the test.
    run_and_check_registers!(
        "
        fn boom() {
            throw(\"reached\");
            return true;
        }
        fn main() {
            let ok = false && boom();
            print(ok);
        }
        ",
        false.into()
    );
}

#[test]
pub fn or_short_circuits_in_a_let() {
    run_and_check_registers!(
        "
        fn boom() {
            throw(\"reached\");
            return false;
        }
        fn main() {
            let ok = true || boom();
            print(ok);
        }
        ",
        true.into()
    );
}

#[test]
pub fn and_short_circuits_in_a_call_argument() {
    run_and_check_registers!(
        "
        fn boom() {
            throw(\"reached\");
            return true;
        }
        fn id(b) { return b; }
        fn main() {
            print(id(false && boom()));
        }
        ",
        false.into()
    );
}

#[test]
pub fn or_short_circuits_in_a_return_value() {
    run_and_check_registers!(
        "
        fn boom() {
            throw(\"reached\");
            return false;
        }
        fn check(b) {
            return b || boom();
        }
        fn main() {
            print(check(true));
        }
        ",
        true.into()
    );
}

#[test]
pub fn short_circuit_value_still_evaluates_the_right_side() {
    run_and_check_registers!(
        "
        fn main() {
            let a = true && false;
            let b = false || true;
            print(a == false && b == true);
        }
        ",
        true.into()
    );
}

#[test]
pub fn and_still_short_circuits_as_a_condition() {
    run_and_check_registers!(
        "
        fn boom() {
            throw(\"reached\");
            return true;
        }
        fn main() {
            if false && boom() { print(0); } else { print(1); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn or_of_and_still_short_circuits_as_a_condition() {
    run_and_check_registers!(
        "
        fn boom() {
            throw(\"reached\");
            return true;
        }
        fn main() {
            if true || boom() && boom() { print(1); } else { print(0); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn and_of_or_evaluates_correctly_as_a_condition() {
    run_and_check_registers!(
        "
        fn main() {
            let a = false;
            let b = true;
            let c = true;
            if (a || b) && c { print(1); } else { print(0); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn or_inside_and_takes_the_false_path() {
    run_and_check_registers!(
        "
        fn main() {
            let a = false;
            let b = false;
            let c = true;
            if (a || b) && c { print(1); } else { print(0); }
        }
        ",
        0.into()
    );
}

#[test]
pub fn and_of_or_on_the_right_evaluates_correctly() {
    run_and_check_registers!(
        "
        fn main() {
            let a = true;
            let b = false;
            let c = true;
            if a && (b || c) { print(1); } else { print(0); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn short_circuit_in_a_while_condition() {
    run_and_check_registers!(
        "
        fn main() {
            let i = 0;
            let go = true;
            while go && i < 3 {
                i += 1;
            }
            print(i);
        }
        ",
        3.into()
    );
}

#[test]
pub fn bool_ops_reject_non_bool_operands() {
    let src = "fn main() { print(1 && true); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_eq!(d.code, "invalid_operation");
    assert_eq!(d.message, "Cannot perform operation int && bool");
}

// ---------------------------------------------------------------------------
// COLLECTION LITERALS PASSED DIRECTLY AS ARGUMENTS
//
// A literal argument is built into a destination register by a group of
// instructions, and the call has to redirect that group into the parameter
// slot. A map literal holding a non-constant value used to abort the compiler
// outright, and an array literal built the same way reached the callee empty.
// ---------------------------------------------------------------------------

#[test]
pub fn const_array_literal_argument() {
    run_and_check_registers!(
        "
        fn head(a) { return a[0]; }
        fn main() { print(head([5, 6])); }
        ",
        5.into()
    );
}

#[test]
pub fn dynamic_array_literal_argument() {
    run_and_check_registers!(
        "
        fn head(a) { return a[0]; }
        fn main() {
            let n = 5;
            print(head([n, 6]));
        }
        ",
        5.into()
    );
}

#[test]
pub fn const_map_literal_argument() {
    run_and_check_registers!(
        "
        fn lookup(m) { return m.get(\"k\"); }
        fn main() { print(lookup({\"k\": 7})); }
        ",
        7.into()
    );
}

#[test]
pub fn dynamic_map_literal_argument() {
    run_and_check_registers!(
        "
        fn lookup(m) { return m.get(\"k\"); }
        fn main() {
            let n = 7;
            print(lookup({\"k\": n}));
        }
        ",
        7.into()
    );
}

#[test]
pub fn dynamic_map_literal_argument_ignored_by_callee() {
    run_and_check_registers!(
        "
        fn take(m) {}
        fn main() {
            let n = 7;
            take({\"k\": n});
            print(1);
        }
        ",
        1.into()
    );
}

#[test]
pub fn array_literal_argument_to_method() {
    run_and_check_registers!(
        "
        struct Box { n: int }
        impl Box {
            fn first(self, a) { return a[0] + self.n; }
        }
        fn main() {
            let b = Box { n: 1 };
            let x = 4;
            print(b.first([x, 9]));
        }
        ",
        5.into()
    );
}

#[test]
pub fn map_literal_argument_to_method() {
    run_and_check_registers!(
        "
        struct Box { n: int }
        impl Box {
            fn at(self, m) { return m.get(\"k\") + self.n; }
        }
        fn main() {
            let b = Box { n: 1 };
            let v = 6;
            print(b.at({\"k\": v}));
        }
        ",
        7.into()
    );
}

#[test]
pub fn nested_dynamic_literals_as_arguments() {
    run_and_check_registers!(
        "
        fn total(rows) { return rows[0][0] + rows[1][0]; }
        fn main() {
            let a = 2;
            let b = 3;
            print(total([[a, 0], [b, 0]]));
        }
        ",
        5.into()
    );
}

#[test]
pub fn dynamic_map_literal_in_a_loop_body() {
    run_and_check_registers!(
        "
        fn lookup(m) { return m.get(\"k\"); }
        fn main() {
            let s = 0;
            for i in 0..3 {
                s += lookup({\"k\": i});
            }
            print(s);
        }
        ",
        3.into()
    );
}

#[test]
pub fn dynamic_array_literal_in_a_loop_body() {
    run_and_check_registers!(
        "
        fn head(a) { return a[0]; }
        fn main() {
            let s = 0;
            for i in 0..3 {
                s += head([i, 9]);
            }
            print(s);
        }
        ",
        3.into()
    );
}

// ---------------------------------------------------------------------------
// Macros
//
// `name!( ... )` hands the raw region to the expander the embedder registered
// and parses the candela source that comes back in its place. candela knows
// where a region starts and ends and nothing else about it.
// ---------------------------------------------------------------------------

use crate::data::NULL;
use crate::macros::MacroEnv;
use crate::macros::MacroError;
use crate::macros::scan_regions;

/// An environment whose one macro, `lmn!`, expands to the number of bytes in
/// its region plus two. Region-dependent, so a test can tell an expansion from
/// a constant.
fn stub_env() -> MacroEnv {
    let mut env = MacroEnv::new();
    env.register("lmn", |body: &str| Ok(format!("{} + 2", body.trim().len())));
    env
}

#[test]
pub fn macro_expands_in_an_expression() {
    stub_env().scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(lmn!(<div/>) * 1);
            }
            ",
            8.into()
        );
    });
}

#[test]
pub fn macro_expands_in_a_variable_declaration() {
    stub_env().scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                let markup = lmn!(abc);
                print(markup);
            }
            ",
            5.into()
        );
    });
}

#[test]
pub fn macro_expands_as_a_call_argument() {
    stub_env().scope(|| {
        run_and_check_registers!(
            "
            fn twice(n) { return n * 2; }
            fn main() {
                print(twice(lmn!(ab)));
            }
            ",
            8.into()
        );
    });
}

#[test]
pub fn macro_region_keeps_parentheses_inside_strings() {
    let mut env = MacroEnv::new();
    env.register("lmn", |body: &str| Ok(format!("{}", body.len())));
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(lmn!(\"a)b\"));
            }
            ",
            5.into()
        );
    });
}

#[test]
pub fn macro_region_balances_nested_parentheses() {
    let mut env = MacroEnv::new();
    env.register("lmn", |body: &str| Ok(format!("{}", body.len())));
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(lmn!(f(g(1))));
            }
            ",
            7.into()
        );
    });
}

#[test]
pub fn an_identifier_spelled_like_a_macro_is_still_an_identifier() {
    run_and_check_registers!(
        "
        fn main() {
            let lmn = 4;
            print(lmn);
        }
        ",
        4.into()
    );
}

#[test]
pub fn a_bang_after_an_identifier_is_still_negation() {
    run_and_check_registers!(
        "
        fn main() {
            let lmn = false;
            if !lmn { print(1); } else { print(0); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn unknown_macro_is_a_compile_error() {
    let err = compile_diag(
        "
        fn main() {
            print(lmn!(<div/>));
        }
        ",
        "macros.cdl",
    )
    .expect_err("an unregistered macro does not compile");
    assert_eq!(err.code, "unknown_macro");
    assert!(err.message.contains("lmn!"), "{}", err.message);
}

#[test]
pub fn an_unknown_macro_is_null_when_the_environment_allows_it() {
    let mut env = MacroEnv::new();
    env.allow_unknown(true);
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(lmn!(<div/>));
            }
            ",
            NULL
        );
    });
}

#[test]
pub fn a_registered_macro_still_expands_when_unknown_ones_are_allowed() {
    let mut env = stub_env();
    env.allow_unknown(true);
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(lmn!(abc));
            }
            ",
            5.into()
        );
    });
}

#[test]
pub fn an_unterminated_region_names_the_macro() {
    let err = stub_env()
        .scope(|| compile_diag("fn main() { let markup = lmn!(<div/>; }", "macros.cdl"))
        .expect_err("an unclosed region does not compile");
    assert_eq!(err.code, "unterminated_macro_region");
    assert!(err.message.contains("lmn!"), "{}", err.message);
}

#[test]
pub fn an_expander_error_points_into_the_region() {
    let mut env = MacroEnv::new();
    env.register("lmn", |body: &str| {
        Err::<String, _>(MacroError::at(
            "unclosed tag",
            body.find("<span").unwrap_or(0),
        ))
    });
    let src = "fn main() { print(lmn!(<div><span>)); }";
    let err = env
        .scope(|| compile_diag(src, "macros.cdl"))
        .expect_err("a refused region does not compile");
    assert_eq!(err.code, "macro_expansion_failed");
    assert!(err.message.contains("unclosed tag"), "{}", err.message);
    // The offset is a position in the region body; the diagnostic is a position
    // in the file.
    assert_eq!(err.span.start, src.find("<span").unwrap());
}

#[test]
pub fn an_expander_error_without_an_offset_covers_the_invocation() {
    let mut env = MacroEnv::new();
    env.register("lmn", |_: &str| {
        Err::<String, _>(MacroError::new("no markup here"))
    });
    let src = "fn main() { print(lmn!(<div/>)); }";
    let err = env
        .scope(|| compile_diag(src, "macros.cdl"))
        .expect_err("a refused region does not compile");
    assert_eq!(err.span.start, src.find("lmn!").unwrap());
    assert_eq!(err.span.end, src.find("));").unwrap() + 1);
}

#[test]
pub fn an_expansion_of_several_expressions_is_rejected() {
    let mut env = MacroEnv::new();
    env.register("lmn", |_: &str| Ok(String::from("1 2")));
    let err = env
        .scope(|| compile_diag("fn main() { print(lmn!(x)); }", "macros.cdl"))
        .expect_err("an expansion is one expression");
    assert_eq!(err.code, "macro_expansion_trailing_tokens");
}

#[test]
pub fn an_error_inside_an_expansion_points_at_the_macro() {
    let mut env = MacroEnv::new();
    env.register("lmn", |_: &str| Ok(String::from("1 +")));
    let src = "fn main() { print(lmn!(x)); }";
    let err = env
        .scope(|| compile_diag(src, "macros.cdl"))
        .expect_err("broken expanded source does not compile");
    assert_eq!(err.span.start, src.find("lmn!").unwrap());
    assert!(err.span.end <= src.len());
}

#[test]
pub fn macros_are_unknown_again_once_the_scope_ends() {
    stub_env().scope(|| ());
    let err = compile_diag("fn main() { print(lmn!(x)); }", "macros.cdl")
        .expect_err("the environment is no longer installed");
    assert_eq!(err.code, "unknown_macro");
}

#[test]
pub fn scan_regions_finds_every_invocation() {
    let src = "fn main() { let a = lmn!(one); let b = lmn!(t(w)o); }";
    let regions = scan_regions(src, "lmn");
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].body, "one");
    assert_eq!(regions[1].body, "t(w)o");
    for region in &regions {
        assert_eq!(&src[region.span.clone()], &format!("lmn!({})", region.body));
        assert_eq!(&src[region.body_start..], &src[region.body_start..]);
        assert!(src[region.body_start..].starts_with(region.body));
    }
}

#[test]
pub fn scan_regions_skips_strings_and_comments() {
    let src = "
fn main() {
    let quoted = \"lmn!(not a macro)\";
    // lmn!(not a macro either)
    let real = lmn!(<p>);
}
";
    let regions = scan_regions(src, "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, "<p>");
}

#[test]
pub fn scan_regions_counts_only_parentheses_outside_strings_and_comments() {
    let src = "lmn!(a \")\" b // )
c)";
    let regions = scan_regions(src, "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, "a \")\" b // )\nc");
}

#[test]
pub fn scan_regions_handles_an_escaped_quote_in_a_region() {
    let src = r#"lmn!("a\")" b)"#;
    let regions = scan_regions(src, "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, r#""a\")" b"#);
}

#[test]
pub fn scan_regions_stops_at_a_region_that_is_never_closed() {
    let regions = scan_regions("lmn!(closed) lmn!(open", "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, "closed");
}

#[test]
pub fn scan_regions_matches_the_name_exactly() {
    let regions = scan_regions("lmn!(a) lmnx!(b) xlmn!(c)", "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, "a");
}

#[test]
pub fn scan_regions_carries_multibyte_text_through() {
    let src = "lmn!(caf\u{e9} (\u{e9}) end)";
    let regions = scan_regions(src, "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, "caf\u{e9} (\u{e9}) end");
    assert_eq!(&src[regions[0].span.clone()], src);
}

#[test]
pub fn a_region_of_multibyte_text_reaches_the_expander() {
    let mut env = MacroEnv::new();
    env.register("lmn", |body: &str| Ok(body.chars().count().to_string()));
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(lmn!(caf\u{e9}));
            }
            ",
            4.into()
        );
    });
}

#[test]
pub fn a_comment_in_a_region_ends_at_a_carriage_return() {
    let regions = scan_regions("lmn!(a // )\r b)\r", "lmn");
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].body, "a // )\r b");
}

#[test]
pub fn a_macro_that_expands_to_itself_stops_at_the_limit() {
    let mut env = MacroEnv::new();
    env.register("lmn", |_: &str| Ok(String::from("lmn!(again)")));
    let src = "fn main() { print(lmn!(<div/>)); }";
    let err = env
        .scope(|| compile_diag(src, "macros.cdl"))
        .expect_err("an expansion that uses itself does not compile");
    assert_eq!(err.code, "macro_recursion_limit");
    assert!(err.message.contains("lmn!"), "{}", err.message);
    // The report is against the invocation in the file, not one of the
    // expansions the reader never wrote.
    assert_eq!(err.span.start, src.find("lmn!").unwrap());
}

#[test]
pub fn a_chain_of_macros_within_the_limit_expands() {
    // Thirty-two macros, each expanding to the next, is the deepest chain that
    // still compiles.
    let mut env = MacroEnv::new();
    for step in 0..31 {
        env.register(&format!("m{step}"), move |_: &str| {
            Ok(format!("m{}!(x)", step + 1))
        });
    }
    env.register("m31", |_: &str| Ok(String::from("9")));
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(m0!(x));
            }
            ",
            9.into()
        );
    });
}

#[test]
pub fn a_chain_of_macros_past_the_limit_is_an_error() {
    let mut env = MacroEnv::new();
    for step in 0..32 {
        env.register(&format!("m{step}"), move |_: &str| {
            Ok(format!("m{}!(x)", step + 1))
        });
    }
    env.register("m32", |_: &str| Ok(String::from("9")));
    let err = env
        .scope(|| compile_diag("fn main() { print(m0!(x)); }", "macros.cdl"))
        .expect_err("one macro too many");
    assert_eq!(err.code, "macro_recursion_limit");
    assert!(err.message.contains("m32!"), "{}", err.message);
}

#[test]
pub fn the_expansion_limit_resets_between_macros() {
    let mut env = MacroEnv::new();
    for step in 0..31 {
        env.register(&format!("m{step}"), move |_: &str| {
            Ok(format!("m{}!(x)", step + 1))
        });
    }
    env.register("m31", |_: &str| Ok(String::from("1")));
    env.scope(|| {
        run_and_check_registers!(
            "
            fn main() {
                print(m0!(x) + m0!(x));
            }
            ",
            2.into()
        );
    });
}

// ---------------------------------------------------------------------------
// GENERICS (type parameters)
//
// A generic declaration is a template: `Cell<int>` registers an ordinary struct
// under that name and every later stage sees a plain concrete type. `<` is also
// the comparison operator, so the shapes that must stay comparisons have a
// group of their own below.
// ---------------------------------------------------------------------------

#[test]
pub fn generic_struct_and_impl() {
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        impl Cell<T> {
            fn get(self) -> T { return self.value; }
        }
        fn main() {
            let c = Cell<int>{ value: 3 };
            print(c.get());
        }
        ",
        3.into()
    );
}

#[test]
pub fn generic_function_with_explicit_type_argument() {
    run_and_check_registers!(
        "
        fn first<T>(items: T[]) -> T { return items[0]; }
        fn main() {
            let nums = [10, 20, 30];
            print(first<int>(nums));
        }
        ",
        10.into()
    );
}

#[test]
pub fn generic_function_without_type_arguments_still_infers() {
    // Leaving the type arguments off is never an error: the parameter falls
    // back to the inference every un-annotated parameter gets.
    run_and_check_registers!(
        "
        fn first<T>(items: T[]) -> T { return items[0]; }
        fn main() {
            print(first([7, 8]));
        }
        ",
        7.into()
    );
}

#[test]
pub fn an_explicit_type_argument_pins_the_parameter() {
    let err = compile_diag(
        "fn first<T>(items: T[]) -> T { return items[0]; }
         fn main() { print(first<int>([1.5, 2.5])); }",
        "generics.cdl",
    )
    .expect_err("a float[] where int[] was named");
    assert_eq!(err.code, "argument_type_mismatch");
}

#[test]
pub fn generic_struct_literal_without_type_arguments() {
    // The type argument comes from the value in the field declared with it.
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        impl Cell<T> { fn get(self) -> T { return self.value; } }
        fn main() {
            let c = Cell{ value: 12 };
            print(c.get());
        }
        ",
        12.into()
    );
}

#[test]
pub fn generic_instantiations_are_distinct_types() {
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        impl Cell<T> { fn get(self) -> T { return self.value; } }
        fn main() {
            let i = Cell<int>{ value: 2 };
            let s = Cell<string>{ value: \"ab\" };
            print(i.get() + s.get().len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn generic_instantiation_is_cached() {
    // Two spellings of one instantiation are one struct, so a value built by
    // one reaches a function annotated with the other.
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        fn take(c: Cell<int>) -> int { return c.value; }
        fn main() {
            print(take(Cell<int>{ value: 5 }));
        }
        ",
        5.into()
    );
}

/// An instantiation is cached for the whole program, so the `impl` methods
/// lowered for it have to live that long as well. The block that first named
/// the type used to take them with it when it ended, and the next body that
/// named the type hit the cache, lowered nothing, and found a type with no
/// methods on it.
#[test]
pub fn an_instantiations_methods_outlive_the_block_that_named_it() {
    run_and_check_registers!(
        "
        struct Signal<T> { name: string }
        fn signal<T>(name: string) -> Signal<T> { return Signal<T>{ name: name }; }
        impl Signal<bool> {
            fn get(self) -> bool { return true; }
            fn set(self, v: bool) { print(v); }
        }
        fn one() { signal<bool>(\"a\").set(true); }
        fn two() -> int {
            if signal<bool>(\"a\").get() { return 9; }
            return 0;
        }
        fn main() {
            one();
            print(two());
        }
        ",
        9.into()
    );
}

/// The same defect from a loop body: the instantiation is made inline inside
/// the `while`, so the block that lowers the methods is the loop's, and the
/// call in the next function comes after that block has ended.
#[test]
pub fn a_method_lowered_in_a_loop_body_outlives_the_loop() {
    run_and_check_registers!(
        "
        struct Signal<T> { name: string }
        fn signal<T>(name: string) -> Signal<T> { return Signal<T>{ name: name }; }
        impl Signal<bool> {
            fn get(self) -> bool { return true; }
            fn set(self, v: bool) { print(v); }
        }
        fn writer() {
            let i = 0;
            while i < 2 {
                signal<bool>(\"a\").set(true);
                i += 1;
            }
        }
        fn reader() -> int {
            if signal<bool>(\"a\").get() { return 8; }
            return 0;
        }
        fn main() {
            writer();
            print(reader());
        }
        ",
        8.into()
    );
}

/// A specialisation of a generic free function is kept on that function's own
/// table entry, which the block that first needed it never owned, so a later
/// caller reuses it.
#[test]
pub fn a_free_function_specialisation_outlives_the_block_it_was_made_in() {
    run_and_check_registers!(
        "
        fn same<T>(x: T) -> T { return x; }
        fn in_a_block() {
            let i = 0;
            while i < 2 {
                same<int>(3);
                i += 1;
            }
        }
        fn elsewhere() -> int { return same<int>(4); }
        fn main() {
            in_a_block();
            print(elsewhere());
        }
        ",
        4.into()
    );
}

/// A block does not take the function table back down with it. The closure it
/// hoisted and the method it lowered both stay, and neither takes a slot in the
/// saved-register table, which only recursive call sites write to.
#[test]
pub fn a_block_leaves_the_function_table_alone() {
    let out = compile(
        String::from(
            "
        struct Cell<T> { value: T }
        impl Cell<T> { fn get(self) -> T { return self.value; } }
        fn apply(f, n) { return f(n); }
        fn main() {
            let i = 0;
            while i < 1 {
                print(apply(fn(x) { return x + 1; }, Cell<int>{ value: 4 }.get()));
                i += 1;
            }
        }
        ",
        ),
        "block_locals.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    assert!(
        out.functions.iter().any(|f| f.name == "Cell<int>#get"),
        "the method lowered in the loop body outlives it"
    );
    assert!(
        out.functions.iter().any(|f| f.name.starts_with("<anon>")),
        "so does the closure the loop body declared"
    );
    // The saved-register table is keyed by recursive call site, and this
    // program has none, so it stays empty however many functions the block
    // hoisted.
    assert!(
        out.callsite_registers.is_empty(),
        "a program without a recursive call saves no registers"
    );
}

/// The saved-register table is keyed by recursive call site, not by function. A
/// recursive call site takes an entry while the callee is still compiling, so a
/// table that also carried one entry per function would hand a function
/// declared later an index naming a call site rather than its own list. Every
/// entry belongs to exactly one `SaveFrame`.
#[test]
pub fn the_saved_register_table_is_one_entry_per_recursive_call_site() {
    let out = compile(
        String::from(
            "
        fn fib(n) {
            if n < 2 { return n; }
            return fib(n - 1) + fib(n - 2);
        }
        fn poly(a, b, c) {
            let d = a * b;
            let e = d + c;
            let f = e * e;
            let g = f - a;
            return g + b + c + d + e;
        }
        fn main() { print(fib(10)); print(poly(2, 3, 4)); }
        ",
        ),
        "callsite_registers.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    let mut callsites: Vec<u16> = out
        .instructions
        .iter()
        .filter_map(|i| match i {
            Instr::SaveFrame(_, _, callsite_id) => Some(*callsite_id),
            _ => None,
        })
        .collect();
    assert!(
        !callsites.is_empty(),
        "the recursive calls must each save a frame"
    );
    callsites.sort_unstable();
    let entries: Vec<u16> = (0..out.callsite_registers.len() as u16).collect();
    assert_eq!(
        callsites, entries,
        "every entry is one call site's, and no function holds one"
    );
}

/// A closure's id is what a call site's specialisation cache is keyed on, so an
/// id is never handed out twice. Two closure literals passed to one function
/// from two blocks each run their own body.
#[test]
pub fn two_blocks_closures_each_run_their_own_body() {
    run_and_check_registers!(
        "
        fn apply(f, x) { return f(x); }
        fn main() {
            if true { print(apply(fn(n) { return n * 2; }, 5)); }
            print(apply(fn(n) { return n * 7; }, 2));
        }
        ",
        14.into()
    );
}

#[test]
pub fn nested_generic_instantiation() {
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        impl Cell<T> { fn get(self) -> T { return self.value; } }
        fn main() {
            let inner = Cell<int>{ value: 4 };
            let outer = Cell<Cell<int>>{ value: inner };
            print(outer.get().get());
        }
        ",
        4.into()
    );
}

#[test]
pub fn unused_type_parameter_is_legal() {
    // `Signal<T>` never stores a `T`; the parameter only picks the impl block.
    run_and_check_registers!(
        "
        struct Signal<T> { name: string }
        fn signal<T>(name: string) { return Signal<T>{ name: name }; }
        impl Signal<int> { fn get(self) -> int { return 7; } }
        impl Signal<float> { fn get(self) -> float { return 1.5; } }
        fn main() {
            let count = signal<int>(\"count\");
            print(count.get());
        }
        ",
        7.into()
    );
}

#[test]
pub fn explicit_type_arguments_pick_the_specialisation() {
    // Both calls pass one string, so the argument types alone cannot tell the
    // two specialisations apart; the type argument does.
    run_and_check_registers!(
        "
        struct Signal<T> { name: string }
        fn signal<T>(name: string) { return Signal<T>{ name: name }; }
        impl Signal<int> { fn label(self) -> string { return \"int\"; } }
        impl Signal<float> { fn label(self) -> string { return \"float\"; } }
        fn main() {
            let a = signal<int>(\"a\");
            let b = signal<float>(\"b\");
            print(a.label().len() + b.label().len());
        }
        ",
        8.into()
    );
}

#[test]
pub fn generic_and_concrete_impl_on_one_type() {
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        impl Cell<T> { fn get(self) -> T { return self.value; } }
        impl Cell<int> { fn double(self) -> int { return self.value * 2; } }
        fn main() {
            let c = Cell<int>{ value: 5 };
            print(c.get() + c.double());
        }
        ",
        15.into()
    );
}

#[test]
pub fn a_method_takes_explicit_type_arguments() {
    // `get` is bound by the impl header (T = int), `tagged` binds its own U.
    run_and_check_registers!(
        "
        struct Box<T> { value: T }
        impl Box<T> {
            fn get(self) -> T { return self.value; }
            fn tagged<U>(self, extra: U) -> U { return extra; }
        }
        fn main() {
            let b = Box<int>{ value: 7 };
            print(b.get() + b.tagged<string>(\"hi\").len());
        }
        ",
        9.into()
    );
}

#[test]
pub fn a_method_type_argument_left_off_is_inferred() {
    run_and_check_registers!(
        "
        struct Box<T> { value: T }
        impl Box<T> {
            fn tagged<U>(self, extra: U) -> U { return extra; }
        }
        fn main() {
            let b = Box<int>{ value: 1 };
            print(b.tagged(\"abc\").len() + b.tagged(4));
        }
        ",
        7.into()
    );
}

#[test]
pub fn a_method_type_argument_picks_the_specialisation() {
    // `code_of` takes no arguments, so nothing at the call site pins `U` but
    // the type argument, and each instantiation reaches a different impl.
    run_and_check_registers!(
        "
        struct Kind<T> { n: int }
        impl Kind<int> { fn code(self) -> int { return 1; } }
        impl Kind<string> { fn code(self) -> int { return 2; } }
        struct Maker<T> { seed: T }
        impl Maker<T> {
            fn code_of<U>(self) -> int { return Kind<U>{ n: 0 }.code(); }
        }
        fn main() {
            let m = Maker<int>{ seed: 1 };
            print(m.code_of<int>() * 10 + m.code_of<string>());
        }
        ",
        12.into()
    );
}

/// A type parameter no argument pins and no type argument names is `any`, so a
/// bare `signal("x")` builds a `Signal<any>` and the concrete `impl Signal<any>`
/// applies. Leaving the type argument off is never an error, so the body has to
/// resolve `T` somehow, and `any` is what an unpinned parameter means.
#[test]
pub fn a_type_parameter_no_argument_names_is_any() {
    run_and_check_registers!(
        "
        struct Signal<T> { name: string }
        fn signal<T>(name: string) -> Signal<T> { return Signal<T>{ name: name }; }
        impl Signal<any> { fn get(self) -> int { return self.name.len(); } }
        fn main() {
            print(signal(\"abc\").get() + signal<any>(\"de\").get());
        }
        ",
        5.into()
    );
}

/// The same for a method's own type parameter: `wrap` names `U` in its body but
/// nothing at the call site pins it, so `b.wrap()` builds a `Tag<any>`.
#[test]
pub fn a_method_type_parameter_no_argument_names_is_any() {
    run_and_check_registers!(
        "
        struct Holder<T> { value: T }
        struct Tag<U> { label: string }
        impl Holder<int> {
            fn wrap<U>(self) -> Tag<U> { return Tag<U>{ label: \"nn\" }; }
        }
        impl Tag<any> { fn width(self) -> int { return self.label.len(); } }
        fn main() {
            let b = Holder{ value: 1 };
            print(b.wrap().width() + b.wrap<any>().width());
        }
        ",
        4.into()
    );
}

/// A generic struct literal written without type arguments takes each parameter
/// from the field declared with it, and a parameter no field declares is `any`,
/// so `Signal{ name: \"x\" }` is a `Signal<any>` too.
#[test]
pub fn a_struct_literal_parameter_no_field_pins_is_any() {
    run_and_check_registers!(
        "
        struct Signal<T> { name: string }
        impl Signal<any> { fn get(self) -> int { return self.name.len(); } }
        fn main() {
            print(Signal{ name: \"abcd\" }.get());
        }
        ",
        4.into()
    );
}

/// A declared return type that mentions a parameter the call did not name stays
/// un-pinned: what the body hands back is inferred rather than checked against
/// `any`, so `make()` is not refused for returning `int[]` where `T[]` reads as
/// `any[]`.
#[test]
pub fn an_unnamed_parameter_leaves_a_return_annotation_unpinned() {
    run_and_check_registers!(
        "
        fn make<T>() -> T[] { return [1, 2]; }
        fn main() {
            print(make().len() + make<int>().len());
        }
        ",
        4.into()
    );
}

#[test]
pub fn generic_enum_variant_construction() {
    run_and_check_registers!(
        "
        enum Slot<T> { Filled(T), Empty }
        fn unwrap(s: Slot<int>) -> int {
            match s {
                Filled(x) => { return x; }
                _ => { return 0; }
            }
        }
        fn main() {
            print(unwrap(Slot<int>::Filled(9)) + unwrap(Slot<int>::Empty));
        }
        ",
        9.into()
    );
}

/// A constructor written without a type argument instantiates the enum at what
/// its payload holds, so a struct put into a `Some` comes back out as that
/// struct.
#[test]
pub fn a_bare_constructor_binds_the_payload_type() {
    run_and_check_registers!(
        "
        enum Opt<T> { Some(T), None }
        struct P { x: int }
        fn find(n: int) {
            if n > 0 {
                return Some(P { x: n });
            }
            return None;
        }
        fn main() {
            match find(4) {
                Some(p) => { print(p.x); }
                None => { print(0); }
            }
        }
        ",
        4.into()
    );
}

/// The instantiation follows the payload, not whichever instantiation happens
/// to be registered first.
#[test]
pub fn bare_constructors_reach_their_own_instantiation() {
    run_and_check_registers!(
        "
        enum Opt<T> { Some(T), None }
        struct P { x: int }
        fn boxed(p: P) -> Opt<P> { return Some(p); }
        fn counted(n: int) -> Opt<int> { return Some(n); }
        fn main() {
            let total = 0;
            match boxed(P { x: 3 }) { Some(p) => { total += p.x; } None => { } }
            match counted(5) { Some(n) => { total += n; } None => { } }
            print(total);
        }
        ",
        8.into()
    );
}

/// A constructor names only the parameters its payload mentions, so the two
/// sides a body returns are the one type between them.
#[test]
pub fn the_two_sides_of_a_result_merge_into_one_type() {
    run_and_check_registers!(
        "
        enum Res<T, E> { Ok(T), Err(E) }
        fn parse(text: string) {
            if text.is_int() {
                return Ok(int(text));
            }
            return Err(\"not a number\");
        }
        fn main() {
            let total = 0;
            match parse(\"7\") { Ok(n) => { total += n; } Err(e) => { total += e.len(); } }
            match parse(\"x\") { Ok(n) => { total += n; } Err(e) => { total += e.len(); } }
            print(total);
        }
        ",
        19.into()
    );
}

/// `None` names no payload, so it is an `Opt<any>`, and that goes where a named
/// instantiation is expected.
#[test]
pub fn a_payloadless_variant_goes_where_an_instantiation_is_expected() {
    run_and_check_registers!(
        "
        enum Opt<T> { Some(T), None }
        struct P { x: int }
        fn take(o: Opt<P>) -> int {
            match o {
                Some(p) => { return p.x; }
                None => { return -1; }
            }
        }
        fn main() {
            print(take(None));
        }
        ",
        (-1).into()
    );
}

/// A field holding a function is called through the same dot a method call
/// uses. The field's type carries the function that went into it, so the call
/// lowers the way `fs[0](x)` does.
#[test]
pub fn a_struct_field_holding_a_function_is_callable() {
    run_and_check_registers!(
        "
        struct Button<T> { label: string, on_press: T }
        fn main() {
            let b = Button { label: \"ok\", on_press: fn(x) { return x * 2; } };
            print(b.on_press(21));
        }
        ",
        42.into()
    );
}

/// A method of that name wins, so adding one never changes which call an
/// existing program makes.
#[test]
pub fn a_method_wins_over_a_field_of_the_same_name() {
    run_and_check_registers!(
        "
        struct S<T> { cb: T }
        impl S<T> { fn cb(self, n) { return 7; } }
        fn main() {
            let s = S { cb: fn(x) { return x * 2; } };
            print(s.cb(21));
        }
        ",
        7.into()
    );
}

/// A field that holds no function is still a missing method, reported against
/// the receiver's type.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn calling_a_field_that_holds_no_function_is_an_error() {
    run!(
        "
        struct S { x: int }
        fn main() {
            let s = S { x: 1 };
            print(s.x(2));
        }
        "
    );
}

#[test]
pub fn generic_type_in_a_struct_field() {
    run_and_check_registers!(
        "
        struct Cell<T> { value: T }
        struct Pair { left: Cell<int>, right: Cell<string> }
        impl Cell<T> { fn get(self) -> T { return self.value; } }
        fn main() {
            let p = Pair{ left: Cell<int>{ value: 6 }, right: Cell<string>{ value: \"hi\" } };
            print(p.left.get() + p.right.get().len());
        }
        ",
        8.into()
    );
}

#[test]
pub fn generic_declared_in_an_imported_module() {
    let dir = std::env::temp_dir().join("candela_generic_import_test");
    let _ = std::fs::create_dir_all(&dir);
    let lib = dir.join("boxes.cdl");
    std::fs::write(
        &lib,
        "struct Boxed<T> { item: T }\n\
         impl Boxed<T> { fn item(self) -> T { return self.item; } }\n\
         fn wrap<T>(x) { return Boxed<T>{ item: x }; }\n",
    )
    .unwrap();
    let main = dir.join("main.cdl");
    std::fs::write(
        &main,
        "import \"./boxes.cdl\";\nfn main() { print(wrap<int>(41).item() + 1); }\n",
    )
    .unwrap();
    let src = std::fs::read_to_string(&main).unwrap();
    let out = compile(
        src,
        main.to_str().unwrap(),
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    let mut arrays = out.pools;
    let mut reg = RegisterFile(out.registers);
    run_vm(
        &out.instructions,
        &mut reg,
        &mut arrays,
        &crate::errors::ErrorCtx {
            instr_src: &out.instr_src,
            sources: &out.sources,
        },
        &out.callsite_registers,
        &[],
        &[],
        &[],
        out.allocated_arg_count,
        out.allocated_call_depth,
        &[],
        &[],
        0,
    );
    assert!(out.instructions.iter().any(|x| {
        if let Instr::Print(tgt) = x {
            reg[(*tgt) as usize] == 42.into()
        } else {
            false
        }
    }));
}

#[test]
pub fn wrong_type_argument_count_on_a_type() {
    let err = compile_diag(
        "struct Cell<T> { value: T }
         fn main() { let c = Cell<int, float>{ value: 1 }; }",
        "generics.cdl",
    )
    .expect_err("two type arguments for one parameter");
    assert_eq!(err.code, "type_argument_count");
}

#[test]
pub fn wrong_type_argument_count_on_a_function() {
    let err = compile_diag(
        "fn id<T>(x) { return x; }
         fn main() { print(id<int, float>(1)); }",
        "generics.cdl",
    )
    .expect_err("two type arguments for one parameter");
    assert_eq!(err.code, "type_argument_count");
}

#[test]
pub fn type_arguments_on_a_plain_type() {
    let err = compile_diag(
        "struct Point { x: int }
         fn main() { let p = Point<int>{ x: 1 }; }",
        "generics.cdl",
    )
    .expect_err("Point has no type parameters");
    assert_eq!(err.code, "type_args_on_plain_type");
}

#[test]
pub fn type_arguments_on_a_plain_function() {
    let err = compile_diag(
        "fn id(x) { return x; }
         fn main() { print(id<int>(1)); }",
        "generics.cdl",
    )
    .expect_err("id has no type parameters");
    assert_eq!(err.code, "type_args_on_plain_function");
}

#[test]
pub fn type_arguments_on_a_method_that_takes_none() {
    let err = compile_diag(
        "struct Box<T> { value: T }
         impl Box<T> { fn get(self) -> T { return self.value; } }
         fn main() { let b = Box<int>{ value: 1 }; print(b.get<int>()); }",
        "generics.cdl",
    )
    .expect_err("get has no type parameters of its own");
    assert_eq!(err.code, "type_args_on_plain_function");
}

#[test]
pub fn type_arguments_on_a_builtin_method() {
    let err = compile_diag(
        "fn main() { let s = \"ab\"; print(s.len<int>()); }",
        "generics.cdl",
    )
    .expect_err("len is built in");
    assert_eq!(err.code, "type_args_on_builtin_method");
}

#[test]
pub fn type_arguments_on_a_builtin_impl_method_that_takes_none() {
    // A method a builtin type gets from an `impl` block is an ordinary
    // function, so a type argument on one that declares no parameters reports
    // the plain-function error, not the builtin-method one.
    let err = compile_diag(
        "impl list { fn second(self) { return self[1]; } }
         fn main() { print([1, 2].second<int>()); }",
        "generics.cdl",
    )
    .expect_err("second has no type parameters");
    assert_eq!(err.code, "type_args_on_plain_function");
}

#[test]
pub fn unknown_type_parameter_in_a_declaration() {
    let err = compile_diag(
        "struct Cell<T> { value: U }
         fn main() { let c = Cell<int>{ value: 1 }; }",
        "generics.cdl",
    )
    .expect_err("U is not a declared type parameter");
    assert_eq!(err.code, "unknown_type_parameter");
}

#[test]
pub fn impl_on_a_generic_type_needs_its_arguments() {
    let err = compile_diag(
        "struct Cell<T> { value: T }
         impl Cell { fn get(self) { return self.value; } }
         fn main() { print(1); }",
        "generics.cdl",
    )
    .expect_err("an impl block must name the type arguments");
    assert_eq!(err.code, "type_argument_count");
}

#[test]
pub fn one_method_defined_by_two_impl_blocks() {
    let err = compile_diag(
        "struct Cell<T> { value: T }
         impl Cell<T> { fn get(self) -> T { return self.value; } }
         impl Cell<int> { fn get(self) -> int { return 0; } }
         fn main() { let c = Cell<int>{ value: 1 }; print(c.get()); }",
        "generics.cdl",
    )
    .expect_err("both blocks define Cell<int>#get");
    assert_eq!(err.code, "function_already_defined");
}

#[test]
pub fn a_generic_type_nested_in_itself_without_end() {
    let err = compile_diag(
        "struct L<T> { next: L<L<T>> }
         fn main() { let x = L<int>{ next: null }; }",
        "generics.cdl",
    )
    .expect_err("no finite set of instantiations");
    assert_eq!(err.code, "generic_instantiation_depth");
}

// ---------------------------------------------------------------------------
// `<` STAYS A COMPARISON
//
// Type arguments have no turbofish, so every one of these has to keep parsing
// as the comparison it has always been.
// ---------------------------------------------------------------------------

#[test]
pub fn less_than_is_still_a_comparison() {
    run_and_check_registers!(
        "
        fn main() {
            let a = 1;
            let b = 2;
            print(a < b);
        }
        ",
        true.into()
    );
}

#[test]
pub fn comparisons_joined_by_and() {
    run_and_check_registers!(
        "
        fn main() {
            let a = 1;
            let b = 2;
            let c = 4;
            let d = 3;
            print(a < b && c > d);
        }
        ",
        true.into()
    );
}

#[test]
pub fn a_parenthesised_comparison_chain() {
    // `(a<b)>(c)` compares a bool with an int, which has always been a type
    // error; what matters is that it is still read as two comparisons.
    let err = compile_diag(
        "fn main() { let a = 1; let b = 2; let c = 3; print((a<b)>(c)); }",
        "generics.cdl",
    )
    .expect_err("bool > int");
    assert_eq!(err.code, "invalid_operation");
}

#[test]
pub fn a_comparison_of_method_results() {
    // The `<` after a call's `)` is not a type-argument position, and the one
    // after `.get` closes on `(` only when a list of types parses in between.
    run_and_check_registers!(
        "
        struct P { x: int }
        impl P { fn get(self) -> int { return self.x; } }
        fn main() {
            let p = P{ x: 2 };
            print(p.get() < 5 && p.get() > 1);
        }
        ",
        true.into()
    );
}

#[test]
pub fn a_comparison_against_an_indexed_element() {
    run_and_check_registers!(
        "
        fn main() {
            let a = 1;
            let xs = [5, 6];
            print(a < xs[0]);
        }
        ",
        true.into()
    );
}

#[test]
pub fn a_comparison_in_a_condition_before_a_block() {
    run_and_check_registers!(
        "
        fn main() {
            let a = 1;
            let b = 2;
            if a < b { print(9); }
        }
        ",
        9.into()
    );
}

#[test]
pub fn comparisons_as_call_arguments() {
    run_and_check_registers!(
        "
        fn both(p, q) { return p && q; }
        fn main() {
            let a = 1;
            let b = 2;
            let c = 4;
            let d = 3;
            print(both(a < b, c > d));
        }
        ",
        true.into()
    );
}

/// A source nested exactly `levels` deep, each level built from `open` and
/// `close` around `core`, dropped into `frame` at its `{}`.
fn nested(frame: &str, open: &str, core: &str, close: &str, levels: usize) -> String {
    frame.replacen(
        "{}",
        &format!("{}{core}{}", open.repeat(levels), close.repeat(levels)),
        1,
    )
}

/// Every construct the parser recurses on stops at the same depth with the
/// same structured error, one level past what compiles. The nesting counted
/// includes the enclosing `main` block and the innermost term, so the source
/// text nests two levels fewer than the cap.
#[test]
pub fn nesting_past_the_limit_is_a_structured_error() {
    use crate::parser::MAX_NESTING_DEPTH;
    let cap = MAX_NESTING_DEPTH as usize;
    let cases: [(&str, &str, &str, &str, usize); 5] = [
        ("fn main() { let x = {}; }", "(", "1", ")", cap - 2),
        ("fn main() { let x = {}; }", "[", "1", "]", cap - 2),
        ("fn main() { let x = {}; }", "-", "1", "", cap - 2),
        // Statement blocks: only `main`'s block is around them.
        ("fn main() { {} }", "{ ", "", " }", cap - 1),
        // Types: a parameter's type stands outside every block.
        (
            "fn f(x: {}) { } fn main() { }",
            "{int: ",
            "int",
            "}",
            cap - 1,
        ),
    ];
    for (frame, open, core, close, fits) in cases {
        let src = nested(frame, open, core, close, fits);
        assert!(
            compile_diag(&src, "nest.cdl").is_ok(),
            "{open}{core}{close} nested {fits} deep compiles: {:?}",
            compile_diag(&src, "nest.cdl")
        );
        let src = nested(frame, open, core, close, fits + 1);
        let err = compile_diag(&src, "nest.cdl").expect_err("one level past the cap is an error");
        assert_eq!(err.code, "nesting_too_deep", "{open}{core}{close}: {err:?}");
        assert_wellformed(&err, &src);
    }
}

/// A macro's expansion is parsed at the depth of its invocation, so a chain
/// of expansions cannot dodge the cap by starting each one from zero.
#[test]
pub fn a_macro_expansion_nests_at_its_invocation_depth() {
    use crate::parser::MAX_NESTING_DEPTH;
    let mut env = MacroEnv::new();
    env.register("deep", |_: &str| Ok(String::from("((1))")));
    // `main`'s block, the parens, the invocation itself, then the expansion's
    // two parens and its literal: one more paren and the literal is past the
    // cap.
    let fits = MAX_NESTING_DEPTH as usize - 5;
    let src = nested("fn main() { let x = {}; }", "(", "deep!(x)", ")", fits);
    let fitting = env.scope(|| compile_diag(&src, "nest.cdl"));
    assert!(fitting.is_ok(), "{fitting:?}");
    let src = nested("fn main() { let x = {}; }", "(", "deep!(x)", ")", fits + 1);
    let err = env
        .scope(|| compile_diag(&src, "nest.cdl"))
        .expect_err("the expansion's own nesting counts");
    assert_eq!(err.code, "nesting_too_deep");
    // Reported against the invocation the reader wrote.
    assert_eq!(err.span.start, src.find("deep!").unwrap());
}

// ---------------------------------------------------------------------------
// INFERENCE SEES A MISTAKE FIRST
//
// A call whose value is used is inferred before anything compiles it, so type
// inference is the first place to meet a receiver, a name, or a collection the
// compile stage would reject. Each program below used to reach an
// `unreachable_unchecked` there: a release build segfaulted and a debug build
// aborted, with no diagnostic either way.
// ---------------------------------------------------------------------------

#[test]
pub fn inference_rejects_an_unknown_method_name() {
    let src = "fn to_x(s: string) -> int { return 1; }\nfn main() { let s = \"1\"; let n = s.to_x(); print(n); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.message, "Cannot find function to_x in this scope");
    assert_eq!(d.code, "unknown_function");
}

#[test]
pub fn inference_rejects_a_receiver_a_builtin_does_not_take() {
    // (source, method, receiver type as the diagnostic renders it)
    let cases = [
        (
            "fn main() { let a = \"x\"; let b = a.abs(); print(b); }",
            "abs",
            "string",
        ),
        (
            "fn main() { let a = 5; let b = a.get(0); print(b); }",
            "get",
            "int",
        ),
        (
            "fn main() { let a = 5; let b = a.keys(); print(b); }",
            "keys",
            "int",
        ),
        (
            "fn main() { let a = 5; let b = a.values(); print(b); }",
            "values",
            "int",
        ),
        (
            "fn main() { let a = 5; let b = a.partition(2); print(b); }",
            "partition",
            "int",
        ),
        (
            "fn main() { let a = 5; let b = a.repeat(2); print(b); }",
            "repeat",
            "int",
        ),
    ];
    for (src, method, received) in cases {
        let d = compile_diag(src, "diag.kl").unwrap_err();
        assert_wellformed(&d, src);
        assert_eq!(d.code, "invalid_object_type", "{src}");
        assert!(
            d.message.starts_with(&format!("Function {method} expects")),
            "{src}: {d:?}"
        );
        assert!(
            d.message
                .ends_with(&format!("but here its type is {received}")),
            "{src}: {d:?}"
        );
    }
}

/// A union receiver is accepted only when every member is, which is the rule
/// the compile stage applies. `int|float` takes neither half of `repeat`.
#[test]
pub fn inference_rejects_a_union_receiver_no_member_takes() {
    let src = "fn main() { let c = true; let s = if c { 1 } else { 2.0 }; let r = s.repeat(2); print(r); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_object_type");
    assert!(
        d.message.ends_with("but here its type is int|float"),
        "{d:?}"
    );
}

/// The mirror of the case above: every member is accepted, so the call stands.
#[test]
pub fn inference_accepts_a_union_receiver_every_member_takes() {
    let src = "fn main() { let c = true; let s = if c { \"ab\" } else { [\"a\"] }; let r = s.repeat(2); print(r); }";
    assert!(compile_diag(src, "diag.kl").is_ok());
}

#[test]
pub fn inference_rejects_indexing_a_value_that_has_no_elements() {
    for src in [
        "fn main() { let a = 5; let b = a[0]; print(b); }",
        "fn main() { let a = 5; let b = a[0..1]; print(b); }",
    ] {
        let d = compile_diag(src, "diag.kl").unwrap_err();
        assert_wellformed(&d, src);
        assert_eq!(d.code, "type_not_indexable", "{src}");
        assert!(d.message.contains("int"), "{src}: {d:?}");
    }
}

/// A function's return type is inferred from its body before anything compiles
/// the body, so a loop over a value nothing can iterate is met here.
#[test]
pub fn inference_rejects_a_loop_over_a_value_that_cannot_be_iterated() {
    let src = "fn f() { for x in 5 { print(x); } }\nfn main() { f(); }";
    let d = compile_diag(src, "diag.kl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "type_not_iterable");
    assert_eq!(&src[d.span], "5");
}

// ---------------------------------------------------------------------------
// WHAT THE TERMINAL SHOWS
//
// A structured diagnostic carries the plain message; the report written to the
// terminal is built separately and colours parts of it. The two have to say
// the same thing, and the colouring has to reach the terminal as colour.
// ---------------------------------------------------------------------------

/// A call that accepts more than one type lists them in colour. Writing the
/// escape placeholders inside a `format_args!` argument printed them as text.
#[test]
pub fn a_report_colours_the_types_a_call_accepts() {
    for (src, listed) in [
        (
            "fn main() { let a = \"x\"; a.abs(); }",
            "type to be int or float but here its type is string",
        ),
        (
            "fn main() { let b = true; print(float(b)); }",
            "of type string or int, but this expression's type is bool",
        ),
    ] {
        let report = compile_report(src, "diag.cdl");
        for placeholder in ["{RESET}", "{BLUE}", "{GREEN}"] {
            assert!(!report.contains(placeholder), "{placeholder} in: {report}");
        }
        assert!(strip_ansi(&report).contains(listed), "{report}");
    }
}

/// A `host` block declares a namespace that has no node in the namespace tree,
/// so resolving a call into it used to fail as an unknown namespace before the
/// unknown-function report could name the callee.
#[test]
pub fn a_call_into_a_host_namespace_names_the_function_it_cannot_find() {
    for src in [
        "host \"lumen\" { int set_title(); }\nfn main() { lumen::set_titel(); }",
        "host \"lumen\" { int set_title(); }\nfn main() { let t = lumen::set_titel(); print(t); }",
    ] {
        let d = compile_diag(src, "diag.cdl").unwrap_err();
        assert_wellformed(&d, src);
        assert_eq!(d.code, "unknown_function_in_namespace", "{src}");
        assert_eq!(
            d.message, "Cannot find function set_titel in namespace lumen",
            "{src}"
        );
        // The declared name is offered as well, which only the report carries.
        let report = strip_ansi(&compile_report(src, "diag.cdl"));
        assert!(
            report.contains("A function with a similar name exists: lumen::set_title"),
            "{report}"
        );
    }
}

/// A path that neither the source nor a `host` block declares is still not a
/// namespace, and says so.
#[test]
pub fn a_call_into_a_namespace_nothing_declares_reports_the_namespace() {
    let src = "fn main() { nope::bar(); }";
    let d = compile_diag(src, "diag.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "unknown_namespace");
    assert_eq!(d.message, "nope is not a valid namespace");
}

/// A statement that is nothing but an expression has no reader for its value.
/// The forms that have no statement arm to compile through are compiled as
/// values anyway and the register is freed again; passing "nobody reads this"
/// down to a literal, a variable or an operator used to trip the register-use
/// assertion instead.
#[test]
pub fn a_statement_that_is_only_an_expression_compiles() {
    run_and_check_registers!(
        "
        struct Point { x: int }
        fn main() {
            let p = Point{x: 5};
            let xs = [1, 2, 3];
            p.x;
            xs;
            xs[0];
            p;
            5;
            1 + 2;
            -1;
            !true;
            1 == 2;
            print(p.x);
        }
        ",
        5.into()
    );
}

/// A bare name nothing declares is an unknown variable, reported against the
/// name itself.
#[test]
pub fn a_statement_that_is_only_an_unknown_name_reports_it() {
    let src = "fn main() { nosuch; }";
    let d = compile_diag(src, "diag.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "unknown_variable");
    assert_eq!(d.message, "Cannot find variable nosuch in this scope");
}

/// A closure reads and writes the variables of the function it is written in:
/// both calls land on the one `total` the caller goes on to print.
#[test]
pub fn a_closure_writes_the_variable_of_the_scope_it_was_written_in() {
    run_and_check_registers!(
        "
        fn main() {
            let total = 0;
            let add = fn(x) { total = total + x; };
            add(3);
            add(4);
            print(total);
        }
        ",
        7.into()
    );
}

/// A captured variable outlives the call that declared it, and each call of the
/// factory makes its own: the first counter reaches 3 while the second is still
/// on 1.
#[test]
pub fn two_counters_from_one_factory_count_apart() {
    run_and_check_registers!(
        "
        fn make() {
            let n = 0;
            return fn() { n = n + 1; return n; };
        }

        fn main() {
            let first = make();
            let second = make();
            let acc = 0;
            acc = acc * 10 + first();
            acc = acc * 10 + first();
            acc = acc * 10 + first();
            acc = acc * 10 + second();
            print(acc);
        }
        ",
        1231.into()
    );
}

/// A closure reads a parameter of the function it is written in, through a
/// higher-order function that calls it from somewhere else entirely.
#[test]
pub fn a_closure_reads_the_parameter_of_the_function_it_sits_in() {
    run_and_check_registers!(
        "
        fn apply_all(f, xs) {
            let out = [];
            for x in xs {
                out.push(f(x));
            }
            return out;
        }

        fn scale(k, xs) {
            return apply_all(fn(x) { return x * k; }, xs);
        }

        fn main() {
            let scaled = scale(3, [1, 2]);
            print(scaled[0] + scaled[1]);
        }
        ",
        9.into()
    );
}

/// A comparator reads the threshold the scope around it holds.
#[test]
pub fn a_comparator_reads_the_threshold_around_it() {
    run_and_check_registers!(
        "
        fn count_matching(f, xs) {
            let n = 0;
            for x in xs {
                if f(x) {
                    n = n + 1;
                }
            }
            return n;
        }

        fn main() {
            let threshold = 2;
            print(count_matching(fn(x) { return x > threshold; }, [1, 2, 3, 4]));
        }
        ",
        2.into()
    );
}

/// A closure inside a closure reads from two levels up: the innermost body sees
/// the outermost parameter and the one in between.
#[test]
pub fn a_closure_inside_a_closure_reads_two_levels_up() {
    run_and_check_registers!(
        "
        fn adder(a) {
            return fn(b) { return fn(c) { return a + b + c; }; };
        }

        fn main() {
            print(adder(1)(20)(300));
        }
        ",
        321.into()
    );
}

/// A closure written in a loop body takes the variable of the turn it was
/// written on, so the closures a loop leaves behind hand back what each turn
/// held rather than what the last one did.
#[test]
pub fn a_closure_in_a_loop_body_takes_that_turn_of_the_loop() {
    run_and_check_registers!(
        "
        fn main() {
            let fs = [];
            for i in 0..3 {
                let doubled = i * 2;
                fs.push(fn() { return i + doubled; });
            }
            print(fs[0]() * 100 + fs[1]() * 10 + fs[2]());
        }
        ",
        36.into()
    );
}

/// A captured list is one list: what the closure pushes onto it the scope that
/// declared it reads back.
#[test]
pub fn a_captured_list_is_mutated_through_the_closure() {
    run_and_check_registers!(
        "
        fn main() {
            let bag = [1];
            let add = fn(x) { bag.push(x); };
            add(2);
            add(3);
            print(bag[0] + bag[1] + bag[2]);
        }
        ",
        6.into()
    );
}

/// The same for a captured struct: the field the closure writes is the field
/// the declaring scope reads.
#[test]
pub fn a_captured_struct_is_mutated_through_the_closure() {
    run_and_check_registers!(
        "
        struct Counter { n: int }

        fn main() {
            let c = Counter { n: 0 };
            let bump = fn() { c.n = c.n + 1; };
            bump();
            bump();
            print(c.n);
        }
        ",
        2.into()
    );
}

/// A closure kept in a list still reads its captures after the call that
/// created it has returned.
#[test]
pub fn a_closure_kept_in_a_list_outlives_the_call_that_made_it() {
    run_and_check_registers!(
        "
        fn make(greeting) {
            let fs = [];
            fs.push(fn(name) { return greeting + name; });
            return fs;
        }

        fn main() {
            let fs = make(\"hi \");
            print(fs[0](\"you\"));
        }
        ",
        crate::data::Data::small_str("hi you")
    );
}

/// Only a captured variable moves into a cell. A function that declares
/// closures keeps every other local in a plain register, so a program with one
/// captured variable allocates exactly one cell and the rest of the body is the
/// code it was before.
#[test]
pub fn an_uncaptured_local_stays_in_a_plain_register() {
    let out = compile(
        String::from(
            "
        fn main() {
            let total = 0;
            let step = 5;
            let scratch = step * 2;
            let add = fn(x) { total = total + x; };
            add(step);
            add(scratch);
            print(total);
        }
        ",
        ),
        "cells.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    let cells = out
        .instructions
        .iter()
        .filter(|instr| matches!(instr, Instr::NewCell(_, _)))
        .count();
    assert_eq!(cells, 1, "only the captured variable takes a cell");
    // The uncaptured locals are read straight out of their registers: nothing
    // loads them from a cell on the way into the call.
    let loads = out
        .instructions
        .iter()
        .filter(|instr| matches!(instr, Instr::LoadCell(_, _)))
        .count();
    assert_eq!(
        loads, 2,
        "the cell is read once in the closure body and once by the print"
    );
}

/// A closure that reads nothing around it carries no environment, so it lowers
/// to the code it did before capture existed: no cell, no environment list.
#[test]
pub fn a_closure_that_captures_nothing_allocates_nothing() {
    let out = compile(
        String::from(
            "
        fn apply(f, n) { return f(n); }
        fn main() {
            print(apply(fn(x) { return x * 2; }, 21));
        }
        ",
        ),
        "nocapture.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    assert!(
        !out.instructions
            .iter()
            .any(|instr| matches!(instr, Instr::NewCell(_, _) | Instr::LoadCell(_, _))),
        "a closure with no captures allocates no cell"
    );
}

/// A list holding two different closures calls the one the index picked.
#[test]
pub fn a_list_of_two_closures_calls_the_one_it_was_indexed_for() {
    run_and_check_registers!(
        "
        fn main() {
            let fs = [fn(x) { return x + 1; }, fn(x) { return x * 10; }];
            print(fs[1](4));
        }
        ",
        40.into()
    );
}

/// The other element of the same list is still the other function.
#[test]
pub fn a_list_of_two_closures_calls_the_first_one_too() {
    run_and_check_registers!(
        "
        fn main() {
            let fs = [fn(x) { return x + 1; }, fn(x) { return x * 10; }];
            print(fs[0](4));
        }
        ",
        5.into()
    );
}

/// A closure bound to a name calls itself by that name.
#[test]
pub fn a_closure_calls_itself_by_the_name_it_is_bound_to() {
    run_and_check_registers!(
        "
        fn main() {
            let fact = fn(n) { if n <= 1 { return 1; } return n * fact(n - 1); };
            print(fact(5));
        }
        ",
        120.into()
    );
}

/// Two closures that call each other through the variables they captured.
#[test]
pub fn two_closures_recurse_through_the_variables_they_captured() {
    run_and_check_registers!(
        "
        fn main() {
            let odd = fn(n) { return false; };
            let even = fn(n) {
                if n == 0 { return true; }
                return odd(n - 1);
            };
            odd = fn(n) {
                if n == 0 { return false; }
                return even(n - 1);
            };
            print(even(10));
        }
        ",
        crate::data::TRUE
    );
}

/// A map of closures dispatches on the key the call went through.
#[test]
pub fn a_map_of_closures_dispatches_on_its_key() {
    run_and_check_registers!(
        "
        fn main() {
            let ops = {\"inc\": fn(x) { return x + 1; }, \"ten\": fn(x) { return x * 10; }};
            print(ops.get(\"ten\")(4));
        }
        ",
        40.into()
    );
}

/// A struct field declared `fn(...)` holds any matching closure, and the dot
/// calls it.
#[test]
pub fn a_field_of_function_type_is_called_through_the_dot() {
    run_and_check_registers!(
        "
        struct Button { label: string, on_press: fn(int) -> int }

        fn main() {
            let b = Button { label: \"ok\", on_press: fn(x) { return x * 2; } };
            print(b.on_press(21));
        }
        ",
        42.into()
    );
}

/// A parameter declared `fn(...)` takes a closure and calls it inside the body.
#[test]
pub fn a_parameter_of_function_type_takes_a_closure() {
    run_and_check_registers!(
        "
        fn apply(f: fn(int) -> int, x: int) -> int { return f(x); }

        fn main() {
            print(apply(fn(y) { return y + 1; }, 41));
        }
        ",
        42.into()
    );
}

/// The same parameter takes the name of a declared function.
#[test]
pub fn a_parameter_of_function_type_takes_a_declared_function() {
    run_and_check_registers!(
        "
        fn apply(f: fn(int) -> int, x: int) -> int { return f(x); }
        fn double(n: int) -> int { return n * 2; }

        fn main() {
            print(apply(double, 21));
        }
        ",
        42.into()
    );
}

/// A closure that hands back something else than the declared type is reported
/// where it was written.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn a_closure_that_does_not_match_the_declared_function_type_is_reported() {
    run!(
        "
        fn apply(f: fn(int) -> int, x: int) -> int { return f(x); }

        fn main() {
            print(apply(fn(y) { return \"no\"; }, 1));
        }
        "
    );
}

/// A closure of the wrong arity is reported the same way.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn a_closure_of_the_wrong_arity_for_a_function_type_is_reported() {
    run!(
        "
        fn apply(f: fn(int) -> int, x: int) -> int { return f(x); }

        fn main() {
            print(apply(fn(a, b) { return a + b; }, 1));
        }
        "
    );
}

/// A `fn(...)` type nests in a list and a map the way any other type does.
#[test]
pub fn a_list_of_function_type_holds_matching_closures() {
    run_and_check_registers!(
        "
        struct Steps { run: (fn(int) -> int)[] }

        fn main() {
            let s = Steps { run: [fn(x) { return x + 1; }, fn(x) { return x * 10; }] };
            print(s.run[1](4));
        }
        ",
        40.into()
    );
}

/// A call the compiler can still settle stays a direct call: no program that
/// never holds a function in a value pays for the ones that do.
#[test]
pub fn a_call_the_compiler_settles_stays_direct() {
    // The list methods come from the standard library in this checkout, not
    // from wherever the test binary happens to sit.
    let mut resolver = crate::compiler::imports::ImportResolver::new();
    resolver.set_lib_dir(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/libs"
    )));
    let out = compile(
        String::from(
            "
        import \"std/list\";

        fn apply(f, n) { return f(n); }
        fn twice(x) { return x * 2; }

        fn main() {
            let double = fn(x) { return x * 2; };
            let fs = [fn(x) { return x + 1; }];
            print(apply(twice, 1), double(2), fs[0](3));
            print([3, 1, 2].sort_by(fn(a, b) { return a < b; }));
            print([1, 2, 3].map(fn(x) { return x * 2; }));
            print([1, 2, 3].filter(fn(x) { return x > 1; }));
        }
        ",
        ),
        "direct.cdl",
        false,
        &resolver,
    );
    assert!(
        !out.instructions
            .iter()
            .any(|instr| matches!(instr, Instr::CallIndirect(_))),
        "a callee the compiler can name is jumped to directly"
    );
}

/// A call through a value the compiler cannot settle is the one that goes
/// indirect.
#[test]
pub fn a_call_through_a_value_goes_indirect() {
    let out = compile(
        String::from(
            "
        fn main() {
            let fs = [fn(x) { return x + 1; }, fn(x) { return x * 10; }];
            print(fs[1](4));
        }
        ",
        ),
        "indirect.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    assert!(
        out.instructions
            .iter()
            .any(|instr| matches!(instr, Instr::CallIndirect(_))),
        "a callee no type names is dispatched on at run time"
    );
}

/// A function value carries one body, so calling it at a second set of
/// argument types is reported rather than running the wrong one.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn a_function_value_called_at_two_argument_types_is_reported() {
    run!(
        "
        fn main() {
            let fs = [fn(x) { return x + x; }, fn(y) { return y; }];
            print(fs[0](2));
            print(fs[0](\"a\"));
        }
        "
    );
}

/// A declared function's name is the function itself where a `let` reads it,
/// and the binding calls what it holds.
#[test]
pub fn a_let_holds_the_function_its_name_means() {
    run_and_check_registers!(
        "
        fn double(x: int) -> int { return x * 2; }

        fn main() {
            let f = double;
            print(f(4));
        }
        ",
        8.into()
    );
}

/// A list of declared function names calls the one the index picked.
#[test]
pub fn a_list_of_declared_functions_calls_the_one_it_was_indexed_for() {
    run_and_check_registers!(
        "
        fn double(x: int) -> int { return x * 2; }
        fn triple(x: int) -> int { return x * 3; }

        fn main() {
            let fs = [double, triple];
            print(fs[1](4));
        }
        ",
        12.into()
    );
}

/// A map value and a struct field take a declared function's name the same way
/// they take a closure.
#[test]
pub fn a_map_and_a_field_hold_a_declared_function_by_name() {
    run_and_check_registers!(
        "
        struct Button { on_press: fn(int) -> int }

        fn double(x: int) -> int { return x * 2; }

        fn main() {
            let ops = {\"double\": double};
            let b = Button { on_press: double };
            print(ops.get(\"double\")(4) + b.on_press(4));
        }
        ",
        16.into()
    );
}

/// A returned name hands the function to the caller, which calls it without an
/// annotation of its own.
#[test]
pub fn a_returned_function_name_is_called_by_the_caller() {
    run_and_check_registers!(
        "
        fn double(x: int) -> int { return x * 2; }

        fn pick() { return double; }

        fn main() {
            let f = pick();
            print(f(21));
        }
        ",
        42.into()
    );
}

/// A body that hands back two functions of the same shape calls the one that
/// came back, not the one the first `return` named.
#[test]
pub fn a_body_returning_two_functions_calls_the_one_that_came_back() {
    run_and_check_registers!(
        "
        fn double(x: int) -> int { return x * 2; }
        fn triple(x: int) -> int { return x * 3; }

        fn pick(flag: bool) {
            if flag { return double; }
            return triple;
        }

        fn main() {
            print(pick(false)(10));
        }
        ",
        30.into()
    );
}

/// A `let` that names a function goes through the value rather than the jump
/// the name used to settle.
#[test]
pub fn a_let_that_names_a_function_calls_through_the_value() {
    let out = compile(
        String::from(
            "
        fn double(x: int) -> int { return x * 2; }

        fn main() {
            let f = double;
            print(f(4));
        }
        ",
        ),
        "let_fn_value.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
    );
    assert!(
        out.instructions
            .iter()
            .any(|instr| matches!(instr, Instr::CallIndirect(_))),
        "a name bound to a function is called through what the binding holds"
    );
}

/// A function whose parameters are inferred is compiled for the types the call
/// through the value asks for.
#[test]
pub fn a_name_bound_to_an_inferred_function_takes_the_types_its_call_names() {
    run_and_check_registers!(
        "
        fn same(x) { return x; }

        fn main() {
            let f = same;
            print(f(\"hello\") == \"hello\");
        }
        ",
        crate::data::TRUE
    );
}

/// One name cannot reach two specialisations through a value, so the second set
/// of argument types is reported.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn a_name_bound_to_a_function_called_at_two_argument_types_is_reported() {
    run!(
        "
        fn same(x) { return x; }

        fn main() {
            let f = same;
            print(f(1));
            print(f(\"hello\"));
        }
        "
    );
}

/// A range loop counts in a register of its own. Counting in the start
/// literal's register advanced the constant every function compiled before
/// the loop reads for that literal.
#[test]
pub fn a_range_loop_leaves_the_literal_it_starts_from_alone() {
    let printed = run_output(
        "
        fn first(xs: int[]) -> int { return xs[0]; }
        fn main() {
            print(first([7, 8, 9]));
            for i in 0..3 { print(first([7, 8, 9])); }
            print(first([7, 8, 9]));
        }
        ",
    );
    assert_eq!(printed, "7\n7\n7\n7\n7\n");
}

/// A range loop that starts at a variable leaves the variable where it was.
#[test]
pub fn a_range_loop_leaves_the_variable_it_starts_from_alone() {
    let printed = run_output(
        "
        fn main() {
            let s = 1;
            for i in s..4 { }
            print(s);
            let a = 2;
            let b = 5;
            for i in a..b { }
            print(a, b);
        }
        ",
    );
    assert_eq!(printed, "1\n2\n5\n");
}

/// A function returns on every path when its try block and its catch both
/// return, so the declared type is met without a return after the block.
#[test]
pub fn a_try_whose_catch_returns_meets_the_declared_type() {
    let printed = run_output(
        "
        fn a(n: int) -> int { try { return n; } catch e { return -1; } }
        fn c(n: int) -> int {
            try { if n == 0 { throw(\"z\"); } return n; }
            catch \"z\" { return 0; }
            catch e { return -1; }
        }
        fn d(n: int) -> int {
            try { if n == 0 { throw(\"z\"); } return n; }
            catch \"z\" { return 0; }
        }
        fn e(n: int) -> int {
            match n {
                0 => { return 1; }
                _ => { throw(\"x\"); }
            }
        }
        fn g(n: int) -> int {
            if n == 0 { return 1; }
            throw(\"x\");
        }
        fn h(n: int) -> int {
            let i = 0;
            loop {
                if i == n { return i; }
                i += 1;
            }
        }
        fn k(n: int) -> int {
            loop {
                if n > 0 { break; }
                n = 1;
            }
            return n;
        }
        fn main() { print(a(1), c(0), c(4), d(0), d(6), e(0), g(0), h(3), k(0)); }
        ",
    );
    assert_eq!(printed, "1\n0\n4\n0\n6\n1\n1\n3\n1\n");
}

/// A body that returns a value on one path and nothing on another, by reaching
/// its end or through a bare `return;`, used to fold the empty path in as
/// `null`, which the annotation check then dropped; the call read whatever the
/// return register held from the call before. The function is reported
/// instead, annotated or not, and a closure the same way.
#[test]
pub fn a_function_that_returns_on_some_paths_only_is_reported() {
    for (name, src) in [
        (
            "b",
            "fn b(n: int) -> int { if n > 0 { return n; } } fn main() { print(b(1)); }",
        ),
        (
            "c",
            "fn c(n) { if n > 0 { return n; } } fn main() { print(c(1)); }",
        ),
        (
            "d",
            "fn d(n) { while n > 0 { return n; } } fn main() { print(d(1)); }",
        ),
        (
            "e",
            "fn e(n) { match n { 1 => { return 1; } 2 => { return 2; } } } fn main() { print(e(1)); }",
        ),
        (
            "g",
            "fn g(n) { if n > 0 { return g(n - 1); } } fn main() { print(g(1)); }",
        ),
        (
            "h",
            "fn h(n) { loop { if n > 0 { break; } return n; } } fn main() { print(h(1)); }",
        ),
        (
            "k",
            "fn k(n: int) -> int { if n > 0 { return n; } return; } fn main() { print(k(1)); }",
        ),
    ] {
        let d = compile_diag(src, "diag.cdl").unwrap_err();
        assert_wellformed(&d, src);
        assert_eq!(d.code, "missing_return", "{}", d.message);
        assert!(
            d.message.contains(&format!("Function {name} ")),
            "{}",
            d.message
        );
    }
    let src = "fn main() { let f = fn(n) { if n > 0 { return n; } }; print(f(1)); }";
    let d = compile_diag(src, "diag.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "missing_return", "{}", d.message);
    assert!(d.message.contains("This function "), "{}", d.message);
    let src = "fn apply(f: fn(int) -> int, x: int) -> int { return f(x); } fn main() { print(apply(fn(n) { if n > 0 { return n; } }, 1)); }";
    let d = compile_diag(src, "diag.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "missing_return", "{}", d.message);
    assert!(d.message.contains("This function "), "{}", d.message);
}

/// A body that returns nothing anywhere still reaches its end freely, and a
/// path that ends in `exit` or in an endless loop is one the function never
/// comes back from.
#[test]
pub fn a_function_returns_on_every_path_it_comes_back_from() {
    let printed = run_output(
        "
        fn a(n: int) { if n > 0 { print(n); } return; }
        fn b(n: int) -> int { if n > 0 { return n; } exit(3); }
        fn c(n: int) -> int { while true { if n > 2 { return n; } n += 1; } }
        fn d(n: int) -> int { loop { if n > 2 { return n; } n += 1; } }
        fn main() { a(1); print(c(0)); print(d(1)); print(b(2)); }
        ",
    );
    assert_eq!(printed, "1\n3\n3\n2\n");
}

/// A builtin method's arguments are compiled after its receiver, and the
/// receiver's register used to be released before that happened. The allocator
/// then handed the same register to an argument that needed one, so
/// `s.items.push(s.n)` wrote `s.n` over the list the push was meant to grow and
/// pushed onto an int instead.
#[test]
pub fn push_keeps_its_receiver_while_the_argument_compiles() {
    run_and_check_registers!(
        "
        struct S { items: int[], n: int }

        fn main() {
            let s = S { items: [1, 2, 3], n: 3 };
            s.items.push(s.n);
            print(str(s.items) == \"[1,2,3,3]\");
        }
        ",
        crate::data::TRUE
    );
}

/// `insert` compiles two arguments after the receiver, so both had to miss it.
#[test]
pub fn insert_keeps_its_receiver_while_its_arguments_compile() {
    run_and_check_registers!(
        "
        struct S { m: {string: int}, text: string, n: int }

        fn main() {
            let s = S { m: {\"a\": 1}, text: \"a\", n: 3 };
            s.m.insert(s.text, s.n);
            print(str(s.m) == \"{\\\"a\\\":3}\");
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn contains_keeps_its_receiver_while_the_argument_compiles() {
    run_and_check_registers!(
        "
        struct S { items: int[], n: int }

        fn main() {
            let s = S { items: [1, 2, 3], n: 3 };
            print(s.items.contains(s.n));
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn find_keeps_its_receiver_while_the_argument_compiles() {
    run_and_check_registers!(
        "
        struct S { items: int[], n: int }

        fn main() {
            let s = S { items: [1, 2, 3], n: 3 };
            print(s.items.find(s.n));
        }
        ",
        2.into()
    );
}

#[test]
pub fn repeat_keeps_its_receiver_while_the_argument_compiles() {
    run_and_check_registers!(
        "
        struct S { items: int[], n: int }

        fn main() {
            let s = S { items: [1, 2, 3], n: 3 };
            print(s.items.repeat(s.n).len());
        }
        ",
        9.into()
    );
}

#[test]
pub fn join_keeps_its_receiver_while_the_argument_compiles() {
    run_and_check_registers!(
        "
        struct S { words: string[], text: string }

        fn main() {
            let s = S { words: [\"x\", \"y\"], text: \"a\" };
            print(s.words.join(s.text) == \"xay\");
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn a_map_get_keeps_its_receiver_while_the_key_compiles() {
    run_and_check_registers!(
        "
        struct S { m: {string: int}, text: string }

        fn main() {
            let s = S { m: {\"a\": 7}, text: \"a\" };
            print(s.m.get(s.text));
        }
        ",
        7.into()
    );
}

/// The argument is a method call on the receiver itself, which is the shape
/// that used to leave the struct in the receiver's register: the remove then
/// read the struct's field count as the list's length.
#[test]
pub fn remove_keeps_its_receiver_while_a_call_argument_compiles() {
    run_and_check_registers!(
        "
        struct S { items: int[], n: int }

        fn main() {
            let s = S { items: [1, 2, 3], n: 3 };
            s.items.remove(s.items.len() - 1);
            print(str(s.items) == \"[1,2]\");
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn a_map_remove_keeps_its_receiver_while_the_key_compiles() {
    run_and_check_registers!(
        "
        struct S { m: {string: int}, text: string }

        fn main() {
            let s = S { m: {\"a\": 1, \"b\": 2}, text: \"a\" };
            s.m.remove(s.text);
            print(str(s.m) == \"{\\\"b\\\":2}\");
        }
        ",
        crate::data::TRUE
    );
}

/// A string method takes the same path, and `replace` compiles two arguments
/// that both read a field of the receiver's struct.
#[test]
pub fn a_string_method_keeps_its_field_receiver() {
    run_and_check_registers!(
        "
        struct T { text: string, sep: string }

        fn main() {
            let t = T { text: \"a-b\", sep: \"-\" };
            print(t.text.replace(t.sep, t.text) == \"aa-bb\");
        }
        ",
        crate::data::TRUE
    );
}

/// An indexed receiver is a scratch register too, and so is an indexed
/// argument.
#[test]
pub fn an_indexed_receiver_keeps_its_register() {
    run_and_check_registers!(
        "
        fn main() {
            let xs = [[1], [2]];
            xs[0].push(xs[1][0]);
            print(str(xs) == \"[[1,2],[2]]\");
        }
        ",
        crate::data::TRUE
    );
}

/// A receiver reached through an index and a field, with arguments that walk
/// the same path.
#[test]
pub fn a_nested_receiver_keeps_its_register() {
    run_and_check_registers!(
        "
        struct Row { m: {string: int}, key: string, n: int }

        fn main() {
            let rows = [Row { m: {\"a\": 1}, key: \"k\", n: 7 }];
            rows[0].m.insert(rows[0].key, rows[0].n);
            print(rows[0].m.get(rows[0].key));
        }
        ",
        7.into()
    );
}

/// A call result is a scratch register the same way a field read is.
#[test]
pub fn a_call_result_receiver_keeps_its_register() {
    run_and_check_registers!(
        "
        struct S { n: int }

        fn nums() { return [1, 2, 3]; }

        fn main() {
            let s = S { n: 3 };
            print(nums().find(s.n));
        }
        ",
        2.into()
    );
}

/// Maps compare by their contents, like every other collection: two maps with
/// the same entries are equal whether or not they are the same map.
#[test]
pub fn two_maps_with_the_same_entries_are_equal() {
    run_and_check_registers!(
        "
        fn main() {
            print({\"a\": 1} == {\"a\": 1});
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn two_maps_with_different_values_are_not_equal() {
    run_and_check_registers!(
        "
        fn main() {
            print({\"a\": 1} != {\"a\": 2});
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn two_maps_of_different_length_are_not_equal() {
    run_and_check_registers!(
        "
        fn main() {
            print({\"a\": 1} == {\"a\": 1, \"b\": 2});
        }
        ",
        crate::data::FALSE
    );
}

/// A map comparison written as a condition is fused into a jump, which is a
/// separate path through the compiler from the one that leaves a bool in a
/// register.
#[test]
pub fn a_map_comparison_reads_as_a_condition() {
    run_and_check_registers!(
        "
        fn main() {
            let x = {\"a\": 1};
            let y = {\"a\": 1};
            let same = 0;
            if x == y {
                same = 1;
            }
            print(same);
        }
        ",
        1.into()
    );
}

#[test]
pub fn a_map_inside_a_map_compares_by_contents() {
    run_and_check_registers!(
        "
        fn main() {
            print({\"a\": {\"b\": 1}} == {\"a\": {\"b\": 1}});
        }
        ",
        crate::data::TRUE
    );
}

/// A field declared `any` is a dynamic slot: it holds a value of whatever type
/// is put in it, and reading it back gives an `any` the downcasts name.
#[test]
pub fn an_any_field_takes_a_value_of_any_type() {
    run_and_check_registers!(
        "
        struct Wrap { v: any }

        fn main() {
            let w = Wrap { v: 5 };
            print(as_int(w.v) + 1);
        }
        ",
        6.into()
    );
}

#[test]
pub fn an_any_field_takes_a_new_value_of_another_type() {
    run_and_check_registers!(
        "
        struct Wrap { v: any }

        fn main() {
            let w = Wrap { v: 5 };
            w.v = \"later\";
            print(as_str(w.v).uppercase() == \"LATER\");
        }
        ",
        crate::data::TRUE
    );
}

#[test]
pub fn an_any_field_of_a_generic_struct_takes_any_value() {
    run_and_check_registers!(
        "
        struct Box<T> { item: T, tag: any }

        fn main() {
            let b = Box<int> { item: 3, tag: \"three\" };
            b.tag = 9;
            print(b.item + as_int(b.tag));
        }
        ",
        12.into()
    );
}

/// The other direction stays reported. A typed field is read back at the type
/// it declares and used without a run-time test, so a dynamic value put into
/// one is a mistake the compiler names where it is written.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn a_dynamic_value_does_not_go_into_a_typed_field() {
    run!(
        "
        struct Wrap { v: any }
        struct Count { n: int }

        fn main() {
            let w = Wrap { v: \"oops\" };
            let c = Count { n: w.v };
            print(c.n);
        }
        "
    );
}

/// A closure calls the function its enclosing function was handed. The body of
/// the closure compiles at the call through the value, by which time the
/// enclosing call has finished and its parameters have left the file scope, so
/// the name has to resolve through the capture the closure carries.
#[test]
pub fn a_closure_calls_the_function_its_enclosing_function_was_handed() {
    run_and_check_registers!(
        "
        fn compose(f, g) {
            return fn(x) { return f(g(x)); };
        }

        fn main() {
            let inc = fn(x) { return x + 1; };
            let dbl = fn(x) { return x * 2; };
            print(compose(inc, dbl)(5));
        }
        ",
        11.into()
    );
}

/// The same when the parameter says what it holds, and the closure calls it
/// twice over.
#[test]
pub fn a_closure_calls_an_annotated_function_parameter() {
    run_and_check_registers!(
        "
        fn twice(f: fn(int) -> int) {
            return fn(x) { return f(f(x)); };
        }

        fn main() {
            let inc = fn(x) { return x + 1; };
            print(twice(inc)(5));
        }
        ",
        7.into()
    );
}

/// A local that took the function reaches the closure the same way the
/// parameter does.
#[test]
pub fn a_closure_calls_a_local_holding_the_function_handed_in() {
    run_and_check_registers!(
        "
        fn main() {
            let inc = fn(x) { return x + 1; };
            let k = fn(h) {
                let hh = h;
                return fn(y) { return hh(y); };
            };
            print(k(inc)(7));
        }
        ",
        8.into()
    );
}

/// A bare payload-less variant pins no type argument, so it is the most open
/// instantiation of its enum. It reaches a payload slot that names one, the way
/// it already reached a function parameter that names one.
#[test]
pub fn a_bare_variant_fills_a_payload_slot_of_a_named_instantiation() {
    run_and_check_registers!(
        "
        enum Tree<T> { Leaf, Node(Tree<T>, T, Tree<T>) }

        fn sum_tree(t: Tree<int>) -> int {
            match t {
                Leaf => { return 0; }
                Node(l, x, r) => { return sum_tree(l) + x + sum_tree(r); }
            }
        }

        fn main() {
            print(sum_tree(Tree<int>::Node(Leaf, 7, Leaf)));
        }
        ",
        7.into()
    );
}

/// The same where the tree is built from constructors nested inside one
/// another, so a named instantiation and a bare variant sit in the same call.
#[test]
pub fn a_nested_generic_variant_constructor_takes_bare_leaves() {
    run_and_check_registers!(
        "
        enum Tree<T> { Leaf, Node(Tree<T>, T, Tree<T>) }

        fn depth(t: Tree<int>) -> int {
            match t {
                Leaf => { return 0; }
                Node(l, x, r) => {
                    let a = depth(l);
                    let b = depth(r);
                    if a > b { return a + 1; }
                    return b + 1;
                }
            }
        }

        fn main() {
            let t = Tree<int>::Node(Tree<int>::Node(Leaf, 1, Leaf), 2, Leaf);
            print(depth(t) * 10 + 0);
        }
        ",
        20.into()
    );
}

/// A payload whose type is settled still has to match: the rule only opens the
/// instantiation whose type arguments nothing pinned.
#[test]
#[should_panic(expected = "explicit panic")]
pub fn a_payload_of_the_wrong_type_is_still_reported() {
    run!(
        "
        enum Tree<T> { Leaf, Node(Tree<T>, T, Tree<T>) }

        fn main() {
            let t = Tree<int>::Node(Leaf, \"two\", Leaf);
        }
        "
    );
}

/// `type` names a struct the way a declaration and a diagnostic name it, with
/// its type arguments and without its fields.
#[test]
pub fn type_names_a_struct_and_its_type_arguments() {
    run_and_check_registers!(
        "
        struct Point { x: int, y: int }
        struct Cell<T> { value: T }

        fn main() {
            print(type(Point { x: 1, y: 2 }) + type(Cell<int> { value: 1 })
                == \"PointCell<int>\");
        }
        ",
        crate::data::TRUE
    );
}

/// The dynamic slot is spelled `any`, the way a program writes it, wherever it
/// appears in the answer.
#[test]
pub fn type_spells_the_dynamic_slot_any() {
    run_and_check_registers!(
        "
        fn main() {
            let xs = as_list([1, 2]);
            print(type(xs[0]) + type(xs) == \"anyany[]\");
        }
        ",
        crate::data::TRUE
    );
}

/// A function reads as its signature does in source: the declared types, and no
/// arrow where it hands nothing back.
#[test]
pub fn type_writes_a_function_as_its_signature() {
    run_and_check_registers!(
        "
        fn id(x: int) -> int { return x; }
        fn greet(name: string) { print(name); }
        fn nothing() { }

        fn main() {
            print(type(id) + type(greet) + type(nothing)
                == \"fn(int) -> intfn(string)fn()\");
        }
        ",
        crate::data::TRUE
    );
}

/// What a declaration leaves out is `any`: an un-annotated parameter, and a
/// return the body settles.
#[test]
pub fn type_writes_an_unannotated_function_with_any() {
    run_and_check_registers!(
        "
        fn add(a, b) { return a + b; }

        fn main() {
            print(type(add) == \"fn(any, any) -> any\");
        }
        ",
        crate::data::TRUE
    );
}

// ---------------------------------------------------------------------------
// Map order.
//
// A map is walked in the order its keys went in. These pin the three rules a
// program can lean on: a first insert appends, an insert over a key already
// there keeps that key's place, and a removed key put back goes to the end.
// ---------------------------------------------------------------------------

/// A literal prints the way it was written, not in any order the hasher picks.
#[test]
pub fn a_map_literal_prints_in_source_order() {
    let printed = run_output(
        "
        fn main() {
            print({\"b\": 1, \"a\": 2, \"c\": 3});
        }
        ",
    );
    assert_eq!(printed, "{\"b\":1,\"a\":2,\"c\":3}\n");
}

/// Each insert of a new key appends, so a `for` loop hands the keys back in
/// the order they were inserted.
#[test]
pub fn iterating_a_map_walks_the_keys_in_insertion_order() {
    let printed = run_output(
        "
        fn main() {
            let m = {};
            m.insert(\"e\", 5);
            m.insert(\"d\", 4);
            m.insert(\"c\", 3);
            m.insert(\"b\", 2);
            m.insert(\"a\", 1);
            for k in m { print(k); }
        }
        ",
    );
    assert_eq!(printed, "e\nd\nc\nb\na\n");
}

/// `keys` and `values` read the same order the loop does, so the two lists
/// line up entry for entry.
#[test]
pub fn map_keys_and_values_come_back_in_insertion_order() {
    let printed = run_output(
        "
        fn main() {
            let m = {};
            m.insert(\"e\", 5);
            m.insert(\"d\", 4);
            m.insert(\"c\", 3);
            print(m.keys());
            print(m.values());
        }
        ",
    );
    assert_eq!(printed, "[\"e\",\"d\",\"c\"]\n[5,4,3]\n");
}

/// Taking a key out and putting it back makes it the newest entry. The entries
/// that stood after the removed one keep their order.
#[test]
pub fn a_reinserted_map_key_goes_to_the_end() {
    let printed = run_output(
        "
        fn main() {
            let m = {\"a\": 1, \"b\": 2, \"c\": 3};
            m.remove(\"a\");
            print(m.keys());
            m.insert(\"a\", 9);
            print(m.keys());
        }
        ",
    );
    assert_eq!(printed, "[\"b\",\"c\"]\n[\"b\",\"c\",\"a\"]\n");
}

/// Writing over a key that is already there replaces the value and leaves the
/// entry where it stood.
#[test]
pub fn overwriting_a_map_key_keeps_its_place() {
    let printed = run_output(
        "
        fn main() {
            let m = {\"a\": 1, \"b\": 2, \"c\": 3};
            m.insert(\"a\", 10);
            print(m.keys());
            print(m.values());
        }
        ",
    );
    assert_eq!(printed, "[\"a\",\"b\",\"c\"]\n[10,2,3]\n");
}

/// A map keyed by ints orders by insertion too; nothing sorts the keys.
#[test]
pub fn an_int_keyed_map_keeps_insertion_order() {
    let printed = run_output(
        "
        fn main() {
            let m = {};
            m.insert(3, 30);
            m.insert(1, 10);
            m.insert(2, 20);
            print(m);
        }
        ",
    );
    assert_eq!(printed, "{3:30,1:10,2:20}\n");
}

/// A parsed json object holds its keys in document order, so writing it back
/// out reproduces the document's key order.
#[test]
pub fn a_json_object_round_trips_with_its_key_order() {
    let printed = run_output(
        "
        fn main() {
            print(json_stringify(json_parse(\"{\\\"z\\\": 1, \\\"m\\\": 2, \\\"a\\\": 3}\")));
        }
        ",
    );
    assert_eq!(printed, "{\"z\":1,\"m\":2,\"a\":3}\n");
}

/// Order is what a walk sees, not what equality tests: two maps holding the
/// same entries are equal however they were built.
#[test]
pub fn two_maps_built_in_different_orders_are_equal() {
    let printed = run_output(
        "
        fn main() {
            let one = {\"x\": 1, \"y\": 2};
            let two = {\"y\": 2, \"x\": 1};
            print(one == two, one != two);
            print(one.keys(), two.keys());
        }
        ",
    );
    assert_eq!(printed, "true\nfalse\n[\"x\",\"y\"]\n[\"y\",\"x\"]\n");
}

// ---------------------------------------------------------------------------
// Operators on your own types.
//
// A method named by an operator symbol is what that operator reaches on a
// receiver of the type the `impl` block names. Resolution is entirely in the
// compiler: the lowered call is an ordinary function call, so none of these
// programs needs an instruction the VM did not already have.
// ---------------------------------------------------------------------------

/// Every binary operator a type may define, on one struct.
#[test]
pub fn a_struct_defines_every_binary_operator() {
    let printed = run_output(
        "
        struct N { v: int }

        impl N {
            fn +(self, other: N) -> N { return N { v: self.v + other.v }; }
            fn -(self, other: N) -> N { return N { v: self.v - other.v }; }
            fn *(self, other: N) -> N { return N { v: self.v * other.v }; }
            fn /(self, other: N) -> N { return N { v: self.v / other.v }; }
            fn %(self, other: N) -> N { return N { v: self.v % other.v }; }
            fn ^(self, other: N) -> N { return N { v: self.v ^ other.v }; }
            fn &(self, other: N) -> N { return N { v: self.v & other.v }; }
            fn |(self, other: N) -> N { return N { v: self.v | other.v }; }
            fn ^^(self, other: N) -> N { return N { v: self.v ^^ other.v }; }
            fn <<(self, other: N) -> N { return N { v: self.v << other.v }; }
            fn >>(self, other: N) -> N { return N { v: self.v >> other.v }; }
        }

        fn main() {
            let a = N { v: 12 };
            let b = N { v: 5 };
            let two = N { v: 2 };
            print((a + b).v, (a - b).v, (a * b).v);
            print((a / b).v, (a % b).v, (b ^ two).v);
            print((a & b).v, (a | b).v, (a ^^ b).v);
            print((a << two).v, (a >> two).v);
        }
        ",
    );
    assert_eq!(printed, "17\n7\n60\n2\n2\n25\n4\n13\n9\n48\n3\n");
}

/// The two prefix operators a type may define. `-` has a binary form as well,
/// and the parameter count tells them apart, so one type carries both.
#[test]
pub fn a_struct_defines_both_prefix_operators() {
    let printed = run_output(
        "
        struct N { v: int }

        impl N {
            fn -(self) -> N { return N { v: -self.v }; }
            fn ~(self) -> N { return N { v: ~self.v }; }
            fn -(self, other: N) -> N { return N { v: self.v - other.v }; }
        }

        fn main() {
            let a = N { v: 5 };
            print((-a).v, (~a).v, (a - N { v: 2 }).v);
        }
        ",
    );
    assert_eq!(printed, "-5\n-6\n3\n");
}

/// `!=` has no method of its own: it is the type's `==` with the answer
/// flipped.
#[test]
pub fn not_equal_comes_from_the_equality_method() {
    let printed = run_output(
        "
        struct Ver { major: int, patch: int }

        impl Ver {
            fn ==(self, other: Ver) -> bool { return self.major == other.major; }
        }

        fn main() {
            let a = Ver { major: 1, patch: 0 };
            let b = Ver { major: 1, patch: 9 };
            let c = Ver { major: 2, patch: 0 };
            print(a == b, a != b);
            print(a == c, a != c);
        }
        ",
    );
    assert_eq!(printed, "true\nfalse\nfalse\ntrue\n");
}

/// `>` and `>=` are the type's `<` and `<=` with the operands swapped, so a
/// type writes two methods and gets four operators.
#[test]
pub fn greater_than_comes_from_the_less_than_method() {
    let printed = run_output(
        "
        struct N { v: int }

        impl N {
            fn <(self, other: N) -> bool { return self.v < other.v; }
            fn <=(self, other: N) -> bool { return self.v <= other.v; }
        }

        fn main() {
            let small = N { v: 1 };
            let big = N { v: 3 };
            print(small < big, big < small);
            print(small > big, big > small);
            print(small >= small, small >= big, big >= small);
        }
        ",
    );
    assert_eq!(printed, "true\nfalse\nfalse\ntrue\ntrue\nfalse\ntrue\n");
}

/// The swap is in the operands, so the right one is evaluated first. A program
/// whose operands have side effects sees that order.
#[test]
pub fn a_derived_comparison_evaluates_the_right_operand_first() {
    let printed = run_output(
        "
        struct N { v: int }

        impl N {
            fn <(self, other: N) -> bool { return self.v < other.v; }
        }

        fn note(log: int[], tag: int) -> N {
            log.push(tag);
            return N { v: tag };
        }

        fn main() {
            let log = [];
            print(note(log, 1) > note(log, 2));
            print(log);
        }
        ",
    );
    assert_eq!(printed, "false\n[2,1]\n");
}

/// A compound assignment is the operator applied to the current value, on
/// every target an assignment takes.
#[test]
pub fn compound_assignment_reaches_an_operator_method() {
    let printed = run_output(
        "
        struct N { v: int }
        struct Holder { n: N }

        impl N {
            fn +(self, other: N) -> N { return N { v: self.v + other.v }; }
            fn *(self, other: N) -> N { return N { v: self.v * other.v }; }
        }

        fn main() {
            let a = N { v: 1 };
            a += N { v: 2 };
            let row = [N { v: 10 }];
            row[0] += N { v: 5 };
            let p = Holder { n: N { v: 100 } };
            p.n *= N { v: 3 };
            print(a.v, row[0].v, p.n.v);
        }
        ",
    );
    assert_eq!(printed, "3\n15\n300\n");
}

/// The other operand takes whatever the method declares, so a vector scales by
/// a plain number.
#[test]
pub fn a_declared_second_parameter_takes_another_type() {
    let printed = run_output(
        "
        struct Vec2 { x: float, y: float }

        impl Vec2 {
            fn *(self, k: float) -> Vec2 { return Vec2 { x: self.x * k, y: self.y * k }; }
        }

        fn main() {
            let v = Vec2 { x: 1.5, y: 2.0 } * 2.0;
            print(v.x, v.y);
        }
        ",
    );
    assert_eq!(printed, "3.0\n4.0\n");
}

/// A second parameter left un-annotated means the other operand is of the
/// receiver's type. Anything else is the operator error it has always been.
#[test]
pub fn an_unannotated_second_parameter_means_the_receivers_type() {
    let printed = run_output(
        "
        struct N { v: int }

        impl N {
            fn +(self, other) -> N { return N { v: self.v + other.v }; }
        }

        fn main() { print((N { v: 1 } + N { v: 2 }).v); }
        ",
    );
    assert_eq!(printed, "3\n");

    let src = "
        struct N { v: int }
        impl N { fn +(self, other) -> N { return N { v: self.v + other.v }; } }
        fn main() { print(N { v: 1 } + 3); }
    ";
    let d = compile_diag(src, "mixed.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
}

/// An `impl` block on a generic type carries its operators into every
/// instantiation.
#[test]
pub fn a_generic_type_defines_an_operator_once() {
    let printed = run_output(
        "
        struct Pair<T> { a: T, b: T }

        impl Pair<T> {
            fn +(self, other: Pair<T>) -> Pair<T> {
                return Pair<T> { a: self.a + other.a, b: self.b + other.b };
            }
        }

        fn main() {
            let nums = Pair<int> { a: 1, b: 2 } + Pair<int> { a: 10, b: 20 };
            let words = Pair<string> { a: \"x\", b: \"y\" } + Pair<string> { a: \"1\", b: \"2\" };
            print(nums.a, nums.b);
            print(words.a, words.b);
        }
        ",
    );
    assert_eq!(printed, "11\n22\nx1\ny2\n");
}

/// An enum takes operators the same way a struct does.
#[test]
pub fn an_enum_defines_an_operator() {
    let printed = run_output(
        "
        enum Bit { Off, On }

        impl Bit {
            fn |(self, other: Bit) -> Bit {
                let answer = other;
                match self {
                    On => { answer = Bit::On; }
                    Off => { answer = other; }
                }
                return answer;
            }
        }

        fn main() {
            print(Bit::Off | Bit::On);
            print(Bit::Off | Bit::Off);
        }
        ",
    );
    assert_eq!(printed, "On\nOff\n");
}

/// `==`, `<` and `<=` answer a question, so each returns a `bool`. The check
/// is at the operator, against what the method actually hands back.
#[test]
pub fn a_comparison_method_returning_another_type_is_refused() {
    let src = "
        struct N { v: int }
        impl N { fn <(self, other: N) -> int { return 1; } }
        fn main() { print(N { v: 1 } < N { v: 2 }); }
    ";
    let d = compile_diag(src, "ret.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "operator_method_return_type");
    assert!(d.message.contains("bool"), "{}", d.message);
}

/// What an operator means on a built-in type is part of the language.
#[test]
pub fn an_operator_on_a_builtin_type_is_refused() {
    let src = "
        impl int { fn +(self, other: int) -> int { return 0; } }
        fn main() { print(1); }
    ";
    let d = compile_diag(src, "builtin.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "operator_method_on_builtin");
    assert!(d.message.contains('+'), "{}", d.message);
}

/// An operator a type cannot define is refused where it is written, and the
/// report says what derives the three comparisons that have no method.
#[test]
pub fn an_operator_a_type_cannot_define_is_refused() {
    for (symbol, derived_from) in [(">", "<"), (">=", "<="), ("!=", "==")] {
        let src = format!(
            "
            struct N {{ v: int }}
            impl N {{ fn {symbol}(self, other: N) -> bool {{ return true; }} }}
            fn main() {{ print(1); }}
            "
        );
        let d = compile_diag(&src, "unknown.cdl").unwrap_err();
        assert_wellformed(&d, &src);
        assert_eq!(d.code, "operator_method_unknown");
        assert!(d.message.contains(derived_from), "{}", d.message);
        assert!(d.message.contains("+ - * / %"), "{}", d.message);
    }

    let src = "
        struct N { v: int }
        impl N { fn !(self) -> bool { return true; } }
        fn main() { print(1); }
    ";
    let d = compile_diag(src, "unknown.cdl").unwrap_err();
    assert_eq!(d.code, "operator_method_unknown");
}

/// An operator takes the operands it has, so the method declares exactly that
/// many parameters.
#[test]
pub fn an_operator_method_with_the_wrong_parameter_count_is_refused() {
    let src = "
        struct N { v: int }
        impl N { fn +(self) -> N { return self; } }
        fn main() { print(1); }
    ";
    let d = compile_diag(src, "arity.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "operator_method_arity");
}

/// The first parameter is the receiver, whose type is the one the `impl` block
/// names.
#[test]
pub fn an_operator_method_receiving_another_type_is_refused() {
    let src = "
        struct N { v: int }
        impl N { fn +(self: int, other: N) -> N { return other; } }
        fn main() { print(1); }
    ";
    let d = compile_diag(src, "receiver.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "operator_method_receiver");
}

/// An operator a type does not define is the operator error it has always
/// been, and the report names the method that would make the expression work.
#[test]
pub fn an_operator_a_type_does_not_define_names_the_method_to_write() {
    let src = "
        struct V { x: int }
        fn main() { print(V { x: 1 } | V { x: 2 }); }
    ";
    let d = compile_diag(src, "missing.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");

    let report = strip_ansi(&compile_report(src, "missing.cdl"));
    assert!(
        report.contains("impl V { fn |(self, other: V) -> ... }"),
        "{report}"
    );
}

/// Only the left operand's type is consulted: there is one spelling of an
/// operator method, so a built-in on the left stays an error.
#[test]
pub fn an_operator_does_not_reflect_onto_the_right_operand() {
    let src = "
        struct V { x: int }
        impl V { fn +(self, other: V) -> V { return other; } }
        fn main() { print(1 + V { x: 2 }); }
    ";
    let d = compile_diag(src, "reflect.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "invalid_operation");
}

/// A type that defines no operator behaves exactly as it did: structs compare
/// field by field and maps are untouched.
#[test]
pub fn a_type_without_operators_compares_the_way_it_always_did() {
    let printed = run_output(
        "
        struct P { x: int, y: int }

        fn main() {
            let a = P { x: 1, y: 2 };
            let b = P { x: 1, y: 2 };
            let c = P { x: 1, y: 3 };
            print(a == b, a != b, a == c);
            print({\"k\": 1} == {\"k\": 1}, {\"k\": 1} != {\"k\": 2});
        }
        ",
    );
    assert_eq!(printed, "true\nfalse\nfalse\ntrue\ntrue\n");
}

/// A method that applies its own operator to `self` calls itself. The
/// recursion check works off the names a body calls, and an operator node
/// records the method it reaches, so the call saves the caller's registers the
/// way any recursive call does. Without that the local read after the call is
/// overwritten by the call's own arguments and the answer is 0.
#[test]
pub fn an_operator_method_may_apply_its_own_operator_to_self() {
    let printed = run_output(
        "
        struct N { n: int }

        impl N {
            fn +(self, other: N) -> N {
                if other.n <= 0 {
                    return N { n: 0 };
                }
                let step = self.n;
                let rest = self + N { n: other.n - 1 };
                return N { n: step + rest.n };
            }
        }

        fn main() { print((N { n: 2 } + N { n: 3 }).n); }
        ",
    );
    assert_eq!(printed, "6\n");
}

/// An operator has one spelling, the operator itself. There is no method-call
/// form of it, and a dot in front of the symbol is a parse error like any
/// other name that is not an identifier.
#[test]
pub fn an_operator_method_has_no_dot_spelling() {
    let src = "
        struct V { x: int }
        impl V { fn +(self, other: V) -> V { return other; } }
        fn main() { print(V { x: 1 }.+(V { x: 2 }).x); }
    ";
    let d = compile_diag(src, "dot.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "unexpected_token");
}

/// The standard library in this checkout, for a test whose program imports
/// from it.
fn std_resolver() -> crate::compiler::imports::ImportResolver {
    let mut resolver = crate::compiler::imports::ImportResolver::new();
    resolver.set_lib_dir(std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/libs"
    )));
    resolver
}

/// [`run_output`] for a program that imports from the standard library.
fn run_output_std(src: &str) -> String {
    let resolver = std_resolver();
    let [debug, release] = [false, true].map(|optimize| {
        CAPTURED_OUTPUT.with(|o| o.borrow_mut().clear());
        let was_capturing = set_capturing(true);
        let result = run_diag_profile_with(src, "out.cdl", optimize, &resolver);
        set_capturing(was_capturing);
        let printed = CAPTURED_OUTPUT.with(|o| o.take());
        result.unwrap_or_else(|d| panic!("{}: {printed}", d.message));
        printed
    });
    assert_eq!(
        debug, release,
        "a release build prints what a debug build does"
    );
    native_agrees(src, "out.cdl", &resolver);
    debug
}

/// A call has the type its function declares, not the type its body happens
/// to return, so a declaration that widens the value is what the call site
/// checks against.
#[test]
pub fn a_call_has_its_declared_return_type() {
    let src = "
        fn g() -> any { return 1; }
        fn main() {
            let xs = [\"a\"];
            xs.push(g());
        }
    ";
    let d = compile_diag(src, "declared.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert!(d.message.contains("any"), "{}", d.message);
    assert!(!d.message.contains("int"), "{}", d.message);
}

/// The call is typed by the declaration whichever reaches the function first,
/// the statement that discards its value or the one that reads it.
#[test]
pub fn a_call_reads_its_declaration_after_an_earlier_call() {
    let src = "
        fn g() -> any { return 1; }
        fn main() {
            g();
            let xs = [\"a\"];
            xs.push(g());
        }
    ";
    let d = compile_diag(src, "declared.cdl").unwrap_err();
    assert!(d.message.contains("any"), "{}", d.message);
}

/// A function held in a value hands back what the function declares.
#[test]
pub fn a_call_through_a_value_has_the_declared_return_type() {
    let src = "
        fn g() -> any { return 1; }
        fn main() {
            let h = g;
            let xs = [\"a\"];
            xs.push(h());
        }
    ";
    let d = compile_diag(src, "declared.cdl").unwrap_err();
    assert!(d.message.contains("any"), "{}", d.message);
}

/// Rows built by a helper declared `{any: any}` share a list with rows that
/// already are, which is what the declaration is for.
#[test]
pub fn a_declared_map_of_any_shares_a_list_with_dynamic_rows() {
    assert_eq!(
        run_output(
            "
            fn row() -> {any: any} { let r = {\"a\": \"b\"}; return r; }

            fn main() {
                let rows = [];
                rows.push(as_map({\"x\": \"y\"}));
                rows.push(row());
                print(rows.len());
            }
            "
        ),
        "2\n"
    );
}

/// A body may return something narrower than a declaration naming `any`.
#[test]
pub fn a_narrower_body_fits_a_declaration_of_any() {
    assert_eq!(
        run_output(
            "
            fn nums() -> any[] { return [1, 2]; }
            fn either(b: bool) -> any[] { if b { return [1]; } return [\"s\"]; }

            fn main() {
                let xs = nums();
                xs.push(\"three\");
                print(xs.len(), either(true).len(), either(false).len());
            }
            "
        ),
        "3\n1\n1\n"
    );
}

/// A declared type still refuses a body returning something it does not hold.
#[test]
pub fn a_body_wider_than_its_declaration_is_refused() {
    let src = "
        fn f() -> int[] { return [\"s\"]; }
        fn main() { f(); }
    ";
    let d = compile_diag(src, "declared.cdl").unwrap_err();
    assert!(d.message.contains("expected int[]"), "{}", d.message);
    assert!(d.message.contains("string[]"), "{}", d.message);
}

/// A recursive call reads the declared type while the body is still being
/// worked out, so the result takes part in arithmetic.
#[test]
pub fn a_recursive_call_has_its_declared_return_type() {
    assert_eq!(
        run_output(
            "
            fn fact(n: int) -> int {
                if n < 2 { return 1; }
                return n * fact(n - 1);
            }

            fn main() { print(fact(5) + 1); }
            "
        ),
        "121\n"
    );
}

/// A declaration naming a type parameter the call leaves unbound keeps the
/// body's type; one the call binds is read with the binding.
#[test]
pub fn a_generic_declaration_types_the_call_once_bound() {
    assert_eq!(
        run_output(
            "
            fn first<T>(xs: T[]) -> T { return xs[0]; }
            fn empty() -> int[] { return []; }

            fn main() {
                let xs = [3, 4];
                print(first([1, 2]) + 1, first<int>(xs) + 1);
                let l = empty();
                l.push(1);
                print(l);
            }
            "
        ),
        "2\n4\n[1]\n"
    );
}

/// Generic library functions and methods keep their types at the call site.
#[test]
pub fn std_generics_keep_their_call_types() {
    assert_eq!(
        run_output_std(
            "
            import \"std/option\";
            import \"std/set\" as set;

            fn main() {
                print(Some(5).unwrap() + 1);
                let s = set::new();
                s.add(1);
                print(s.len());
            }
            "
        ),
        "6\n1\n"
    );
}

/// A union declaration types the call as the union, which arithmetic refuses.
#[test]
pub fn a_declared_union_types_the_call_as_the_union() {
    let src = "
        fn f() -> int | string { return 1; }
        fn main() { print(f() + 1); }
    ";
    let d = compile_diag(src, "declared.cdl").unwrap_err();
    assert!(d.message.contains("int|string"), "{}", d.message);
}

/// An `any` value returned from a function declared `int` is checked on the
/// way out: the wrong value raises the catchable `bad_downcast`, the right one
/// passes through.
#[test]
pub fn returning_any_from_a_declared_type_is_checked() {
    assert_eq!(
        run_output(
            "
            fn g(s: string) -> int { return json_parse(s); }
            fn m(s: string) -> {string: string} { return json_parse(s); }

            fn main() {
                print(g(\"4\") + 1);
                try { print(g(\"\\\"s\\\"\")); } catch err { print(err); }
                try { print(m(\"[1]\")); } catch err { print(err); }
            }
            "
        ),
        "5\nbad_downcast\nbad_downcast\n"
    );
}

/// Runs the compile `candela build` does in both profiles, gathering the
/// warnings it raises, and checks that the two profiles raise the same ones
/// and export the same functions. Hands back the debug profile's result.
fn checked_with_warnings(
    src: &str,
    filename: &str,
) -> (Vec<candela_vm::artifact::ExportImage>, Vec<Diagnostic>) {
    let [debug, release] = [false, true].map(|optimize| {
        crate::warnings::collect_warnings(|| {
            crate::trampoline::compile_checked_profile(
                String::from(src),
                filename,
                &crate::compiler::imports::ImportResolver::new(),
                optimize,
            )
            .1
        })
    });
    assert_eq!(
        debug.1, release.1,
        "a release build warns the way a debug build does"
    );
    let names = |exports: &[candela_vm::artifact::ExportImage]| {
        exports.iter().map(|e| e.name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(
        names(&debug.0),
        names(&release.0),
        "a release build exports what a debug build does"
    );
    debug
}

/// A function nothing in the program calls, with a bare parameter, is one a
/// host calls by name. Its body is checked at `any`, and a body that does not
/// compile there costs the entry point, not the build.
#[test]
pub fn a_host_called_bare_parameter_warns_and_the_build_goes_on() {
    let src = "fn on_click(id) { let n = 1; n.uppercase(); }\nfn main() {}\n";
    let (exports, warnings) = checked_with_warnings(src, "click.cdl");
    assert!(exports.iter().all(|e| e.name != "on_click"));
    assert_eq!(warnings.len(), 2, "{warnings:?}");

    assert_eq!(warnings[0].code, "unannotated_host_parameter");
    assert!(
        warnings[0].message.contains("id"),
        "{}",
        warnings[0].message
    );
    assert!(
        warnings[0].message.contains("on_click"),
        "{}",
        warnings[0].message
    );
    assert_eq!(warnings[0].span, 3..11, "the warning points at the name");

    assert_eq!(warnings[1].code, "no_host_entry_point");
    assert!(
        warnings[1].message.contains("uppercase"),
        "the reason the body fails travels with the warning: {}",
        warnings[1].message
    );
    let at = src.find("n.uppercase").unwrap();
    assert_eq!(warnings[1].span.start, at);
}

/// A body that compiles at `any` gets its entry point, typed `any` where the
/// parameter is bare and at its declared type where it is annotated.
#[test]
pub fn a_host_called_bare_parameter_gets_an_any_entry_point() {
    let src = "
        fn on_click(id) { print(id); }
        fn h(a: int, b) { print(a, b); }
        fn main() {}
    ";
    let (exports, warnings) = checked_with_warnings(src, "entry.cdl");
    let on_click = exports.iter().find(|e| e.name == "on_click").unwrap();
    assert!(matches!(
        on_click.arg_types[..],
        [crate::rt::DataType::Unknown]
    ));
    let h = exports.iter().find(|e| e.name == "h").unwrap();
    assert!(matches!(
        h.arg_types[..],
        [crate::rt::DataType::Int, crate::rt::DataType::Unknown]
    ));
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|w| w.code == "unannotated_host_parameter")
    );
    assert!(warnings[1].message.contains("Parameter b of h"));
}

/// Only a function a host would be the one to call is warned about: one the
/// program calls is compiled by that call, a generic one by the call that
/// names its types, and methods and closures are never reached by a bare name.
#[test]
pub fn a_function_the_program_calls_is_not_warned_about() {
    let src = "
        struct S { a: int }
        impl S { fn m(self, k) { return k; } }
        fn fib(n) { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); }
        fn same<T>(x) { return x; }
        fn handle(x) { print(x.m(1)); }
        fn main() {
            print(fib(5));
            handle(S { a: 41 });
            let f = fn(v) { return v; };
            print(f(1));
        }
    ";
    let (exports, warnings) = checked_with_warnings(src, "called.cdl");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(exports.is_empty(), "nothing is left for a host to call");
}

/// A bare function in an imported module is the importer's to call, not a
/// host's.
#[test]
pub fn a_bare_function_in_an_imported_module_is_not_warned_about() {
    let dir = std::env::temp_dir().join("candela_bare_import_warning_test");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("lib.cdl"), "fn helper(x) { return x; }\n").unwrap();
    let main = dir.join("main.cdl");
    let src = "import \"./lib.cdl\";\nfn main() {}\n";
    std::fs::write(&main, src).unwrap();
    let (exports, warnings) = checked_with_warnings(src, main.to_str().unwrap());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(exports.is_empty(), "nothing is left for a host to call");
}

/// A file with no `main` is a library, checked on its own or as a package
/// entry: the files that import it call its functions and specialise their
/// bare parameters, so it is not warned about what a host would pass. The
/// same functions in a file with a `main` are a host's to call (#192).
#[test]
pub fn a_library_file_is_not_warned_about_bare_parameters() {
    let library = "fn shout(s) {\n    return s + \"!\";\n}\n";
    let (exports, warnings) = checked_with_warnings(library, "shout.cdl");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(exports.is_empty(), "shout's body does not compile at any");

    let with_main = format!("{library}fn main() {{}}\n");
    let (_, warnings) = checked_with_warnings(&with_main, "shout.cdl");
    let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(codes, ["unannotated_host_parameter", "no_host_entry_point"]);

    // A library body that compiles at `any` still gets its entry point.
    let (exports, warnings) = checked_with_warnings("fn echo(x) { print(x); }\n", "echo.cdl");
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(exports.iter().any(|e| e.name == "echo"));
}

/// A literal's template register is read on every run of the literal, so a
/// register allocated after the literal's last use in the source must not
/// land on it: the next call of the function would clone whatever was left
/// there.
#[test]
pub fn a_value_computed_after_a_struct_loop_leaves_the_template_alone() {
    let src = "
        struct R { c: int }
        fn g(n: int) -> int {
            let rows = [];
            for i in 0..n { rows.push(R { c: i }); }
            return rows.len();
        }
        fn main() { print(g(3)); print(g(5)); print(g(2)); }
    ";
    assert_eq!(run_output(src), "3\n5\n2\n");
}

/// The counter of a later loop is the register the issue first showed on the
/// template in a longer function.
#[test]
pub fn a_second_loop_counter_leaves_the_struct_template_alone() {
    let src = "
        struct R { c: int }
        fn g(n: int) -> int {
            let rows = [];
            for i in 0..n { rows.push(R { c: i }); }
            let s = 0;
            for j in 0..rows.len() { s = s + rows[j].c; }
            return s * 10 + rows.len();
        }
        fn main() { print(g(3)); print(g(4)); print(g(1)); print(g(3)); }
    ";
    assert_eq!(run_output(src), "33\n64\n1\n33\n");
}

/// An enum, a constant list and a map literal in a loop each read a template
/// too.
#[test]
pub fn enum_list_and_map_templates_survive_repeated_calls() {
    let src = "
        enum Shape { Dot, Line(int, int) }
        fn lines(n: int) -> int {
            let xs = [];
            for i in 0..n { xs.push(Shape::Line(i, 7)); }
            let t = 0;
            for j in 0..xs.len() { match xs[j] { Shape::Line(a, b) => { t = t + b; } Shape::Dot => {} } }
            return t;
        }
        fn pairs(n: int) -> int {
            let xs = [];
            for i in 0..n { xs.push([1, 2]); }
            let t = 0;
            for j in 0..xs.len() { t = t + xs[j][1]; }
            return t;
        }
        fn maps(n: int) -> int {
            let xs = [];
            for i in 0..n { xs.push({\"k\": i, \"w\": 3}); }
            let t = 0;
            for j in 0..xs.len() { t = t + xs[j].get(\"w\"); }
            return t;
        }
        fn main() {
            for r in 0..3 { print(lines(2)); print(pairs(3)); print(maps(4)); }
        }
    ";
    assert_eq!(run_output(src), "14\n6\n12\n".repeat(3));
}

/// An inner loop reruns on every turn of the outer one, so its template has to
/// outlive the inner loop even in code that runs once.
#[test]
pub fn an_inner_loop_template_survives_the_outer_loop() {
    let src = "
        struct R { c: int }
        fn main() {
            let total = 0;
            for j in 0..3 {
                let rows = [];
                for i in 0..2 { rows.push(R { c: i + 1 }); }
                let k = rows.len();
                total = total + k + rows[1].c;
            }
            print(total);
        }
    ";
    assert_eq!(run_output(src), "12\n");
}

/// The name `type` answers is a constant as well: freeing its register after
/// the print let a later value overwrite it for the next call.
#[test]
pub fn a_type_name_survives_repeated_calls() {
    let src = "
        fn t(n: int) -> int {
            print(type(n));
            let s = 0;
            for i in 0..n { s = s + i; }
            return s;
        }
        fn main() { print(t(2)); print(t(3)); print(t(2)); }
    ";
    assert_eq!(run_output(src), "int\n1\nint\n3\nint\n1\n");
}

/// A variable written more than once takes the first value it is given
/// straight into its own register, in both profiles: the instruction that
/// works the value out writes it there, and no move follows.
#[test]
pub fn a_reassigned_variable_takes_its_first_value_without_a_move() {
    let src = "
        fn f(a: float, b: float) -> float {
            let m = (a * a + b * b).sqrt();
            m = m * 2.0;
            let s = a * b;
            s = s + m;
            for j in int(a) + 1..10 { s = s + 1.0; }
            return s;
        }
        fn main() { print(f(3.0, 4.0)); }
    ";
    for optimize in [false, true] {
        let out = crate::compiler::compile_profile(
            String::from(src),
            "moves.cdl",
            false,
            &crate::compiler::imports::ImportResolver::new(),
            optimize,
        );
        let moved_on = out.instructions.windows(2).find(
            |pair| matches!(pair[1], Instr::Mov(from, _) if pair[0].get_tgt_id() == Some(from)),
        );
        assert_eq!(
            moved_on, None,
            "a value is worked out where its variable lives (release profile: {optimize})"
        );
    }
    assert_eq!(run_output(src), "28.0\n");
}

/// A first value that already lives somewhere else is copied into the
/// variable's register, since writing the variable must leave that place
/// alone: another variable, a constant, a parameter, or the start of a loop.
#[test]
pub fn a_reassigned_variable_leaves_where_its_first_value_came_from_alone() {
    let src = "
        fn g(p: int) -> int {
            let q = p;
            q = q + 10;
            return p * 100 + q;
        }
        fn main() {
            let a = 5;
            let b = a;
            b = b + 1;
            let c = 5;
            c = c * 3;
            let d = 5;
            let start = a + 1;
            for i in start..8 { d = d + i; }
            print(a, b, c, d, start, g(2));
            let xs = [1, 2];
            let ys = xs;
            ys = [3];
            print(xs, ys);
            for i in 0..2 {
                let t = i * 2;
                t = t + 1;
                print(t);
            }
        }
    ";
    assert_eq!(run_output(src), "5\n6\n15\n18\n6\n212\n[1,2]\n[3]\n1\n3\n");
}

/// Declaring a variable from another one and then writing either leaves the
/// other as it was, in code that runs once as much as in a loop or a function.
#[test]
pub fn writing_a_copied_variable_leaves_the_original_alone() {
    let body = "
        let a = 5;
        let b = a;
        b = 7;
        let c = 1;
        let d = c;
        c += 1;
        let xs = [1];
        let ys = xs;
        xs = [2];
        print(a, b, c, d, xs, ys);
    ";
    let src = format!(
        "fn f(n: int) {{ {body} }}\nfn main() {{ {body} f(1); for k in 0..1 {{ {body} }} }}"
    );
    assert_eq!(run_output(&src), "5\n7\n2\n1\n[2]\n[1]\n".repeat(3));
}

/// Two moves where the second refills the register the first read become one
/// `MovChain`, and rotating values through variables gives the same answer.
#[test]
pub fn rotating_values_through_variables_chains_the_moves() {
    let src = "
        fn fib(n: int) -> int {
            let a = 0;
            let b = 1;
            let c = 0;
            for i in 0..n {
                c = a + b;
                a = b;
                b = c;
            }
            return a;
        }
        fn main() {
            print(fib(10));
            let x = 1;
            let y = 2;
            x = y;
            y = x;
            print(x, y);
        }
    ";
    for optimize in [false, true] {
        let out = crate::compiler::compile_profile(
            String::from(src),
            "chain.cdl",
            false,
            &crate::compiler::imports::ImportResolver::new(),
            optimize,
        );
        assert!(
            out.instructions
                .iter()
                .any(|instr| matches!(instr, Instr::MovChain(..))),
            "the rotation is one instruction (release profile: {optimize})"
        );
    }
    assert_eq!(run_output(src), "55\n2\n2\n");
}

/// A branch that lands on the second of two moves runs only that one, so the
/// pair stays two instructions.
#[test]
pub fn a_move_a_branch_lands_on_is_not_chained() {
    let src = "
        fn step(flag: bool, c: int) -> int {
            let a = 1;
            let b = c - 1;
            if flag {
                a = b;
            }
            b = c;
            return a * 100 + b;
        }
        fn main() {
            print(step(true, 3), step(false, 3));
        }
    ";
    assert_eq!(run_output(src), "203\n103\n");
}

/// `s.f += x` on a float field is one instruction, and it adds what the field
/// held before `x` was worked out. A call in `x` could write the field in
/// between, so that form keeps the read ahead of the call.
#[test]
pub fn adding_to_a_float_field_reads_it_before_the_value() {
    let src = "
        struct P { x: float, n: int }
        fn bump(p: P) -> float {
            p.x = 100.0;
            return 1.0;
        }
        fn main() {
            let p = P { x: 1.5, n: 1 };
            let d = 0.25;
            p.x += d * 2.0;
            print(p.x);
            p.x += bump(p);
            print(p.x);
            p.n += 2;
            print(p.n);
        }
    ";
    for optimize in [false, true] {
        let out = crate::compiler::compile_profile(
            String::from(src),
            "field.cdl",
            false,
            &crate::compiler::imports::ImportResolver::new(),
            optimize,
        );
        let fused = out
            .instructions
            .iter()
            .filter(|instr| matches!(instr, Instr::AddFieldFloat(..)))
            .count();
        assert_eq!(
            fused, 1,
            "only the addition with no call in it is fused (release profile: {optimize})"
        );
    }
    assert_eq!(run_output(src), "2.0\n3.0\n3\n");
}

/// A `for` loop over a slice walks the list itself in the release profile,
/// and sees exactly the elements the slice would have held, pushes onto the
/// walked list included.
#[test]
pub fn a_loop_over_a_slice_walks_the_list_and_sees_the_slice() {
    let src = "
        fn split(xs: int[], pivot: int) -> int[] {
            let small = [];
            let big = [];
            for x in xs[1..xs.len()] {
                if x < pivot { small.push(x); } else { big.push(x); }
                xs.push(x * 10);
            }
            return small + [pivot] + big;
        }
        fn main() {
            let xs = [5, 3, 8, 1, 9, 2];
            print(split(xs, 5));
            print(xs.len());
            let total = 0;
            for y in xs[2..4] { total = total + y; }
            print(total);
            for z in xs[3..3] { total = total + z; }
            print(total);
        }
    ";
    let out = crate::compiler::compile_profile(
        String::from(src),
        "walk.cdl",
        false,
        &crate::compiler::imports::ImportResolver::new(),
        true,
    );
    // The slice is built only on the path a bad range takes, which a jump
    // steps over.
    let slices: Vec<usize> = out
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, instr)| matches!(instr, Instr::GetSliceArray(..)))
        .map(|(at, _)| at)
        .collect();
    assert!(!slices.is_empty());
    for at in slices {
        assert!(
            matches!(out.instructions[at - 2], Instr::Jmp(size) if at - 2 + size as usize > at),
            "the slice at {at} is stepped over"
        );
    }
    assert_eq!(run_output(src), "[3,1,2,5,8,9]\n11\n9\n9\n");
}

/// A body that writes an element, or calls anything, could change what the
/// walk reads, so its loop still builds the slice; and a range the list cannot
/// supply raises the same error, at the same place, in both profiles.
#[test]
pub fn a_loop_over_a_slice_that_could_change_it_copies_and_bad_ranges_raise_alike() {
    let src = "
        fn clobber(ys: int[]) { ys[2] = 700; }
        fn main() {
            let xs = [1, 2, 3, 4];
            for x in xs[0..3] { xs[2] = 100; print(x); }
            for x in xs[0..4] { clobber(xs); print(x); }
            try {
                for x in xs[2..9] { print(x); }
            } catch e {
                print(e);
            }
        }
    ";
    assert_eq!(
        run_output(src),
        "1\n2\n3\n1\n2\n100\n4\nslice_out_of_bounds\n"
    );
    let bad = "
        fn main() {
            let xs = [1, 2, 3];
            let n = 5;
            for x in xs[1..n] { print(x); }
        }
    ";
    let [debug, release] =
        [false, true].map(|optimize| run_diag_profile(bad, "range.cdl", optimize).unwrap_err());
    assert_eq!(debug.code, "slice_out_of_bounds");
    assert_eq!(debug, release);
}

/// A string longer than six bytes that is built at run time finds the map
/// entry an equal literal made, and the reverse, in both profiles (#189).
#[test]
pub fn a_built_string_finds_the_map_key_with_its_text() {
    let src = "
        fn main() {
            let lit = \"abcdefghijabcdefghijabcdefghij\";
            let r = \"\";
            for i in 0..3 {
                r = r + \"abcdefghij\";
            }
            let joined = [\"abcdefghij\", \"abcdefghij\", \"abcdefghij\"].join(\"\");
            print(r == lit, joined == lit);
            let m = {\"abcdefghijabcdefghijabcdefghij\": 1};
            print(m.contains(r), m.contains(joined));
            let n = {};
            n.insert(lit, 1);
            n.insert(joined, 2);
            print(n.len(), n.get(lit));
            for k in n.keys() { print(m.get(k)); }
        }
    ";
    assert_eq!(run_output(src), "true\ntrue\ntrue\ntrue\n1\n2\n1\n");
}

/// A field that holds a function takes, in an assignment, any function its
/// struct literal would: an anonymous one, one bound to a variable, or a
/// declared one by name (#190).
#[test]
pub fn assigning_a_function_to_a_function_field_works_like_the_literal() {
    let src = "
        struct Button {
            label: string,
            on_press: fn(int) -> int,
        }
        fn inc(x: int) -> int { return x + 1; }
        fn main() {
            let b = Button { label: \"ok\", on_press: fn(x) { return x * 2; } };
            print(b.on_press(21));
            b.on_press = fn(x) { return x + 1; };
            print(b.on_press(21));
            let f = fn(x) { return x - 1; };
            b.on_press = f;
            print(b.on_press(21));
            b.on_press = inc;
            print(b.on_press(21));
        }
    ";
    assert_eq!(run_output(src), "42\n22\n20\n22\n");
    let wrong = "
        struct Button { on_press: fn(int) -> int }
        fn main() {
            let b = Button { on_press: fn(x) { return x * 2; } };
            b.on_press = 5;
        }
    ";
    let d = compile_diag(wrong, "field.cdl").unwrap_err();
    assert_eq!(
        d.message,
        "Field on_press in struct Button expects type fn(int) -> int, but this expression is of type int"
    );
    // The literal refuses the same value: it used to take it and crash on
    // the call.
    let literal = "
        struct Button { on_press: fn(int) -> int }
        fn main() {
            let b = Button { on_press: 5 };
            print(b.on_press(2));
        }
    ";
    assert_eq!(
        compile_diag(literal, "field.cdl").unwrap_err().message,
        d.message
    );
}

/// An `any` value leaving a function declared to return a struct, an enum, a
/// function or a union is checked on the way out, like a scalar one: a value
/// of the declared type passes, anything else raises the catchable
/// `bad_downcast` (#191).
#[test]
pub fn an_any_returned_as_a_struct_enum_function_or_union_is_checked() {
    let src = "
        struct P { x: int }
        struct Q { x: int }
        enum E { A(int), B }
        struct Slot { v: any }
        fn to_p(s: Slot) -> P { return s.v; }
        fn to_e(s: Slot) -> E { return s.v; }
        fn to_f(s: Slot) -> fn(int) -> int { return s.v; }
        fn to_u(s: Slot) -> int|string { return s.v; }
        fn main() {
            print(to_p(Slot { v: P { x: 3 } }).x);
            match to_e(Slot { v: E::A(4) }) {
                E::A(n) => { print(n); }
                E::B => { print(0); }
            }
            print(to_u(Slot { v: \"six\" }));
            print(to_u(Slot { v: 7 }));
            try { to_p(Slot { v: Q { x: 1 } }); } catch e { print(e); }
            try { to_p(Slot { v: 5 }); } catch e { print(e); }
            try { to_e(Slot { v: \"x\" }); } catch e { print(e); }
            try { to_f(Slot { v: 7 }); } catch e { print(e); }
            try { to_u(Slot { v: 1.5 }); } catch e { print(e); }
        }
    ";
    assert_eq!(
        run_output(src),
        "3\n4\nsix\n7\n".to_owned() + &"bad_downcast\n".repeat(5)
    );
    let uncaught = "
        enum E { A(int), B }
        struct Slot { v: any }
        fn to_u(s: Slot) -> E|int { return s.v; }
        fn main() { to_u(Slot { v: \"x\" }); }
    ";
    let [debug, release] = [false, true]
        .map(|optimize| run_diag_profile(uncaught, "downcast.cdl", optimize).unwrap_err());
    assert_eq!(debug.code, "bad_downcast");
    assert_eq!(debug.message, "Cannot read this string value as E|int");
    assert_eq!(debug, release);
}

/// A function that reaches an `any` and comes back out as a `fn(...)` type is
/// called, not jumped past: one that annotates every parameter has a body at
/// those types, and one with a bare parameter or a type parameter, which
/// nothing says how to compile, raises `bad_downcast` on the way out of the
/// `any` rather than running whatever its first slot pointed at (#194).
#[test]
pub fn a_function_through_an_any_is_called() {
    let src = "
        struct Slot { v: any }
        fn double(x: int) -> int { return x * 2; }
        fn fact(n: int) -> int { if n <= 1 { return 1; } return n * fact(n - 1); }
        fn show(x: any) { print(x); }
        fn to_any(x: any) -> any { return x; }
        fn get(s: Slot) -> fn(int) -> int { return s.v; }
        fn back(a: any) -> fn(any) { return a; }
        fn main() {
            let f = get(Slot { v: double });
            print(f(4));
            print(get(Slot { v: fact })(5));
            let g = back(to_any(show));
            g(\"hi\");
            let h = show;
            h(3);
            print(Slot { v: double });
        }
    ";
    assert_eq!(run_output(src), "8\n120\nhi\n3\nSlot {v:<fn>}\n");

    let unannotated = "
        fn id<T>(x: T) -> T { return x; }
        fn twice(x) { return x * 2; }
        struct Slot { v: any }
        fn get(s: Slot) -> fn(int) -> int { return s.v; }
        fn main() {
            let k = 10;
            let add = fn(x) { return x + k; };
            try { get(Slot { v: twice }); } catch e { print(e); }
            try { get(Slot { v: id }); } catch e { print(e); }
            try { get(Slot { v: add }); } catch e { print(e); }
            try { get(Slot { v: fn(x) { return x; } }); } catch e { print(e); }
        }
    ";
    assert_eq!(run_output(unannotated), "bad_downcast\n".repeat(4));
    let uncaught = "
        fn twice(x) { return x * 2; }
        struct Slot { v: any }
        fn get(s: Slot) -> fn(int) -> int { return s.v; }
        fn main() { get(Slot { v: twice })(1); }
    ";
    let d = run_diag(uncaught, "through_any.cdl").unwrap_err();
    assert_eq!(d.code, "bad_downcast");
    assert_eq!(
        d.message,
        "Cannot read this function value as function with annotated parameters"
    );
}

// ---------------------------------------------------------------------------
// STRUCT UPDATE SYNTAX AND DEFAULTS
// ---------------------------------------------------------------------------

const OPTS: &str = "
struct Inner {
    n: int,
    tags: string[],
}

struct Opts {
    cwd: string = \"here\",
    retries: int = 3,
    verbose: bool,
    ratio: float,
    env: {string: string},
    inner: Inner,
    note: any,
}

fn show(o: Opts) {
    print(o.cwd + \" \" + str(o.retries) + \" \" + str(o.verbose) + \" \" + str(o.ratio) + \" \"
        + str(o.env.len()) + \" \" + str(o.inner.n) + \" \" + str(o.inner.tags.len()) + \" \" + str(o.note));
}
";

/// The fields a literal does not write come from the base after `..`, and the
/// base is left as it was.
#[test]
pub fn struct_update_takes_the_unwritten_fields_from_the_base() {
    let src = format!(
        "{OPTS}
fn main() {{
    let a = Opts {{ cwd: \"a\", retries: 1, verbose: true, ratio: 0.5, env: {{}}, inner: Inner {{ n: 2, tags: [] }}, note: 4 }};
    let b = Opts {{ retries: 9, ..a }};
    show(b);
    show(a);
    let c = Opts {{ ..b }};
    show(c);
}}
"
    );
    assert_eq!(
        run_output(&src),
        "a 9 true 0.5 0 2 0 4\na 1 true 0.5 0 2 0 4\na 9 true 0.5 0 2 0 4\n"
    );
}

/// A struct's default takes each field's declared value, or the empty value of
/// its type, a nested struct's own default included; each default builds new
/// lists and maps, so two defaults share none.
#[test]
pub fn a_struct_default_fills_every_field() {
    let src = format!(
        "{OPTS}
fn main() {{
    show(Opts::default());
    let d = Opts {{ cwd: \"x\", ..Default::default() }};
    show(d);
    d.inner.tags.push(\"t\");
    d.env.insert(\"k\", \"v\");
    show(Opts::default());
    show(Opts {{ inner: Inner {{ n: 7, ..Default::default() }}, ..Default::default() }});
}}
"
    );
    assert_eq!(
        run_output(&src),
        "here 3 false 0.0 0 0 0 null\nx 3 false 0.0 0 0 0 null\n\
         here 3 false 0.0 0 0 0 null\nhere 3 false 0.0 0 7 0 null\n"
    );
}

/// `Default::default()` is the default of the struct its position names: a
/// typed parameter, a method's typed parameter, and an annotated return.
#[test]
pub fn default_resolves_from_the_type_its_position_names() {
    let src = format!(
        "{OPTS}
fn make() -> Opts {{
    if true {{
        return Default::default();
    }}
    return Opts::default();
}}

impl Inner {{
    fn with(self, o: Opts) -> int {{
        return self.n + o.retries;
    }}
}}

fn main() {{
    show(Default::default());
    show(make());
    let i = Inner {{ n: 1, tags: [] }};
    print(i.with(Default::default()));
}}
"
    );
    assert_eq!(
        run_output(&src),
        "here 3 false 0.0 0 0 0 null\nhere 3 false 0.0 0 0 0 null\n4\n"
    );
}

/// Defaults inside a function body, where a literal is built each time the
/// function runs rather than once.
#[test]
pub fn struct_update_and_defaults_inside_a_loop() {
    let src = "
struct Opts { cwd: string = \"here\", tags: string[], }
fn build(cwd: string) -> Opts {
    let base = Opts { cwd: cwd, ..Default::default() };
    return Opts { tags: [\"t\"], ..base };
}
fn main() {
    for i in 0..3 {
        let o = build(str(i));
        o.tags.push(\"u\");
        print(o.cwd + \" \" + str(o.tags.len()) + \" \" + str(Opts::default().tags.len()));
    }
}
";
    assert_eq!(run_output(src), "0 2 0\n1 2 0\n2 2 0\n");
}

/// A generic struct takes a base of its own instantiation, and defaults at the
/// types its arguments name.
#[test]
pub fn generic_structs_update_and_default() {
    let src = "
struct Cell<T> { value: T, label: string = \"cell\", }
fn main() {
    let a = Cell<int> { value: 4, label: \"a\" };
    let b = Cell { value: 5, ..a };
    print(b.label + \" \" + str(b.value));
    let c = Cell<int>::default();
    print(c.label + \" \" + str(c.value));
}
";
    assert_eq!(run_output(src), "a 5\ncell 0\n");
}

#[test]
pub fn a_struct_base_of_another_type_is_refused() {
    let src = "
struct A { x: int, }
struct B { x: int, }
fn main() { let b = B { x: 1 }; let a = A { ..b }; print(a.x); }
";
    let d = compile_diag(src, "base.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "struct_base_type");
    assert_eq!(&src[d.span], "b");
}

#[test]
pub fn a_field_the_struct_lacks_is_refused_beside_a_base() {
    let src = "
struct A { x: int, }
fn main() { let a = A { y: 1, ..Default::default() }; print(a.x); }
";
    let d = compile_diag(src, "field.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "struct_no_such_field");
}

#[test]
pub fn a_written_field_of_the_wrong_type_is_refused_beside_a_base() {
    let src = "
struct A { x: int, }
fn main() { let a = A { x: \"no\", ..Default::default() }; print(a.x); }
";
    let d = compile_diag(src, "field.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "struct_field_type_mismatch");
}

#[test]
pub fn a_declared_default_of_the_wrong_type_is_refused() {
    let src = "
struct A { x: int = \"s\", }
fn main() { print(A::default().x); }
";
    let d = compile_diag(src, "decl.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "struct_field_type_mismatch");
    assert_eq!(&src[d.span], "\"s\"");
}

#[test]
pub fn default_with_no_type_to_take_is_refused() {
    let src = "
fn main() { let a = Default::default(); print(a); }
";
    let d = compile_diag(src, "none.cdl").unwrap_err();
    assert_wellformed(&d, src);
    assert_eq!(d.code, "default_without_type");
}

/// An enum has no empty value, and a struct reached again through its own
/// fields has no finite one; both ask for a declared default.
#[test]
pub fn a_field_with_no_default_value_is_refused() {
    for src in [
        "
enum E { P, Q }
struct A { e: E, }
fn main() { print(A::default()); }
",
        "
struct A { b: B, }
struct B { a: A, }
fn main() { print(A::default()); }
",
    ] {
        let d = compile_diag(src, "none.cdl").unwrap_err();
        assert_wellformed(&d, src);
        assert_eq!(d.code, "struct_field_no_default");
    }
    // A declared value is what an enum field takes instead.
    let src = "
enum E { P, Q }
struct A { e: E = E::Q, }
fn main() {
    match A::default().e {
        P => { print(1); }
        Q => { print(2); }
    }
}
";
    assert_eq!(run_output(src), "2\n");
}
