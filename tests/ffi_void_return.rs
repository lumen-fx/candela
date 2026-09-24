//! A C function that returns nothing is still called: its effect is the point
//! of calling it.

mod ffi_fixture;

use candela::Engine;
use candela::Value;

const FIXTURE: &str = r#"
use std::sync::atomic::{AtomicI64, Ordering};

static STORED: AtomicI64 = AtomicI64::new(0);

#[no_mangle]
pub extern "C" fn store(x: i64) {
    STORED.store(x, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn stored() -> i64 {
    STORED.load(Ordering::SeqCst)
}
"#;

const PROGRAM: &str = "
dylib \"voidret\" {
    store(int);
    int stored();
}

fn round_trip() {
    voidret::store(42);
    return voidret::stored();
}

fn main() {}
";

#[test]
fn a_void_c_function_runs() {
    let root = ffi_fixture::scratch_dir("void", "store");
    let _library = ffi_fixture::build_cdylib(&root, "voidret", FIXTURE);
    let script = root.join("app.cdl");

    let mut program = Engine::new()
        .compile(PROGRAM, script.to_str().expect("scratch path is utf-8"))
        .expect("the fixture import compiles");
    let value = program.call("round_trip", &[]).expect("round_trip runs");
    std::fs::remove_dir_all(&root).ok();

    assert_eq!(value, Value::Int(42));
}

/// A type with no C shape in a signature is a compile error at the signature,
/// naming the function and the type, not a panic when the calling interface is
/// built.
#[test]
fn a_type_with_no_c_shape_is_a_compile_error() {
    let root = ffi_fixture::scratch_dir("void", "no_c_shape");
    let _library = ffi_fixture::build_cdylib(&root, "voidret", FIXTURE);
    let script = root.join("app.cdl");
    let filename = script.to_str().expect("scratch path is utf-8");

    for (signature, name, type_name) in [
        ("bool stored();", "stored", "bool"),
        ("store({string: int});", "store", "{string: int}"),
        ("store(int | string);", "store", "int|string"),
    ] {
        let src = format!("dylib \"voidret\" {{\n    {signature}\n}}\n\nfn main() {{}}\n");
        let diagnostic = Engine::new()
            .compile(&src, filename)
            .err()
            .expect("the signature is refused");
        assert_eq!(diagnostic.code, "no_c_representation", "{signature}");
        assert!(
            diagnostic.message.contains(type_name),
            "{}",
            diagnostic.message
        );
        assert_eq!(
            &src[diagnostic.span.clone()],
            name,
            "the report points at the function"
        );
    }
    std::fs::remove_dir_all(&root).ok();
}
