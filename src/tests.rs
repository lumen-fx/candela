use crate::RegisterFile;
use crate::compile;
use crate::compiler::compiler_data::Source;
use crate::data::Data;
use crate::instr::Instr;

macro_rules! run_and_check_registers {
    ($contents:expr, $expected:expr) => {
        let filename = "test.kl";
        let out = compile(String::from($contents), filename, true);
        let instructions = out.instructions;
        let mut arrays = out.pools;
        let mut reg = RegisterFile(out.registers);
        crate::vm::execute(
            &instructions,
            &mut reg,
            &mut arrays,
            &crate::errors::ErrorCtx {
                instr_src: out.instr_src,
                sources: vec![Source {
                    filename: filename.into(),
                    contents: String::from($contents),
                }],
            },
            &out.fn_registers,
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
        }));
    };
}

macro_rules! run {
    ($contents:expr) => {
        let filename = "test.kl";
        let out = compile(String::from($contents), filename, true);
        let mut arrays = out.pools;
        crate::vm::execute(
            &out.instructions,
            &mut RegisterFile(out.registers),
            &mut arrays,
            &crate::errors::ErrorCtx {
                instr_src: out.instr_src,
                sources: vec![Source {
                    filename: filename.into(),
                    contents: String::from($contents),
                }],
            },
            &out.fn_registers,
            &[],
            &[],
            &[],
            out.allocated_arg_count,
            out.allocated_call_depth,
            &[],
            &[],
            0,
        );
    };
}

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
            let x = 2147483647;
            x += 1;
            print(x);
        }
        ",
        (-2_147_483_648_i32).into()
    );
}

#[test]
pub fn int_wraps_on_underflow() {
    run_and_check_registers!(
        "
        fn main() {
            let x = -2147483648;
            x -= 1;
            print(x);
        }
        ",
        2_147_483_647_i32.into()
    );
}

#[test]
pub fn negative_int_literal() {
    run_and_check_registers!(
        "
        fn main() {
            let x = -2147483648;
            print(x);
        }
        ",
        (-2_147_483_648_i32).into()
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
// The three error funnels (throw_parser_error, throw_compiler_error,
// throw_error) record a structured `Diagnostic` and unwind instead of printing
// + exiting whenever `collect_diagnostic` has installed a sink on the thread.
// Without a sink the CLI path is byte-for-byte unchanged.
// ---------------------------------------------------------------------------

use crate::Diagnostic;
use crate::errors::collect_diagnostic;

/// Compiles `src` under a diagnostic sink, returning the first structured error
/// (parser or compiler) instead of printing + exiting. This is exactly what an
/// embedder would write against the public `collect_diagnostic` surface.
fn compile_diag(src: &str, filename: &str) -> Result<(), Diagnostic> {
    collect_diagnostic(|| {
        let _ = compile(String::from(src), filename, false);
    })
}

/// Compiles then executes `src` under a diagnostic sink, surfacing parser,
/// compiler and runtime errors as structured `Diagnostic`s.
fn run_diag(src: &str, filename: &str) -> Result<(), Diagnostic> {
    collect_diagnostic(|| {
        let out = compile(String::from(src), filename, false);
        let mut arrays = out.pools;
        crate::vm::execute(
            &out.instructions,
            &mut RegisterFile(out.registers),
            &mut arrays,
            &crate::errors::ErrorCtx {
                instr_src: out.instr_src,
                sources: vec![Source {
                    filename: filename.into(),
                    contents: String::from(src),
                }],
            },
            &out.fn_registers,
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
    let d = compile_diag("fn foo() { return 1; }", "diag.kl").unwrap_err();
    assert_eq!(d.message, "Cannot find main function");
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
// candela is gradually typed: a condition of any type compiles. Only the
// boolean `false` fails a test, so every other value takes the true branch.
// ---------------------------------------------------------------------------

#[test]
pub fn if_accepts_int_condition_and_takes_the_true_branch() {
    run_and_check_registers!(
        "
        fn main() {
            if 0 { print(1); } else { print(2); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn if_accepts_null_condition_and_takes_the_true_branch() {
    run_and_check_registers!(
        "
        fn main() {
            if null { print(1); } else { print(2); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn if_accepts_string_condition_and_takes_the_true_branch() {
    run_and_check_registers!(
        "
        fn main() {
            if \"s\" { print(1); } else { print(2); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn else_if_accepts_non_bool_condition() {
    run_and_check_registers!(
        "
        fn main() {
            if false { print(0); } else if 3 { print(1); } else { print(2); }
        }
        ",
        1.into()
    );
}

#[test]
pub fn while_accepts_non_bool_condition() {
    run_and_check_registers!(
        "
        fn main() {
            let n = 0;
            while 1 {
                n += 1;
                break;
            }
            print(n);
        }
        ",
        1.into()
    );
}

#[test]
pub fn inline_if_accepts_non_bool_condition() {
    run_and_check_registers!(
        "
        fn main() {
            let x = if 1 { 7 } else { 8 };
            print(x);
        }
        ",
        7.into()
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
