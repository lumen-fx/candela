# Candela standard library: design

This is the design for the first version of the Candela standard library. It
records what the language can express today, how the library is laid out and
resolved, and which pieces are deferred behind language decisions the owner still
needs to make.

## Three tiers

The standard library is split by what implements each piece.

- Built-ins live in the interpreter as Rust host functions: `print`, the `int` /
  `float` / `str` / `bool` conversions, `input`, `range`, `argv`, `exit`,
  `throw`, and the string / array / map / float methods (`split`, `uppercase`,
  `len`, `contains`, `sort`, `push`, `sqrt`, `floor`, `round`, `abs`, and so on).
  These are Rust because the interpreter is Rust and inherits Rust's portable,
  safe IO, filesystem, and time across Linux, macOS, and Windows. They are not
  part of the shipped library and are not reimplemented in C.
- C-backed modules reuse a mature C library through Candela's dynamic-library
  FFI: `math`, `time`, and `random`. Their C sources live in `libs/std_src` and
  are compiled to a shared object per platform by the release workflow.
- Candela modules are written in `.cdl` and build on Candela's own types:
  `string`, `list`, `convert`, and `assert`.

## What the language can express today

The library was designed against the language as it actually is, verified by
running programs, not against an assumed feature set.

- Polymorphism through compile-time monomorphization works. A single `fn sum(arr)`
  specializes to `int[]` or `float[]` at each call site. This is what makes the
  `list` and `string` helpers generic without any annotations. Reductions seed
  their accumulator from the first element (`let total = arr[0]`) so the same
  definition serves every numeric element type.
- First-class functions do not exist. A function name cannot be passed as a
  value; `apply(dbl, 5)` fails with "cannot find variable dbl". The parser
  accepts an anonymous-function term, but the compiler lowers it to null, so
  closures are not usable.
- Enums do not exist. There is `struct`, `impl` (methods that lower to free
  functions taking `self`), and `match`, where `match` is sugar for an
  equality-chained `if`.
- Structs are not generic. A field has a fixed declared type, so a struct field
  typed `int` rejects a `float`. There is no way to write a struct that holds an
  arbitrary payload.
- Errors are strings raised by `throw` and caught by `try` / `catch`.

## Import resolution and ship layout

There are two import forms.

- A namespaced import, `import std::string;`, is the form for the standard
  library. It binds the module under `string::` and the resolver maps it to a
  `.cdl` file in the shipped library directory (`std::string` -> `std/string.cdl`).
  This is the default a normal user reaches for, and it needs nothing set.
- A path-literal import, `import "./local.cdl";`, is for a user's own files. It
  resolves relative to the importing file first, then falls back to the shipped
  library directory.

The shipped library directory is the single source of truth for where std lives.
By default it is `libs/` beside the running executable, found by canonicalizing
the executable's own path. This is the ship-beside-the-toolchain layout: the
installer places the binary and `libs/` together, so `import std::string` works
from any working directory with nothing configured. Namespaced imports resolve
against this directory only; they are never source-relative, so the working
directory never matters.

`CANDELA_LIB_PATH` overrides the directory as an escape hatch for source
checkouts and custom builds where the binary is not laid out like an install. It
names the `libs/` directory that holds `std/` and, for the C-backed modules,
`std_src/`. It is not part of normal use and a normal `candela run` never needs
it.

Because a C-backed module names its shared object with a path relative to its own
`.cdl` file (`../std_src/math/math`), that path resolves correctly once the module
itself is found, under either the default or the override.

### Install layout

`install.sh` unpacks the release archive, which carries the binary and the whole
`libs/` tree, into one directory (`/usr/local/lib/candela` on Linux,
`/Library/Candela` on macOS) and symlinks the binary onto the PATH. The result is
`<install-dir>/candela` next to `<install-dir>/libs/std`, which is exactly what
the resolver's default looks up. The installer verifies `libs/std` is present so
a normal `candela run` using std works immediately, with nothing set. The
resolver's default path and the installer's destination are the same location by
construction, so they cannot drift.

## Whole-program `.cdlb` model

Resolution happens at compile time. When a program that imports the library is
built to a `.cdlb` artifact, every imported `.cdl` module is linked into the
single image, so the artifact runs under `candela-vm` with no source tree
present. This was verified: the `string`, `list`, `convert`, and `assert` tests
build to `.cdlb` and run from an unrelated directory with the source tree absent.

The C-backed modules do not yet satisfy this model. A dynamic-library import is
captured by reference, and for `math` / `time` / `random` that reference is a
path relative to the source (`../std_src/math/math`). `candela-vm` cannot
re-resolve a source-relative path once the source tree is gone, and it opens the
library eagerly at load, so merely importing such a module makes the artifact
fail to load away from the build tree. See the math note below.

## The two prerequisites

### Collection higher-order functions

`map`, `filter`, `reduce`, and `sort_by` need to receive a function. Candela has
no first-class functions and no working closures, and its `sort` takes no
comparator, so these cannot be expressed today in any idiom. They are deferred.
Adding first-class functions or closures is a language decision for the owner;
this pass does not add them.

### Option and result

`option` and `result` want a sum type. Candela has no enums, and its structs are
not generic, so a struct-convention `Option` cannot hold an arbitrary payload
(a `value` field typed `int` rejects a `float`). These are deferred. Adding
enums, or generic struct fields, is a language decision for the owner; this pass
does not add either speculatively.

## Math approach

The owner's stated preference was to bind libm through the FFI under the logical
name `m`. On this Linux box that fails: `/usr/lib/libm.so` is a GNU ld linker
script, not a shared object, so opening the logical name `m` fails at both
compile time and run time. Only `libm.so.6` is directly openable, and the
logical-name mapping does not target it.

The repository already takes a more robust variant of the same idea: a small C
wrapper (`libs/std_src/math/math.c`) forwards to libm's functions and is compiled
to a shared object that the module loads by path. This keeps the owner's
"reuse a mature C library through the FFI" intent and works in the interpreter,
which was verified (`cos(0)` is 1, `log(e)` is 1). This pass extends the wrapper
with the requested `sqrt`, `pow`, `floor`, `ceil`, `round`, plus `trunc`,
`fmod`, `copysign`, and the constants `pi`, `e`, and `tau`.

The open tradeoff is the `.cdlb` model above: the path-referenced wrapper does
not re-resolve under `candela-vm` away from the build tree. Two approaches close
that, and the choice belongs to the owner:

- Native math built-ins. Rust's `f64` covers `sin`, `cos`, `tan`, `exp`, `ln`,
  `powf`, and the rest through the platform libm linked at compile time, with no
  runtime open. Candela already does exactly this for `sqrt`, `floor`, `round`,
  and `abs` as VM operations, so these functions inline into `.cdlb` and run
  everywhere. This is the most robust option and the recommendation. It changes
  the VM, so it is flagged rather than taken here against the stated FFI
  preference.
- Keep the C wrapper but give it a logical name and install it where the system
  loader searches (or with an rpath). The `.cdlb` then captures a logical name
  the loader re-resolves at run time. This keeps the FFI but adds a
  distribution requirement for the shared object's location.

## Modules shipped versus deferred

Shipped:

- `math` (C-backed): the existing transcendental set plus `sqrt`, `pow`,
  `floor`, `ceil`, `round`, `trunc`, `fmod`, `copysign`, and `pi` / `e` / `tau`.
- `string` (Candela): `substring`, `char_at`, `is_empty`, `capitalize`, `lines`,
  `pad_left`, `pad_right`, `count`.
- `list` (Candela): `first`, `last`, `is_empty`, `sum`, `product`, `min`, `max`,
  `index_of`, `count`, `unique`, `chunk`, `take`, `drop`.
- `convert` (Candela): `to_int`, `to_float`, `to_string`, `to_bool`.
- `assert` (Candela): `assert`, `assert_msg`, `assert_true`, `assert_false`,
  `assert_eq`, `assert_ne`.

Deferred, with the blocker:

- `map` / `set`: the VM exposes only `get` and `insert` on maps. There is no
  empty-map literal, no `len`, `keys`, `values`, `contains`, or iteration, so
  these cannot be built yet. The blocker is missing VM primitives, not a language
  feature.
- `json`: there is no parse primitive to build on.
- `iter` higher-order functions (`map`, `filter`, `reduce`, `sort_by`): blocked
  on first-class functions or closures.
- `option` / `result`: blocked on enums or generic struct fields.

## Notes for the owner

Two pre-existing VM defects surfaced while testing and shaped how the tests are
written. They are not caused by the library and are worth a separate look:

- Throwing from a called function and catching it in the caller hangs the VM.
  Only a `throw` in the same block as the `try` is caught. The `assert` module
  therefore aborts the run on failure rather than being caught by a harness.
- Passing an array-of-arrays as a function argument reads back as null. Nested
  results in the tests are checked by indexing into them instead.

A third effect appears only in the release build: a program with many distinct
type specializations of one imported function in a single compilation can abort
with an illegal instruction. The tests keep each program small to stay clear of
it.
