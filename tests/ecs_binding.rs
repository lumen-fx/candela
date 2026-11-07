//! A small column-store engine bound to candela through attributes, the way an
//! ECS engine binds script systems.
//!
//! Nothing here ships. The engine and its `ecs` package exist to drive the
//! generic seams end to end: a `host` block declares the attributes, a script
//! puts them on its structs and functions, the engine lists them from the
//! attribute table without running anything, builds one row list per query,
//! calls each system once per frame with `call_in_out`, and copies back only
//! what each system declares it writes. The same binding runs a script the
//! compiler holds (`Program`) and an artifact `candela build` wrote
//! (`RuntimeProgram`).
//!
//! The binding's rules are its own, not candela's: a list of structs whose
//! fields are all components is a query, a list of a `message` struct is the
//! frame's messages, a `resource` struct is a resource, and `Commands` is the
//! command buffer. A tuple is a struct whose fields are `0`, `1`, ..., so a
//! list of tuples of components is a query under the same rule.

use candela::AttributeTable;
use candela::AttributeTarget;
use candela::AttributeValue;
use candela::Engine;
use candela::HostRegistry;
use candela::Program;
use candela::RuntimeProgram;
use candela::Value;
use candela::load_program;
use candela::read_attributes;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

/// The engine's own package: the attribute vocabulary and the command buffer.
const ECS_PACKAGE: &str = r#"
host "ecs" {
    attribute component on struct;
    attribute resource on struct;
    attribute message on struct;
    attribute query(with: type[] = [], without: type[] = []) on struct;
    attribute system(writes: type[] = [], after: fn[] = [], before: fn[] = [],
                     schedule: Schedule = Schedule::Update) on fn;
}

enum Schedule { Startup, Update }

struct Commands { ops: any[] }
impl Commands {
    fn spawn(self, bundle: any[]) { self.ops.push(bundle); }
}
"#;

/// The game: move by velocity, spawn a spark per hit, count hits into a
/// score, and one system that writes a component it does not declare.
const GAME: &str = r#"
import "ecs.cdl";

@ecs::component struct Position { x: float, y: float }
@ecs::component struct Velocity { x: float, y: float }
@ecs::component struct Spark { ttl: float }
@ecs::message struct Hit { x: float, y: float }
@ecs::resource struct Score { value: int }
@ecs::resource struct Time { dt: float }

@ecs::query(without: [Spark])
struct Mover { pos: Position, vel: Velocity }

@ecs::system(writes: [Position])
fn movement(movers: Mover[], time: Time) {
    for m in movers {
        m.pos.x += m.vel.x * time.dt;
        m.pos.y += m.vel.y * time.dt;
    }
}

@ecs::system(after: [movement])
fn spawn_on_hit(hits: Hit[], cmd: ecs::Commands) {
    for h in hits {
        let bundle: any[] = [];
        bundle.push(Spark { ttl: 0.5 });
        bundle.push(Position { x: h.x, y: h.y });
        cmd.spawn(bundle);
    }
}

@ecs::system(writes: [Score], after: [spawn_on_hit])
fn count_hits(hits: Hit[], score: Score) {
    score.value += hits.len();
}

// Writes Velocity without declaring it, so the engine drops the write.
@ecs::system(before: [movement])
fn drag(movers: Mover[]) {
    for m in movers {
        m.vel.x = 0.0;
    }
}

@ecs::system(writes: [Score], schedule: ecs::Schedule::Startup)
fn setup(score: Score) {
    score.value = 100;
}

fn main() {}
"#;

/// What a binding needs from either kind of program.
trait Script {
    fn attributes(&self) -> AttributeTable;
    fn call_in_out(&mut self, name: &str, args: &mut [Value]) -> Result<Value, String>;
}

impl Script for Program {
    fn attributes(&self) -> AttributeTable {
        Self::attributes(self)
    }
    fn call_in_out(&mut self, name: &str, args: &mut [Value]) -> Result<Value, String> {
        Self::call_in_out(self, name, args).map_err(|d| d.message)
    }
}

impl Script for RuntimeProgram {
    fn attributes(&self) -> AttributeTable {
        Self::attributes(self)
    }
    fn call_in_out(&mut self, name: &str, args: &mut [Value]) -> Result<Value, String> {
        Self::call_in_out(self, name, args).map_err(|e| e.to_string())
    }
}

/// One parameter of a system, as the binding reads it.
#[derive(Debug, Clone)]
enum Param {
    /// A list of rows of the struct `row`, one field per component.
    Query {
        row: String,
        fields: Vec<(String, String)>,
        without: Vec<String>,
    },
    /// The frame's messages of this struct.
    Messages(String),
    /// The resource of this struct.
    Resource(String),
    /// The command buffer.
    Commands,
}

#[derive(Debug, Clone)]
struct System {
    name: String,
    params: Vec<Param>,
    writes: Vec<String>,
    schedule: String,
}

/// The engine: one column per component, resources, this frame's messages,
/// and the frame each component of each entity last changed in.
#[derive(Default)]
struct World {
    entities: usize,
    columns: BTreeMap<String, Vec<Option<Value>>>,
    resources: BTreeMap<String, Value>,
    messages: BTreeMap<String, Vec<Value>>,
    changed: BTreeMap<(String, usize), u32>,
    frame: u32,
}

impl World {
    fn spawn(&mut self, components: Vec<Value>) -> usize {
        let entity = self.entities;
        self.entities += 1;
        for column in self.columns.values_mut() {
            column.push(None);
        }
        for component in components {
            let name = component
                .struct_name()
                .expect("a component is a struct")
                .to_owned();
            let column = self
                .columns
                .entry(name.clone())
                .or_insert_with(|| vec![None; entity + 1]);
            column[entity] = Some(component);
            self.changed.insert((name, entity), self.frame);
        }
        entity
    }

    fn get(&self, component: &str, entity: usize) -> Option<&Value> {
        self.columns.get(component)?.get(entity)?.as_ref()
    }

    /// Writes `value` into a component, marking it changed only when it
    /// differs from what the column held.
    fn set(&mut self, component: &str, entity: usize, value: Value) {
        let slot = &mut self.columns.get_mut(component).expect("a column")[entity];
        if slot.as_ref() != Some(&value) {
            *slot = Some(value);
            self.changed
                .insert((component.to_owned(), entity), self.frame);
        }
    }
}

/// The engine's view of a script: its systems in run order, per schedule.
struct Binding {
    systems: Vec<System>,
}

fn struct_names(value: &AttributeValue, table: &AttributeTable) -> Vec<String> {
    value
        .as_list()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| match v {
            AttributeValue::Type(ty) => Some(table.type_name(ty)),
            _ => None,
        })
        .collect()
}

fn fn_names(value: &AttributeValue) -> Vec<String> {
    value
        .as_list()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| match v {
            AttributeValue::Fn(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

impl Binding {
    /// Reads the attribute table and orders the systems by `after` and
    /// `before`.
    fn load(table: &AttributeTable) -> Self {
        let marked = |name: &str| -> Vec<u16> {
            table
                .named("ecs", name)
                .filter_map(|a| match a.target {
                    AttributeTarget::Struct(id) => Some(id),
                    AttributeTarget::Fn { .. } => None,
                })
                .collect()
        };
        let components = marked("component");
        let messages = marked("message");
        let resources = marked("resource");
        let queries: BTreeMap<u16, Vec<String>> = table
            .named("ecs", "query")
            .filter_map(|a| match a.target {
                AttributeTarget::Struct(id) => {
                    Some((id, struct_names(a.arg("without").unwrap(), table)))
                }
                AttributeTarget::Fn { .. } => None,
            })
            .collect();

        let mut systems = Vec::new();
        let mut edges: Vec<(String, String)> = Vec::new();
        for attr in table.named("ecs", "system") {
            let AttributeTarget::Fn { name, params } = &attr.target else {
                panic!("system goes on fn");
            };
            let params = params
                .iter()
                .map(|(param, ty)| {
                    let row_of = |ty: &candela::DataType| match ty {
                        candela::DataType::Array(Some(inner)) => match **inner {
                            candela::DataType::Struct(id) => Some(id),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(id) = row_of(ty) {
                        let info = &table.structs[id as usize];
                        if messages.contains(&id) {
                            return Param::Messages(info.name.clone());
                        }
                        // Any struct whose fields are all components is a row:
                        // a `query` struct, or a tuple of components.
                        let fields: Option<Vec<(String, String)>> = info
                            .fields
                            .iter()
                            .map(|(field, ty)| match ty {
                                candela::DataType::Struct(c) if components.contains(c) => {
                                    Some((field.clone(), table.structs[*c as usize].name.clone()))
                                }
                                _ => None,
                            })
                            .collect();
                        let fields = fields.unwrap_or_else(|| {
                            panic!(
                                "parameter {param} of {name} is a list of {}, which is no query",
                                info.name
                            )
                        });
                        return Param::Query {
                            row: info.name.clone(),
                            fields,
                            without: queries.get(&id).cloned().unwrap_or_default(),
                        };
                    }
                    match ty {
                        candela::DataType::Struct(id) if resources.contains(id) => {
                            Param::Resource(table.structs[*id as usize].name.clone())
                        }
                        candela::DataType::Struct(id)
                            if table.structs[*id as usize].name == "Commands" =>
                        {
                            Param::Commands
                        }
                        other => panic!(
                            "parameter {param} of {name} is a {}",
                            table.type_name(other)
                        ),
                    }
                })
                .collect();
            for after in fn_names(attr.arg("after").unwrap()) {
                edges.push((after, name.clone()));
            }
            for before in fn_names(attr.arg("before").unwrap()) {
                edges.push((name.clone(), before));
            }
            let schedule = match attr.arg("schedule").unwrap() {
                AttributeValue::Variant { enum_id, name } => {
                    assert_eq!(table.enums[*enum_id as usize].name, "Schedule");
                    name.clone()
                }
                other => panic!("schedule is {other:?}"),
            };
            systems.push(System {
                name: name.clone(),
                params,
                writes: struct_names(attr.arg("writes").unwrap(), table),
                schedule,
            });
        }

        // Kahn's order over the declared edges, ties broken by declaration.
        let mut ordered = Vec::new();
        let mut left = systems;
        while !left.is_empty() {
            let at = left
                .iter()
                .position(|s| {
                    !edges
                        .iter()
                        .any(|(from, to)| *to == s.name && left.iter().any(|o| o.name == *from))
                })
                .expect("the ordering has no cycle");
            ordered.push(left.remove(at));
        }
        Self { systems: ordered }
    }

    /// Runs every system of `schedule` once against `world`.
    fn run(&self, schedule: &str, script: &mut dyn Script, world: &mut World) {
        for system in self.systems.iter().filter(|s| s.schedule == schedule) {
            // Which entity each row of each query came from.
            let mut rows_of: Vec<Vec<usize>> = Vec::new();
            let mut args: Vec<Value> = Vec::new();
            for param in &system.params {
                let mut entities = Vec::new();
                args.push(match param {
                    Param::Query {
                        row,
                        fields,
                        without,
                    } => {
                        let mut rows = Vec::new();
                        for entity in 0..world.entities {
                            if without.iter().any(|c| world.get(c, entity).is_some()) {
                                continue;
                            }
                            let values: Option<Vec<(String, Value)>> = fields
                                .iter()
                                .map(|(field, component)| {
                                    Some((field.clone(), world.get(component, entity)?.clone()))
                                })
                                .collect();
                            if let Some(values) = values {
                                entities.push(entity);
                                rows.push(Value::Struct {
                                    name: row.clone(),
                                    fields: values,
                                });
                            }
                        }
                        Value::Array(rows)
                    }
                    Param::Messages(message) => {
                        Value::Array(world.messages.get(message).cloned().unwrap_or_default())
                    }
                    Param::Resource(resource) => world.resources[resource].clone(),
                    Param::Commands => Value::Struct {
                        name: String::from("Commands"),
                        fields: vec![(String::from("ops"), Value::Array(Vec::new()))],
                    },
                });
                rows_of.push(entities);
            }

            script
                .call_in_out(&system.name, &mut args)
                .unwrap_or_else(|e| panic!("{} failed: {e}", system.name));

            // Copy back what the system declares it writes, and nothing else.
            for ((param, arg), entities) in system.params.iter().zip(args).zip(rows_of) {
                match param {
                    Param::Query { fields, .. } => {
                        let Value::Array(rows) = arg else {
                            panic!("rows come back as a list")
                        };
                        for (row, entity) in rows.iter().zip(entities) {
                            for (field, component) in fields {
                                if system.writes.contains(component) {
                                    world.set(component, entity, row.field(field).unwrap().clone());
                                }
                            }
                        }
                    }
                    Param::Resource(resource) => {
                        if system.writes.contains(resource) {
                            world.resources.insert(resource.clone(), arg);
                        }
                    }
                    Param::Commands => {
                        let ops = arg.field("ops").and_then(Value::as_array).unwrap().clone();
                        for bundle in ops {
                            world.spawn(bundle.into_array().unwrap());
                        }
                    }
                    Param::Messages(_) => {}
                }
            }
        }
    }
}

fn point(name: &str, x: f64, y: f64) -> Value {
    Value::Struct {
        name: name.to_owned(),
        fields: vec![
            (String::from("x"), Value::Float(x)),
            (String::from("y"), Value::Float(y)),
        ],
    }
}

fn resource(name: &str, field: &str, value: Value) -> Value {
    Value::Struct {
        name: name.to_owned(),
        fields: vec![(field.to_owned(), value)],
    }
}

/// A directory holding the engine's package and the game, unique to `label`.
fn project(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("candela_ecs_{label}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ecs.cdl"), ECS_PACKAGE).unwrap();
    std::fs::write(dir.join("game.cdl"), GAME).unwrap();
    dir
}

/// Loads the game's systems, runs the startup schedule and two update frames,
/// and checks what the world holds after each.
fn drive(script: &mut dyn Script) {
    let table = script.attributes();
    let binding = Binding::load(&table);
    let order: Vec<&str> = binding.systems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        order,
        ["drag", "movement", "spawn_on_hit", "count_hits", "setup"]
    );
    let movement = &binding.systems[1];
    assert_eq!(movement.writes, ["Position"]);
    assert!(
        matches!(&movement.params[0], Param::Query { row, without, .. } if row == "Mover" && without == &["Spark"])
    );

    let mut world = World::default();
    world.resources.insert(
        String::from("Score"),
        resource("Score", "value", Value::Int(0)),
    );
    world.resources.insert(
        String::from("Time"),
        resource("Time", "dt", Value::Float(0.5)),
    );
    let mover = world.spawn(vec![
        point("Position", 0.0, 0.0),
        point("Velocity", 2.0, 4.0),
    ]);
    let still = world.spawn(vec![
        point("Position", 5.0, 5.0),
        point("Velocity", 0.0, 0.0),
    ]);
    let spark = world.spawn(vec![
        point("Position", 9.0, 9.0),
        point("Velocity", 1.0, 1.0),
        resource("Spark", "ttl", Value::Float(1.0)),
    ]);

    binding.run("Startup", script, &mut world);
    assert_eq!(
        world.resources["Score"],
        resource("Score", "value", Value::Int(100))
    );

    world.frame = 1;
    world.messages.insert(
        String::from("Hit"),
        vec![point("Hit", 1.0, 1.0), point("Hit", 3.0, 3.0)],
    );
    binding.run("Update", script, &mut world);

    // Moved by its velocity: `drag` zeroed the velocity it was handed, but it
    // declares no write, so the engine kept the old one.
    assert_eq!(
        world.get("Position", mover),
        Some(&point("Position", 1.0, 2.0))
    );
    assert_eq!(
        world.get("Velocity", mover),
        Some(&point("Velocity", 2.0, 4.0))
    );
    assert_eq!(world.changed[&(String::from("Position"), mover)], 1);
    // Written back with the same value, so not marked changed.
    assert_eq!(
        world.get("Position", still),
        Some(&point("Position", 5.0, 5.0))
    );
    assert_eq!(world.changed[&(String::from("Position"), still)], 0);
    // Left out of the query by `without: [Spark]`.
    assert_eq!(
        world.get("Position", spark),
        Some(&point("Position", 9.0, 9.0))
    );
    // One spark per hit, at the hit.
    assert_eq!(world.entities, 5);
    assert_eq!(world.get("Position", 3), Some(&point("Position", 1.0, 1.0)));
    assert_eq!(
        world.get("Spark", 4),
        Some(&resource("Spark", "ttl", Value::Float(0.5)))
    );
    assert_eq!(
        world.resources["Score"],
        resource("Score", "value", Value::Int(102))
    );

    world.frame = 2;
    world.messages.clear();
    binding.run("Update", script, &mut world);
    assert_eq!(
        world.get("Position", mover),
        Some(&point("Position", 2.0, 4.0))
    );
    assert_eq!(world.entities, 5);
    assert_eq!(
        world.resources["Score"],
        resource("Score", "value", Value::Int(102))
    );
}

/// The binding drives a script the compiler holds.
#[test]
fn systems_run_on_a_compiled_program() {
    let dir = project("compiled");
    let path = dir.join("game.cdl");
    let mut program = Engine::new()
        .compile(GAME, path.to_str().unwrap())
        .unwrap_or_else(|d| panic!("{}", d.message));
    drive(&mut program);
}

/// The same binding drives the artifact `candela build` writes, loaded by the
/// VM alone, and lists the systems from the file before loading it.
#[test]
fn systems_run_on_a_built_artifact() {
    let dir = project("built");
    let out = dir.join("game.cdlb");
    let status = Command::new(env!("CARGO_BIN_EXE_candela"))
        .arg("build")
        .arg(dir.join("game.cdl"))
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&out).unwrap();

    let listed = read_attributes(&bytes).unwrap();
    let systems: Vec<String> = listed
        .named("ecs", "system")
        .map(|a| match &a.target {
            AttributeTarget::Fn { name, .. } => name.clone(),
            AttributeTarget::Struct(_) => unreachable!(),
        })
        .collect();
    assert_eq!(
        systems,
        ["movement", "spawn_on_hit", "count_hits", "drag", "setup"]
    );

    let mut program = load_program(&bytes, &HostRegistry::new()).unwrap();
    program.run();
    drive(&mut program);
}
