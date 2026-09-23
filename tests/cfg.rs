//! `@cfg(...)`: code compiled only under the flags the embedder turns on.
//!
//! Each test compiles through an [`Engine`] carrying a configuration and reads
//! back which declarations survived by calling into the program. The command
//! line's `--cfg` is exercised at the end through the `candela` binary.

use candela::{Cfg, Engine, ImportResolver, Value};
use std::path::PathBuf;
use std::process::Command;

mod common;

fn engine(flags: &[&str]) -> Engine {
    Engine::new().with_cfg(flags.iter().copied().collect())
}

/// Compiles `src` with `flags` on and returns what `probe()` answers.
fn probe(flags: &[&str], src: &str) -> Value {
    let mut program = engine(flags)
        .compile(src, "main.cdl")
        .unwrap_or_else(|d| panic!("compile with {flags:?} failed: {}", d.message));
    program.call("probe", &[]).expect("probe runs")
}

fn string(s: &str) -> Value {
    Value::String(String::from(s))
}

const TARGETS: &str = r#"
@cfg(web)
fn target() -> string { return "web"; }
@cfg(not(web))
fn target() -> string { return "desktop"; }

fn probe() -> string { return target(); }
fn main() {}
"#;

#[test]
fn a_flag_selects_between_two_declarations() {
    assert_eq!(probe(&["web"], TARGETS), string("web"));
    assert_eq!(probe(&[], TARGETS), string("desktop"));
}

#[test]
fn a_flag_nobody_enabled_is_false() {
    assert_eq!(probe(&["mobile", "debug"], TARGETS), string("desktop"));
}

#[test]
fn any_holds_when_one_condition_does() {
    let src = r#"
@cfg(any(web, mobile))
fn size() -> string { return "small"; }
@cfg(not(any(web, mobile)))
fn size() -> string { return "large"; }
fn probe() -> string { return size(); }
fn main() {}
"#;
    assert_eq!(probe(&["web"], src), string("small"));
    assert_eq!(probe(&["mobile"], src), string("small"));
    assert_eq!(probe(&[], src), string("large"));
}

#[test]
fn all_holds_when_every_condition_does() {
    let src = r#"
@cfg(all(web, debug))
fn mode() -> string { return "web debug"; }
@cfg(not(all(web, debug)))
fn mode() -> string { return "other"; }
fn probe() -> string { return mode(); }
fn main() {}
"#;
    assert_eq!(probe(&["web", "debug"], src), string("web debug"));
    assert_eq!(probe(&["web"], src), string("other"));
    assert_eq!(probe(&["debug"], src), string("other"));
}

#[test]
fn empty_any_is_false_and_empty_all_is_true() {
    let src = r#"
fn probe() -> string {
    let s = "";
    @cfg(any())
    s = s + "any";
    @cfg(all())
    s = s + "all";
    return s;
}
fn main() {}
"#;
    assert_eq!(probe(&[], src), string("all"));
}

#[test]
fn a_key_value_pair_matches_only_its_value() {
    let src = r#"
fn probe() -> string {
    let s = "";
    @cfg(target = "wasm32")
    s = s + "wasm32";
    @cfg(target = "x86_64")
    s = s + "x86_64";
    return s;
}
fn main() {}
"#;
    let mut cfg = Cfg::new();
    cfg.enable_value("target", "wasm32");
    let mut program = Engine::new()
        .with_cfg(cfg)
        .compile(src, "main.cdl")
        .expect("compiles");
    assert_eq!(program.call("probe", &[]).unwrap(), string("wasm32"));
}

#[test]
fn stacked_attributes_all_have_to_hold() {
    let src = r#"
fn probe() -> string {
    let s = "none";
    @cfg(web)
    @cfg(debug)
    s = "both";
    return s;
}
fn main() {}
"#;
    assert_eq!(probe(&["web", "debug"], src), string("both"));
    assert_eq!(probe(&["web"], src), string("none"));
}

#[test]
fn an_import_of_a_missing_module_under_a_false_cfg_compiles() {
    let src = r#"
@cfg(web)
import "js";
@cfg(web)
import "./not/here.cdl" as nowhere;

@cfg(web)
fn track(event: string) { js::call("gtag", ["event", event]); }
@cfg(not(web))
fn track(event: string) {}

fn probe() -> string {
    track("opened");
    return "tracked";
}
fn main() {}
"#;
    assert_eq!(probe(&[], src), string("tracked"));
}

#[test]
fn the_same_import_is_an_error_when_its_cfg_holds() {
    let src = r#"
@cfg(web)
import "js";
fn main() {}
"#;
    let err = engine(&["web"])
        .compile(src, "main.cdl")
        .err()
        .expect("there is no js module to import");
    assert_eq!(err.code, "cannot_read_file", "{err:?}");
}

#[test]
fn both_declarations_active_is_the_ordinary_duplicate_error() {
    let src = r"
@cfg(web)
fn f() -> int { return 1; }
@cfg(any(web, desktop))
fn f() -> int { return 2; }
fn main() {}
";
    let err = engine(&["web"])
        .compile(src, "main.cdl")
        .err()
        .expect("both are compiled");
    let plain = engine(&[])
        .compile(
            "fn f() -> int { return 1; }\nfn f() -> int { return 2; }\nfn main() {}",
            "main.cdl",
        )
        .err()
        .expect("a plain duplicate");
    assert_eq!(err.code, plain.code);
    // Only one survives under `desktop`, so that compile is clean.
    assert!(engine(&["desktop"]).compile(src, "main.cdl").is_ok());
}

#[test]
fn structs_enums_and_impl_methods_take_the_attribute() {
    let src = r#"
@cfg(web)
struct Point { x: int, web: bool }
@cfg(not(web))
struct Point { x: int }

@cfg(web)
enum Shade { Light, Dark, Auto }
@cfg(not(web))
enum Shade { Light, Dark }

impl Point {
    @cfg(web)
    fn describe(self) -> string { return "web point"; }
    @cfg(not(web))
    fn describe(self) -> string { return "point"; }
}

@cfg(web)
impl Point {
    fn only_on_web(self) -> int { return 1; }
}

fn probe() -> string {
    let s = Shade::Dark;
    return Point { x: 1 }.describe();
}
fn main() {}
"#;
    assert_eq!(probe(&[], src), string("point"));
}

#[test]
fn a_statement_takes_the_attribute() {
    let src = r#"
fn probe() -> string {
    let s = "start";
    @cfg(web)
    s = web_only_function(s);
    @cfg(not(web))
    {
        s = s + " desktop";
    }
    return s;
}
fn main() {}
"#;
    assert_eq!(probe(&[], src), string("start desktop"));
}

#[test]
fn a_macro_in_dropped_code_is_not_expanded() {
    let mut engine = Engine::new();
    engine.register_macro("boom", |_: &str| {
        panic!("a dropped macro must not reach its expander")
    });
    let src = r"
@cfg(web)
fn f() { let x = boom!(anything); let y = missing!(too); }
fn probe() -> int { return 1; }
fn main() {}
";
    let mut program = engine.compile(src, "main.cdl").expect("compiles");
    assert_eq!(program.call("probe", &[]).unwrap(), Value::Int(1));
}

#[test]
fn dropped_code_still_has_to_parse() {
    let err = engine(&[])
        .compile("@cfg(web)\nfn f( { }\nfn main() {}", "main.cdl")
        .err()
        .expect("a syntax error is an error under any configuration");
    assert_eq!(err.code, "unexpected_token");
}

#[test]
fn an_unknown_attribute_is_an_error() {
    let err = engine(&[])
        .compile("@derive(eq)\nfn main() {}", "main.cdl")
        .err()
        .expect("derive is not an attribute");
    assert_eq!(err.code, "unknown_attribute");
}

#[test]
fn an_attribute_needs_something_to_apply_to() {
    let err = engine(&[])
        .compile("fn main() {}\n@cfg(web)\n", "main.cdl")
        .err()
        .expect("nothing follows the attribute");
    assert_eq!(err.code, "attribute_without_item");
    let err = engine(&[])
        .compile("fn main() {\n@cfg(web)\n}", "main.cdl")
        .err()
        .expect("nothing follows the attribute");
    assert_eq!(err.code, "attribute_without_item");
}

#[test]
fn not_takes_exactly_one_condition() {
    let err = engine(&[])
        .compile("@cfg(not(a, b))\nfn f() {}\nfn main() {}", "main.cdl")
        .err()
        .expect("not() of two conditions");
    assert_eq!(err.code, "unexpected_token");
}

#[test]
fn a_scope_configures_a_compile_outside_an_engine() {
    let src = "@cfg(web)\nimport \"js\";\nfn main() {}";
    // Nothing enabled: the import is dropped and the build succeeds.
    assert!(
        candela::collect_diagnostic(|| candela::build_bytecode(
            src.to_owned(),
            "main.cdl",
            &ImportResolver::new()
        ))
        .is_ok()
    );
    // `web` on: the import is compiled and there is no such module.
    let cfg: Cfg = std::iter::once("web").collect();
    assert!(
        cfg.scope(|| candela::collect_diagnostic(|| {
            candela::build_bytecode(src.to_owned(), "main.cdl", &ImportResolver::new())
        }))
        .is_err()
    );
}

#[test]
fn the_command_line_turns_flags_on() {
    let dir = std::env::temp_dir().join(format!("candela-cfg-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file: PathBuf = dir.join("main.cdl");
    std::fs::write(
        &file,
        r#"
@cfg(web)
fn target() -> string { return "web"; }
@cfg(not(web))
fn target() -> string { return "desktop"; }
fn main() {
    print(target());
    @cfg(mode = "fast")
    print("fast");
}
"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_candela"));
        command.current_dir(&dir).arg("run").args(args).arg(&file);
        let output = common::output_with_deadline(&mut command, "candela run --cfg");
        assert!(output.status.success(), "{output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert_eq!(run(&[]).trim(), "desktop");
    assert_eq!(run(&["--cfg", "web"]).trim(), "web");
    assert_eq!(
        run(&["--cfg", "web", "--cfg", "mode=fast"]).trim(),
        "web\nfast"
    );
    assert_eq!(run(&["--cfg", "mode=\"fast\""]).trim(), "desktop\nfast");
    let _ = std::fs::remove_dir_all(&dir);
}
