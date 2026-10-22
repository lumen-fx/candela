//! Lists crossing into C: an empty list, and a list of numbers C fills in.

mod ffi_fixture;

use candela::Engine;
use candela::Value;

const FIXTURE: &str = r#"
/// The sum of the first `len` ints at `xs`.
#[no_mangle]
pub unsafe extern "C" fn sum(xs: *const i64, len: i64) -> i64 {
    (0..len as usize).map(|i| *xs.add(i)).sum()
}

/// How many strings `argv` holds, counting up to `len`, after checking the
/// pointer it was handed is one it may read.
#[no_mangle]
pub unsafe extern "C" fn count(argv: *const *const u8, len: i64) -> i64 {
    if argv.is_null() {
        return -1;
    }
    len
}

/// Writes `i * i` into each of the `len` ints at `out`, and answers `len`.
#[no_mangle]
pub unsafe extern "C" fn squares(out: *mut i64, len: i64) -> i64 {
    for i in 0..len as usize {
        *out.add(i) = (i * i) as i64;
    }
    len
}

/// Halves each of the `len` floats at `out`.
#[no_mangle]
pub unsafe extern "C" fn halve(out: *mut f64, len: i64) {
    for i in 0..len as usize {
        *out.add(i) /= 2.0;
    }
}
"#;

const PROGRAM: &str = r#"
dylib "arrays" {
    int sum(int[], int);
    int count(string[], int);
    int squares(int[], int);
    halve(float[], int);
}

fn empty_ints() -> int {
    let xs: int[] = [];
    return arrays::sum(xs, 0);
}

fn empty_strings() -> int {
    let args: string[] = [];
    return arrays::count(args, 0);
}

fn filled() -> int[] {
    let buf = [0, 0, 0, 0];
    arrays::squares(buf, buf.len());
    return buf;
}

fn halved() -> float[] {
    let xs = [1.0, 3.0];
    arrays::halve(xs, 2);
    return xs;
}

fn main() {}
"#;

fn program(tag: &str) -> (std::path::PathBuf, candela::Program) {
    let root = ffi_fixture::scratch_dir("arrays", tag);
    let _library = ffi_fixture::build_cdylib(&root, "arrays", FIXTURE);
    let script = root.join("app.cdl");
    let program = Engine::new()
        .compile(PROGRAM, script.to_str().expect("scratch path is utf-8"))
        .expect("the fixture import compiles");
    (root, program)
}

/// An empty list reaches C as a pointer it may hold, not a read past the end
/// of the list.
#[test]
fn an_empty_list_crosses_as_a_valid_pointer() {
    let (root, mut program) = program("empty");
    assert_eq!(program.call("empty_ints", &[]).unwrap(), Value::Int(0));
    assert_eq!(program.call("empty_strings", &[]).unwrap(), Value::Int(0));
    std::fs::remove_dir_all(&root).ok();
}

/// What C writes into a list of ints or floats it was handed is in the list
/// once the call returns.
#[test]
fn a_list_of_numbers_is_written_back() {
    let (root, mut program) = program("write_back");
    assert_eq!(
        program.call("filled", &[]).unwrap(),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(4),
            Value::Int(9)
        ])
    );
    assert_eq!(
        program.call("halved", &[]).unwrap(),
        Value::Array(vec![Value::Float(0.5), Value::Float(1.5)])
    );
    std::fs::remove_dir_all(&root).ok();
}
