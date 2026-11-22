//! Attributes a `host` block declares: what the compiler checks when a script
//! writes one, and what the attribute table records.

use candela::AttributeTarget;
use candela::AttributeValue;
use candela::DataType;
use candela::Engine;
use candela::HostRegistry;
use candela::Program;
use candela::build_bytecode;
use candela::load_program;
use candela::read_attributes;
use std::path::PathBuf;

const TAGS: &str = r#"
host "tags" {
    attribute marker on struct;
    attribute task(
        writes: type[] = [],
        after: fn[] = [],
        priority: int = 0,
        weight: float = 1.0,
        label: string = "",
        enabled: bool = true,
        phase: Phase = Phase::Late,
    ) on fn;
    attribute needs(level: int) on fn;
}

enum Phase { Early, Late, Payload(int) }

fn helper() {}
"#;

/// A directory holding the `tags` package and `main.cdl` with `src`, unique to
/// `label`.
fn project(label: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("candela_attrs_{label}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tags.cdl"), TAGS).unwrap();
    std::fs::write(dir.join("main.cdl"), src).unwrap();
    dir.join("main.cdl")
}

fn compile(label: &str, src: &str) -> Result<Program, candela::Diagnostic> {
    let path = project(label, src);
    Engine::new().compile(src, path.to_str().unwrap())
}

fn error_code(label: &str, src: &str) -> String {
    match compile(label, src) {
        Ok(_) => panic!("{label} compiles, and should not"),
        Err(diagnostic) => diagnostic.code,
    }
}

#[test]
fn every_argument_is_recorded_with_the_defaults_filled_in() {
    let src = r#"
import "tags.cdl";

@tags::marker struct Position { x: float }

@tags::task(writes: [Position, int[]], priority: -2, label: "move", phase: tags::Phase::Early)
fn step(rows: Position[], dt: float) {}

@tags::task(after: [step, tags::helper])
fn later(n) {}

fn main() {}
"#;
    let program = compile("recorded", src).unwrap_or_else(|d| panic!("{}", d.message));
    let table = program.attributes();
    assert_eq!(table.attributes.len(), 3);

    let marker = &table.attributes[0];
    assert_eq!(
        (marker.namespace.as_str(), marker.name.as_str()),
        ("tags", "marker")
    );
    let AttributeTarget::Struct(id) = marker.target else {
        panic!("marker goes on a struct");
    };
    assert_eq!(table.structs[id as usize].name, "Position");

    let step = &table.attributes[1];
    let AttributeTarget::Fn { name, params } = &step.target else {
        panic!("task goes on a fn");
    };
    assert_eq!(name, "step");
    let params: Vec<(&str, String)> = params
        .iter()
        .map(|(n, t)| (n.as_str(), table.type_name(t)))
        .collect();
    assert_eq!(
        params,
        [
            ("rows", String::from("Position[]")),
            ("dt", String::from("float"))
        ]
    );
    let names: Vec<&str> = step.args.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "writes", "after", "priority", "weight", "label", "enabled", "phase"
        ]
    );
    let writes: Vec<String> = step
        .arg("writes")
        .and_then(AttributeValue::as_list)
        .unwrap()
        .iter()
        .map(|v| match v {
            AttributeValue::Type(ty) => table.type_name(ty),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(writes, ["Position", "int[]"]);
    assert!(matches!(
        step.arg("priority"),
        Some(AttributeValue::Int(-2))
    ));
    assert!(
        matches!(step.arg("weight"), Some(AttributeValue::Float(w)) if (*w - 1.0).abs() < f64::EPSILON)
    );
    assert!(matches!(step.arg("label"), Some(AttributeValue::String(s)) if s == "move"));
    assert!(matches!(
        step.arg("enabled"),
        Some(AttributeValue::Bool(true))
    ));
    let Some(AttributeValue::Variant { enum_id, name }) = step.arg("phase") else {
        panic!("phase is a variant");
    };
    assert_eq!(
        (table.enums[*enum_id as usize].name.as_str(), name.as_str()),
        ("Phase", "Early")
    );

    let later = &table.attributes[2];
    let AttributeTarget::Fn { params, .. } = &later.target else {
        panic!("task goes on a fn");
    };
    // A parameter without a type is recorded as any.
    assert!(matches!(params[0].1, DataType::Unknown));
    let after = later
        .arg("after")
        .and_then(AttributeValue::as_list)
        .unwrap();
    assert!(matches!(&after[0], AttributeValue::Fn { name, .. } if name == "step"));
    assert!(matches!(&after[1], AttributeValue::Fn { name, .. } if name == "helper"));
    let Some(AttributeValue::Variant { name, .. }) = later.arg("phase") else {
        panic!("phase defaults to a variant");
    };
    assert_eq!(name, "Late");
}

/// The table records the namespace the host block declares, whatever the
/// script binds the module as, and an item import reaches the attribute by
/// its bare name.
#[test]
fn the_host_namespace_is_recorded_under_any_import_name() {
    let src = r#"
import "tags.cdl" as t;
import "tags.cdl" { marker };

@t::marker struct A { x: int }
@marker struct B { x: int }

fn main() {}
"#;
    let program = compile("alias", src).unwrap_or_else(|d| panic!("{}", d.message));
    let table = program.attributes();
    let found: Vec<(&str, &str)> = table
        .attributes
        .iter()
        .map(|a| (a.namespace.as_str(), a.name.as_str()))
        .collect();
    assert_eq!(found, [("tags", "marker"), ("tags", "marker")]);
}

/// A file that declares an attribute writes it bare.
#[test]
fn the_declaring_file_writes_its_own_attribute_bare() {
    let src = r#"
host "engine" {
    attribute tick(rate: int = 60) on fn;
}

@tick(rate: 30)
fn update() {}

fn main() {}
"#;
    let program = Engine::new()
        .compile(src, "own.cdl")
        .unwrap_or_else(|d| panic!("{}", d.message));
    let table = program.attributes();
    assert_eq!(table.attributes.len(), 1);
    assert!(matches!(
        table.attributes[0].arg("rate"),
        Some(AttributeValue::Int(30))
    ));
}

/// A host function returning a type named `attribute` is still a function,
/// one with parameters or without.
#[test]
fn a_function_returning_a_type_named_attribute_still_parses() {
    let src = r#"
struct attribute { x: int }
host "app" {
    fn make(x: int) -> attribute;
    fn none() -> attribute;
}
fn use_them() -> int { return app::make(1).x + app::none().x; }
fn main() {}
"#;
    Engine::new()
        .check(src, "fns.cdl")
        .unwrap_or_else(|d| panic!("{}", d[0].message));
}

/// The attribute table reads out of a `.cdlb` with nothing registered, and a
/// loaded artifact answers the same table.
#[test]
fn the_table_reads_out_of_an_artifact() {
    let src = String::from(
        r#"
host "tags" {
    attribute task(priority: int = 0) on fn;
    fn now() -> int;
}

@task(priority: 3)
fn step() {}

fn main() {}
"#,
    );
    let bytes = build_bytecode(src, "art.cdl", &candela::ImportResolver::new()).expect("builds");
    let table = read_attributes(&bytes).expect("reads");
    assert_eq!(table.attributes.len(), 1);
    assert!(matches!(
        table.attributes[0].arg("priority"),
        Some(AttributeValue::Int(3))
    ));

    let mut hosts = HostRegistry::new();
    hosts.register_host_fn("tags", "now", || 0_i64);
    let program = load_program(&bytes, &hosts).expect("loads");
    assert_eq!(program.attributes().attributes.len(), 1);
}

#[test]
fn an_unknown_attribute_is_an_error() {
    let src = "import \"tags.cdl\";\n@tags::markr struct A { x: int }\nfn main() {}";
    assert_eq!(error_code("unknown", src), "unknown_attribute");
    let src = "@marker struct A { x: int }\nfn main() {}";
    assert_eq!(error_code("unknown_bare", src), "unknown_attribute");
}

#[test]
fn an_attribute_on_the_wrong_kind_of_item_is_an_error() {
    let cases = [
        (
            "struct_attr_on_fn",
            "import \"tags.cdl\";\n@tags::marker fn f() {}\nfn main() {}",
        ),
        (
            "fn_attr_on_struct",
            "import \"tags.cdl\";\n@tags::task struct A { x: int }\nfn main() {}",
        ),
        (
            "on_generic_fn",
            "import \"tags.cdl\";\n@tags::task fn f<T>(x: T) {}\nfn main() {}",
        ),
        (
            "on_generic_struct",
            "import \"tags.cdl\";\n@tags::marker struct A<T> { x: T }\nfn main() {}",
        ),
        (
            "on_statement",
            "import \"tags.cdl\";\nfn main() { @tags::marker let x = 1; }",
        ),
        (
            "on_enum",
            "import \"tags.cdl\";\n@tags::marker enum E { A }\nfn main() {}",
        ),
        (
            "on_method",
            "import \"tags.cdl\";\nstruct A { x: int }\nimpl A { @tags::task fn get(self) -> int { return self.x; } }\nfn main() {}",
        ),
    ];
    for (label, src) in cases {
        assert_eq!(error_code(label, src), "attribute_wrong_target", "{label}");
    }
}

#[test]
fn a_bad_argument_is_an_error() {
    let cases = [
        ("unknown_param", "@tags::task(writes: [A], prio: 1)"),
        ("missing_required", "@tags::needs"),
        ("wrong_type", "@tags::task(priority: \"high\")"),
        ("float_for_int", "@tags::task(priority: 1.5)"),
        ("not_a_list", "@tags::task(writes: A)"),
        ("unknown_fn", "@tags::task(after: [stpe])"),
        ("repeated", "@tags::task(priority: 1, priority: 2)"),
        ("positional_after_named", "@tags::task(priority: 1, [A])"),
        ("too_many_positional", "@tags::needs(1, 2)"),
        (
            "payload_variant",
            "@tags::task(phase: tags::Phase::Payload)",
        ),
        ("not_a_variant", "@tags::task(phase: tags::Phase::Middle)"),
    ];
    for (label, attr) in cases {
        let src = format!(
            "import \"tags.cdl\";\nstruct A {{ x: int }}\n{attr}\nfn step() {{}}\nfn main() {{}}"
        );
        assert_eq!(error_code(label, &src), "attribute_bad_argument", "{label}");
    }
}

/// Arguments fill the parameters in order and then by name, the way a call's
/// do.
#[test]
fn positional_arguments_fill_the_parameters_in_order() {
    let src = "import \"tags.cdl\";\nstruct A { x: int }\n@tags::task([A], priority: 3)\nfn step() {}\n@tags::needs(2)\nfn other() {}\nfn main() {}";
    let program = compile("positional", src).unwrap_or_else(|d| panic!("{}", d.message));
    let table = program.attributes();
    let step = &table.attributes[0];
    assert!(matches!(step.arg("priority"), Some(AttributeValue::Int(3))));
    let writes = step
        .arg("writes")
        .and_then(AttributeValue::as_list)
        .unwrap();
    assert_eq!(writes.len(), 1);
    assert!(matches!(
        table.attributes[1].arg("level"),
        Some(AttributeValue::Int(2))
    ));
}

/// An attribute goes on every kind of declaration its `on` clause lists: a
/// function, a method, a parameter, a constant, a struct, a field, an enum, a
/// variant and an `impl` block. Each records what it is on, and `@cfg` drops a
/// field, a variant or a parameter it turns off.
#[test]
fn an_attribute_goes_on_every_kind_of_declaration() {
    let src = r#"
host "meta" {
    attribute doc(text: string) on fn, method, param, const, struct, field, enum, variant, impl;
    attribute range(min: int = 0, max: int = 100) on field, param;
}

@doc("a point")
struct Point {
    @doc("across") @range(max: 10) x: int,
    @cfg(never) gone: int,
    y: int,
}

@doc("the point's methods")
impl Point {
    @doc("the sum")
    fn sum(self, @range(1, 5) scale: int) -> int { return (self.x + self.y) * scale; }
}

@doc("a color")
enum Color {
    @doc("warm") Red,
    @cfg(never) Gone,
    Blue,
}

@doc("the limit")
const LIMIT: int = 3;

@doc("adds")
fn add(@doc("left") a: int, @cfg(never) b: int, c: int) -> int { return a + c; }

fn main() {
    let p = Point { x: 1, y: 2 };
    print(p.sum(2) + add(1, 2) + LIMIT);
    print(Color::Blue);
}
"#;
    let program = Engine::new()
        .compile(src, "meta.cdl")
        .unwrap_or_else(|d| panic!("{}", d.message));
    let table = program.attributes();
    let seen: Vec<String> = table
        .attributes
        .iter()
        .map(|a| {
            let on = match &a.target {
                AttributeTarget::Fn { name, params } => format!("fn {name}/{}", params.len()),
                AttributeTarget::Method {
                    owner,
                    name,
                    params,
                } => format!("method {}::{name}/{}", table.type_name(owner), params.len()),
                AttributeTarget::Param {
                    function,
                    owner,
                    name,
                    ty,
                } => format!(
                    "param {}{function}({name}: {})",
                    owner
                        .as_ref()
                        .map_or_else(String::new, |o| format!("{}::", table.type_name(o))),
                    table.type_name(ty)
                ),
                AttributeTarget::Const { name, ty } => {
                    format!("const {name}: {}", table.type_name(ty))
                }
                AttributeTarget::Struct(id) => {
                    format!("struct {}", table.structs[*id as usize].name)
                }
                AttributeTarget::Field { struct_id, name } => {
                    format!("field {}.{name}", table.structs[*struct_id as usize].name)
                }
                AttributeTarget::Enum(id) => format!("enum {}", table.enums[*id as usize].name),
                AttributeTarget::Variant { enum_id, name } => {
                    format!("variant {}::{name}", table.enums[*enum_id as usize].name)
                }
                AttributeTarget::Impl(ty) => format!("impl {}", table.type_name(ty)),
            };
            format!("{} {on}", a.name)
        })
        .collect();
    assert_eq!(
        seen,
        [
            "doc struct Point",
            "doc field Point.x",
            "range field Point.x",
            "doc impl Point",
            "doc method Point::sum/2",
            "range param Point::sum(scale: int)",
            "doc enum Color",
            "doc variant Color::Red",
            "doc const LIMIT: int",
            "doc fn add/2",
            "doc param add(a: int)",
        ],
    );
    let range = &table.attributes[2];
    assert!(matches!(range.arg("min"), Some(AttributeValue::Int(0))));
    assert!(matches!(range.arg("max"), Some(AttributeValue::Int(10))));
    let scale = &table.attributes[5];
    assert!(matches!(scale.arg("min"), Some(AttributeValue::Int(1))));
    assert!(matches!(scale.arg("max"), Some(AttributeValue::Int(5))));
    assert_eq!(
        table.structs[0].fields.len(),
        2,
        "@cfg(never) dropped the field"
    );
    assert_eq!(table.enums[0].variants, ["Red", "Blue"]);

    // The table survives a .cdlb round trip.
    let bytes = build_bytecode(src.to_owned(), "meta.cdl", &candela::ImportResolver::new())
        .expect("builds");
    assert_eq!(read_attributes(&bytes).unwrap().attributes.len(), 11);
}

/// An attribute goes only where its declaration says, and never on the parts
/// of a generic declaration or of a struct declared inside a function.
#[test]
fn an_attribute_on_a_part_it_does_not_list_is_an_error() {
    let decl = "host \"meta\" {\n    attribute doc on fn;\n    attribute any on fn, method, param, const, struct, field, enum, variant, impl;\n}\n";
    for (label, body, code) in [
        (
            "field",
            "struct A { @doc x: int }",
            "attribute_wrong_target",
        ),
        ("variant", "enum E { @doc A }", "attribute_wrong_target"),
        ("param", "fn f(@doc a: int) {}", "attribute_wrong_target"),
        ("const", "@doc const N: int = 1;", "attribute_wrong_target"),
        (
            "impl",
            "struct A { x: int }\n@doc impl A {}",
            "attribute_wrong_target",
        ),
        (
            "method",
            "struct A { x: int }\nimpl A { @doc fn f(self) {} }",
            "attribute_wrong_target",
        ),
        (
            "generic_impl",
            "struct C<T> { v: T }\n@any impl<T> C<T> {}",
            "attribute_wrong_target",
        ),
        (
            "generic_method",
            "struct C<T> { v: T }\nimpl<T> C<T> { @any fn get(self) -> T { return self.v; } }",
            "attribute_wrong_target",
        ),
        (
            "generic_field",
            "struct C<T> { @any v: T }",
            "attribute_wrong_target",
        ),
        (
            "generic_param",
            "fn f<T>(@any v: T) {}",
            "attribute_wrong_target",
        ),
        (
            "local_struct_field",
            "fn g() { struct L { @any x: int } }",
            "attribute_wrong_target",
        ),
    ] {
        let src = format!("{decl}{body}\nfn main() {{}}");
        let err = Engine::new()
            .compile(&src, &format!("{label}.cdl"))
            .err()
            .unwrap_or_else(|| panic!("{label} compiles, and should not"));
        assert_eq!(err.code, code, "{label}: {}", err.message);
    }
}

/// A type name that does not resolve is the unknown-type error any other
/// annotation gets.
#[test]
fn a_misspelt_type_argument_is_an_unknown_type() {
    let src = "import \"tags.cdl\";\nstruct Position { x: int }\n@tags::task(writes: [Positon])\nfn step() {}\nfn main() {}";
    let err = compile("misspelt", src)
        .err()
        .expect("Positon is not a type");
    assert!(err.message.contains("Positon"), "{}", err.message);
}

#[test]
fn a_bad_declaration_is_an_error() {
    let src = "host \"h\" {\n    attribute bad(m: {string: int}) on fn;\n}\nfn main() {}";
    let err = Engine::new().compile(src, "bad.cdl").err().unwrap();
    assert_eq!(err.code, "attribute_parameter_type");

    let src = "host \"h\" {\n    attribute bad(n: int = \"x\") on fn;\n}\nfn main() {}";
    let err = Engine::new().compile(src, "default.cdl").err().unwrap();
    assert_eq!(err.code, "attribute_bad_argument");

    let src =
        "host \"h\" {\n    attribute twice on fn;\n    attribute twice on fn;\n}\nfn main() {}";
    let err = Engine::new().compile(src, "twice.cdl").err().unwrap();
    assert_eq!(err.code, "attribute_already_declared");
}

/// `@cfg` is untouched: it still drops what it turns off, attributes included.
#[test]
fn cfg_still_drops_an_item_with_its_attributes() {
    let src = r#"
import "tags.cdl";

@cfg(never) @tags::task(priority: "not even checked")
fn gone() {}

@tags::task
@cfg(not(never))
fn kept() {}

fn main() {}
"#;
    let program = compile("cfg", src).unwrap_or_else(|d| panic!("{}", d.message));
    let table = program.attributes();
    assert_eq!(table.attributes.len(), 1);
    assert!(
        matches!(&table.attributes[0].target, AttributeTarget::Fn { name, .. } if name == "kept")
    );
}

/// An attribute declared `on enum` goes on an enum declaration and records
/// the enum's id; a parameter typed `enum` takes a variant of any enum the
/// script declares; a parameter written `T?` may be left out, and is then
/// absent from the table; and a function argument carries the function's
/// parameters and declared return type.
#[test]
fn enum_targets_any_enum_parameters_and_function_signatures() {
    let src = r#"
host "flow" {
    attribute state on enum;
    attribute stage(in_state: enum[] = [], on_enter: enum?, run_if: fn[] = []) on fn;
}

@state
enum Phase { Ready, Playing, Over }

enum Mode { Easy, Hard }

fn playing(level: int, label) -> bool { return level > 0; }

@stage(in_state: [Phase::Playing, Mode::Hard], on_enter: Phase::Over, run_if: [playing])
fn crash() {}

@stage
fn idle() {}

fn main() {}
"#;
    let program = Engine::new().compile(src, "flow.cdl").unwrap();
    let table = program.attributes();
    let state = &table.attributes[0];
    let AttributeTarget::Enum(phase) = state.target else {
        panic!("state goes on an enum");
    };
    assert_eq!(table.enums[phase as usize].name, "Phase");

    let crash = &table.attributes[1];
    let in_state = crash
        .arg("in_state")
        .and_then(AttributeValue::as_list)
        .unwrap();
    let names: Vec<(&str, &str)> = in_state
        .iter()
        .map(|v| match v {
            AttributeValue::Variant { enum_id, name } => {
                (table.enums[*enum_id as usize].name.as_str(), name.as_str())
            }
            other => panic!("{other:?} is not a variant"),
        })
        .collect();
    assert_eq!(names, [("Phase", "Playing"), ("Mode", "Hard")]);
    assert!(
        matches!(crash.arg("on_enter"), Some(AttributeValue::Variant { name, .. }) if name == "Over")
    );
    let run_if = crash
        .arg("run_if")
        .and_then(AttributeValue::as_list)
        .unwrap();
    let AttributeValue::Fn { name, params, ret } = &run_if[0] else {
        panic!("run_if holds functions");
    };
    assert_eq!(name, "playing");
    assert_eq!(params.len(), 2);
    assert!(matches!(params[0].1, DataType::Int));
    assert!(matches!(params[1].1, DataType::Unknown));
    assert!(matches!(ret, DataType::Bool));

    let idle = &table.attributes[2];
    assert!(idle.arg("on_enter").is_none());
    assert!(idle.arg("in_state").is_some());
}

/// A variant with a payload, a name that is no variant, and an attribute on
/// the wrong kind of declaration are compile errors.
#[test]
fn enum_attribute_errors() {
    let decl = r#"
host "flow" {
    attribute state on enum;
    attribute stage(on_enter: enum?) on fn;
}
enum Phase { Ready, Hit(int) }
"#;
    for (body, code) in [
        (
            "@stage(on_enter: Phase::Hit)\nfn f() {}",
            "attribute_bad_argument",
        ),
        (
            "@stage(on_enter: Phase::Gone)\nfn f() {}",
            "attribute_bad_argument",
        ),
        ("@stage(on_enter: 3)\nfn f() {}", "attribute_bad_argument"),
        ("@state\nstruct S { n: int = 0 }", "attribute_wrong_target"),
        ("@stage\nenum E { A }", "attribute_wrong_target"),
    ] {
        let src = format!("{decl}{body}\nfn main() {{}}\n");
        let code_found = match Engine::new().compile(&src, "flow.cdl") {
            Ok(_) => panic!("{body} compiles, and should not"),
            Err(d) => d.code,
        };
        assert_eq!(code_found, code, "{body}");
    }
}
