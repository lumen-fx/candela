# Language expansion: the features that unblock the standard library

This document records the design for a set of language features candela needs
before its standard library can grow: first-class functions, native enums,
a dynamic value, json, and fuller map/set primitives. For each feature it states
the chosen minimal delta, where it lands in the compiler or VM, its effect on the
`candela-vm` binary size, and how it stays inside the size budget.

## Size budget

`candela-vm` is the shipped runtime. The ceiling is 1.5 MiB hard; the target is
to stay near 1 MiB. The baseline before this work is about 0.74 MiB, so there is
headroom, but the guiding rule still holds: prefer deltas that live in the
compiler (which ships in the `candela` toolchain, not in `candela-vm`) over
deltas that add runtime value types, instructions, or heap pools to the VM.

The single most budget-friendly result below is that first-class functions and
the collection higher-order methods need no VM change at all: they are resolved
entirely in the compiler by monomorphization, so the runtime binary is unchanged.

## Status

Implemented: first-class function references (feature 1) -- named and anonymous
functions passed to higher-order functions, non-capturing -- and the collection
higher-order functions in `std::list` (feature 6), available both as free
functions (`list::map(arr, f)`) and as methods on arrays (`arr.map(f)`,
`arr.filter(f)`, `arr.reduce(init, f)`, `arr.each(f)`, `arr.find(f)`,
`arr.any(f)`, `arr.all(f)`, `arr.sort_by(f)`, plus the reductions and slicers
`first`/`last`/`sum`/`min`/`max`/`take`/`drop`/...). The method spelling needs no
explicit import: an auto-prelude implicitly resolves `std::list`. These add
nothing to `candela-vm`.

Implemented: native enums and payload-binding match (2), and `option`/`result`
as std enums (7). An `enum Name { Variant, Variant(T), ... }` declares a native
tagged union; `Name::Variant(args)` and bare `Some(x)`/`None` construct values;
`match` binds variant payloads. `option` (`Some`/`None`) and `result`
(`Ok`/`Err`) ship as `.cdl` enums in `libs/std` with `is_some`/`unwrap`/
`unwrap_or`/`map` and `is_ok`/`unwrap_err`/... helpers, usable as free functions
(`option::unwrap(o)`) or methods (`o.unwrap()`). A payload typed `any` holds a
value of any type. These inline into a `.cdlb` and run under `candela-vm`.

Designed, not yet implemented: the `Any` type as a full feature (3, only the
`any`-as-payload subset is used by option/result), json (4), and the fuller
map/set primitives (5). The rest of this document is the design each of those
follows when it lands.

## 1. First-class functions (compile-time function references)

### What it enables

Passing a function by name, or an anonymous function, as an argument:

    fn double(x) { return x * 2 }
    let ds = [1, 2, 3].map(double)
    let sq = [1, 2, 3].map(fn(x) { return x * x })

### Chosen minimal delta

candela already monomorphizes: every call specializes the callee for the exact
argument types at the call site, and functions are inlined into one instruction
stream. A function argument is therefore modeled as a compile-time value, not a
runtime one:

- `DataType::Fn(fn_id)` (already present) is the static type of a function
  reference. A bare identifier that names a function infers to `Fn(id)`; an
  anonymous function is hoisted to a synthetic top-level function and also
  infers to `Fn(id)`.
- When a `Fn(id)` argument reaches a callee, the compiler does not move it into
  a register. Instead it binds the parameter name to `SymbolKind::Fn(id)` in the
  callee's symbol table for the duration of that specialization. A call to the
  parameter inside the body (`f(x)`) then resolves through the ordinary
  function-call path to the concrete `fn_id`. This machinery already exists in
  `compile_function` and `handle_user_function`; the missing pieces are the two
  inference rules above and anonymous-function hoisting.

Anonymous functions are non-capturing. A synthetic function sees only its own
parameters, globals, and other functions, exactly like a named function. An
anonymous function that references an enclosing local fails to compile with an
"unknown variable" error at the point of use. Capturing closures are a possible
later addition (a heap closure object plus an indirect-call instruction); they
are deliberately out of scope here because the collection methods do not need
them and they would add a runtime value type.

### Size impact

None on `candela-vm`. Everything is resolved in the compiler; the emitted
bytecode contains only ordinary calls. The cost is a small amount of compiler
code (inference plus anon-fn hoisting) which ships in the `candela` toolchain.

### Upstream

Horace's upstream `keel` has a work-in-progress branch (commit `4f08db4`,
"Started implementing HOFs") that sketched this same compile-time approach:
model a function argument as `DataType::Fn(fn_id)` and bind the parameter name to
that id in the callee's symbol table. That branch predates candela's crate and
parser reorganisation and its parser still rejected the syntax, so none of its
code was ported verbatim; the direction it set is followed here and the
implementation is written fresh against the current tree. The scaffolding the
current tree already carried (skipping function-typed arguments during the call,
binding them as symbols in `compile_function`) comes from that same lineage. The
keel/Horace attribution and Apache-2.0 terms in `NOTICE` and `LICENSE` continue
to cover this.

### Trade-off

Because a function reference is compile-time, it cannot be stored in a variable,
an array, or a map, and cannot be returned across a dynamic boundary. It can only
be passed as a call argument and called there. This covers the collection
higher-order methods and user-written higher-order functions, which is the goal.
A true runtime function value is the follow-up if first-class storage is needed.

## 6. Collection higher-order methods (built on feature 1)

Array operations are methods, per the owner directive:

    arr.map(f)  arr.filter(f)  arr.reduce(init, f)  arr.each(f)
    arr.find(f) arr.any(f)     arr.all(f)           arr.sort_by(f)

### Chosen minimal delta

The methods are candela source in `libs/std/list.cdl`, written against feature 1
(each takes a function parameter and calls it). The array-method dispatch
(`methods.rs` array fall-through) recognizes these names on an array receiver and
lowers `arr.map(f)` to the library call `list::map(arr, f)`, so the method form
works without the caller importing the module. The library remains usable
directly as free functions as well.

The module resolves without an explicit import through an auto-prelude: when the
top-level file is parsed, `std::list` is loaded as an implicit `list` child
namespace (resolved through the same `CANDELA_LIB_PATH` / exe-relative `libs/`
path as a namespaced import). Resolution is best-effort, so an embedding host
with no `libs/` tree still compiles; array methods there simply fall back to the
normal unknown-method error. `find` routes to the module only when its argument
is a function value (the predicate form); `find(value)` stays on the builtin
index search.

### Size impact

None on `candela-vm`: the methods are candela code, compiled and inlined like any
user function, and the whole-program `.cdlb` inlines them with no source tree.

## 3. Dynamic / any value

### What it enables

Values whose type is not known until runtime: json parse output, and map values
that are not all the same type.

### Chosen minimal delta

The runtime value (`Data`) is already a NaN-boxed tagged union: every value
carries its own type at runtime, so the VM already holds "any value" without
change. What restricts heterogeneity is the compiler's static type system. The
delta is one type, `DataType::Any`, that is assignment-compatible in both
directions (a top type): anything may be used where `Any` is expected, and an
`Any` may be used where anything is expected, with runtime dispatch already
guaranteed by the tagged representation. It is written `any` in source.

### Size impact

None on `candela-vm`. `Any` exists only in the compiler's type checker; the
runtime representation is the existing `Data`.

## 4. json

### Chosen minimal delta

A native parse builtin implemented in Rust that turns a json string into existing
`Data`: objects become maps (`{string: any}`), arrays become arrays, and scalars
become the existing int/float/string/bool/null. No new value type is needed
because every json shape maps onto a value candela already has, with `Any` (from
feature 3) as the map value type. A `std::json` candela module wraps this with
`parse` and `stringify`; `stringify` walks a value with the existing formatting
path. Parse errors surface as catchable candela errors.

### Size impact

Small and confined to the `candela` toolchain for the parser front end. The
native parse routine is modest Rust; it reuses the existing map/array/string
allocation paths in the VM rather than adding new ones.

## 5. Map and set primitives

### What it enables

    let m = {}          // empty map literal
    m.len()  m.keys()  m.values()  m.contains(k)
    for k in m { ... }  // iteration
    // set: a thin layer over map

### Chosen minimal delta

Maps already exist as a runtime value with `get`/`insert`. The additions are:

- `len`, `contains`: `len` already covers maps; `contains` reuses the existing
  map lookup and returns a bool.
- `keys`, `values`: build an array from the map's entries using the existing
  array allocation path. These are two small VM operations (or one parameterized
  operation) that read the map pool and emit an array.
- Iteration: `for k in m` lowers over the key array, so it reuses `keys` and the
  existing array for-loop rather than adding a map-specific loop.
- set: a thin layer over map (keys are the members, values are a unit). This
  avoids a second heap pool and a second GC path. It is a candela module plus a
  small amount of method sugar, not a new runtime value. The justification for
  reusing map rather than a dedicated primitive is purely size: a separate set
  value would duplicate the map pool, its GC, and its NaN-box tag for no
  behavior that map keys do not already provide.

### Size impact

Small on `candela-vm`: a couple of map-reading operations that emit arrays. No
new heap pool (set rides the map pool). Iteration and set add no VM code.

## 2. Native enums and pattern matching

### What it enables

A real tagged union with payload-binding match, so `option` and `result` are
ordinary library enums rather than sentinel values:

    enum Shape { Circle(float), Rect(float, float), Unit }
    match s {
        Circle(r)   => ...,
        Rect(w, h)  => ...,
        Unit        => ...,
    }

### Chosen minimal delta

Today `match` is sugar for an equality-chained `if`; it cannot bind a payload.
A native enum needs a runtime value that carries a variant tag and an optional
payload.

- Value representation: a new NaN-box tag for an enum value. The tight 3-bit tag
  space has one unused encoding (the all-zero type field on the quiet-NaN base),
  which is claimed for the enum tag; its payload is an index into an enum pool.
  An enum pool entry holds `{enum_type_id, variant_tag, payload: Data}`. A
  nullary variant (like `Unit` or `None`) still allocates a pool slot for
  uniformity, or is special-cased to an inline tag-only encoding to avoid the
  allocation; the inline form is preferred to keep the common `option` case
  allocation-free.
- `match` becomes a real variant match: it reads the variant tag, jumps to the
  arm, and binds the payload registers for that arm. Equality-based arms (the
  current behavior for non-enum scrutinees) are retained so existing matches keep
  working.

### Implementation plan (minimal-VM shape)

The landing shape that keeps the `candela-vm` delta smallest reuses the existing
object pool for storage rather than adding a second heap pool and GC:

- Value: a new NaN tag `NAN_ENUM` on the free all-zero type field
  (`NAN_ENUM == NAN_BASE`, distinct from every existing tag and from computed
  float NaNs, whose sign bit is clear). The 48-bit payload packs
  `enum_type_id` (high 16 bits) and an object-pool index (low 32), exactly like
  the struct encoding. The pool entry is a `Vec<Data>` whose element 0 is the
  variant tag (an int) and elements `1..` are the payload, so nullary variants
  are a one-element `[tag]` vector (uniform allocation first; the inline
  tag-only optimization for `None`/`Unit` is a follow-up).
- GC: `array_gc` treats an enum value like a struct -- a root when found in a
  register, and its non-scalar children are marked -- so no new GC path is added.
- Instructions: one new construction op, `CloneEnum(template_reg, dest_reg)`,
  mirroring `CloneStruct` but preserving the enum tag. Payload and tag access
  reuse `GetFieldStruct`/`SetFieldStruct` (they extract the low-32 object index
  regardless of the box tag; the `as_struct` debug assert is relaxed to accept an
  enum value). Construction is `CloneEnum` followed by `SetFieldStruct` writes,
  matching how structs are built from a compile-time template.
- Match dispatch lowers to a tag-compare chain: evaluate the scrutinee once, read
  element 0 as the variant tag, then reuse the existing conditional-jump / `if`
  machinery to select the arm; each arm prefixes payload bindings that lower to
  `GetFieldStruct` reads into fresh locals. This delivers payload-binding match
  without a dedicated match opcode, keeping the VM change to the single
  `CloneEnum` op.
- Registry and artifact: an `enums` table (`EnumType { name, variants:
  [(name, arity)] }`) parallel to `structs`, added to `DataType::Enum(u16)` in
  the runtime `DataType`, threaded through `execute()` and `Data::format()` (so
  enum values print as `Variant(payload...)`), and serialized into the `.cdlb`
  image behind a `FORMAT_VERSION` bump.
- Compiler front end: an `enum` keyword and declaration, variant construction
  (`Name::Variant(args)`), and `match` patterns with payload identifiers in the
  parser; `DataType::Enum` inference, variant/arity resolution, and arm type
  checking in the type system; declaration registration and the construction /
  match lowering in the code generator.

Status: landed. The value uses the `NAN_ENUM` tag on the free all-zero type
field, storing an object-pool index like a struct; the pool entry holds the
variant tag at element 0 and the payload at `1..`. One new `CloneEnum`
construction op was added; tag and payload access reuse
`GetFieldStruct`/`SetFieldStruct`, and `match` lowers to a tag-compare chain over
the existing conditional-jump machinery. GC traces enum values like structs (no
new pool). The `.cdlb` format carries an `enums` table behind a version bump.
Nullary variants currently allocate a one-element pool slot; the inline
tag-only optimization is a follow-up.

### Size impact

This is the feature that grows the VM: a value tag, a construction instruction,
and enum handling in equality, formatting, and GC tracing. It reuses the object
pool and its GC rather than adding a second heap, so the runtime binary stays
near its prior size, comfortably under the 1 MiB target.

## 7. option and result

Landed. `option` (`Some`/`None`) and `result` (`Ok`/`Err`) are ordinary candela
enums in `libs/std/option.cdl` and `libs/std/result.cdl`, with helpers written
as candela functions and `impl` methods on top of the native enum and feature 1:
`is_some`/`is_none`/`unwrap`/`unwrap_or`/`map` for option, and
`is_ok`/`is_err`/`unwrap`/`unwrap_err`/`unwrap_or` for result. Each works as a
free function (`option::unwrap(o)`) or a method (`o.unwrap()`). The payload is
typed `any`, so it holds a value of any type; direct arithmetic on an extracted
`any` value needs a concrete type and is not available (match and rebind, or use
type-agnostic operations like `str`). No VM change beyond feature 2.

## Shipping

The pure-candela parts (the list methods, `std::json`, `std::map`/`std::set`
helpers, `option`/`result`) ship as `.cdl` files beside the toolchain, resolved
by the existing zero-config `import std::x` path, and inline into a `.cdlb`
whole-program image so a shipped program runs under `candela-vm` with no source
tree.
