# A struct crosses the embedding boundary as Value::Struct, not Value::Map

`Value` has a new variant, `Value::Struct { name, fields }`, carrying the
struct's name and its fields in declaration order. A struct that leaves a
script, as a call's result, inside a list, or as the argument of a host
function, arrives as one. It used to arrive as a `Value::Map` of its fields,
with no name.

This affects an embedding host that reads a struct back from `call`, or that
registers a host function taking a struct a `host` block declares. A `match` on
`Value` that lists every variant needs an arm for the new one.

Before:

```rust
let opts = args[3].as_map().expect("a record");
let cwd = opts["cwd"].as_str().unwrap_or(".");
```

After:

```rust
let cwd = args[3].field("cwd").and_then(Value::as_str).unwrap_or(".");
```

A struct now crosses inward too: `call` takes a `Value::Struct` for a struct
parameter, and an artifact exports a function that takes a struct.

Three functions of `candela_vm::embed` that a host rarely calls directly take
the program's struct table as a new last argument: `value_matches_type`,
`is_host_callable_type` and `marshal_args`. `marshal_value` is no longer
public; `marshal_args` is the way to write host values into a program's
registers.
