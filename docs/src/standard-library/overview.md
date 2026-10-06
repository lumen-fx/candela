# Standard library overview

The candela standard library is a set of modules. The ones that give the
built-in types their methods, and the `Option` and `Result` enums, are the
prelude: every program has them with no import. The rest you import by path.

## How std ships

The library ships as candela source. The toolchain installs a `libs` directory
beside the `candela` executable:

- `libs/std/` holds one `.cdl` file per module: `assert.cdl`, `fs.cdl`,
  `hash.cdl`, `json.cdl`, `list.cdl`, `map.cdl`, `math.cdl`,
  `option.cdl`, `random.cdl`, `result.cdl`, `set.cdl`, `string.cdl`,
  `time.cdl`.
- `libs/std_src/` holds the C sources and the compiled dynamic libraries for the
  four modules that call into native code: `hash`, `math`, `random`, and
  `time`.

Set `CANDELA_LIB_PATH` to point at a different `libs` directory. The variable
names the directory that contains `std/` and `std_src/`, and it overrides the
default location for library imports. A Rust host that embeds candela names it
with `Engine::with_lib_dir` instead; see
[embedding](../integration/embedding.md). The prelude does not depend on it:
the compiler carries its own copy, so a program has the prelude wherever it
compiles.

Because the modules are ordinary source files, the compiler links the ones you
import into your program. A `.cdlb` artifact built from a program that imports
only pure-candela modules runs under `candela-vm` with no library directory
present. The `hash`, `math`, `random`, and `time` modules bind a dynamic
library, so a `.cdlb` that uses them records the binding recipe and needs the
`libs` directory when it runs; `candela-vm` finds it the way the compiler does,
through `CANDELA_LIB_PATH` or beside its own binary. See
[artifacts](../reference/artifacts.md).

## In a browser

The WebAssembly build has no file system and loads no dynamic library. It
carries `hash`, `math`, `random` and `time` inside itself and runs their native
half on its own functions, so `import "std/math";` works there as it
does on a desktop, with the same functions. A `.cdlb` built on a desktop that
imports them runs in the browser as well. The prelude is there too. The other
modules do not import in a browser.

## Importing a module

A library import is a quoted path with no file extension. The resolver appends
`.cdl` and looks the file up in the shipped library directory, never next to
the importing file, so it works from any working directory:

```rust
import "std/json";

fn main() {
    print(json::stringify(json::parse("[1, 2]")));
}
```

An import binds the module under its name (`import "std/json";` then
`json::parse(text)`), and a list of items brings those into the file's own
scope (`import "std/json" { parse };` then `parse(text)`). The import form is
covered in full in [modules](../language/modules.md).

A call to a name a module you have not imported declares, such as `sqrt(x)` or
`fs::read(path)`, is a compile error whose help names the import it needs.

A library import's first segment is looked up among the packages the project
depends on before the library directory, so a dependency named `std` would
shadow the standard library. Nothing else about `std` is special: it is the
directory a library path falls back to.

## The prelude

What a program reaches for on every other line needs no import:

- Every method on a string, a list, a map, an int, a float and a bool, whether
  the language builds it in (`s.uppercase()`, `xs.push(x)`) or a module
  defines it (`s.capitalize()`, `xs.map(f)`, `m.get(k)`, `"42".parse<int>()`).
- `Option` and `Result`, with `Some`, `None`, `Ok`, `Err` and their methods.
- The free built-ins: `print`, `input`, `exit`, `argv`, `type` and the
  conversions. They are listed in [built-in functions](builtins.md).

```rust
fn main() {
    let xs = [1, 2, 3, 4];
    print(xs.map(fn(x) => x * 2));
    print(xs.max().unwrap_or(0));
    print({"a": 1}.get("b"));
}
```

The modules behind it are `list`, `string`, `map`, `option` and `result`.
Importing one of them still works, reaches the same module, and binds its name
like any import; its names stay reachable with no prefix. A method nothing calls is never compiled, so the prelude adds nothing to
a program that does not use it. A program that declares its own enum with a
`Some` variant, or its own `Option`, gets its own when it names it.

Everything else is imported: `assert`, `fs`, `hash`, `json`, `math`,
`random`, `set` and `time`.

## The modules

| Module | What it gives you |
| --- | --- |
| [assert](assert.md) | Assertions that raise an error when a check fails |
| [builtins](builtins.md) | The always-available functions and methods (no import) |
| [fs](fs.md) | Read, write, append to and delete files |
| [hash](hash.md) | md5, sha1 and sha256 digests of a string, as hex |
| [json](json.md) | Parse a json string into candela values, and serialise back |
| [list](list.md) | Reductions, slicing, and higher-order methods on arrays |
| [map](map.md) | Map methods: `get`, which answers an `Option`, and `is_empty` |
| [math](math.md) | Trigonometry, logarithms, roots, rounding, and the constants |
| [option](option.md) | The `Option` enum: `Some(x)` or `None`, with methods |
| [random](random.md) | A seedable pseudo-random generator for ints and floats |
| [result](result.md) | The `Result` enum: `Ok(v)` or `Err(e)`, with methods |
| [set](set.md) | `Set<T>`, a set of unique values, with union, intersection, difference, and symmetric difference |
| [string](string.md) | Substrings, characters, padding, capitalisation, line splitting, counting |
| [time](time.md) | The current unix time, and formatting a timestamp |

## Errors

Standard library functions report failure the same way the rest of the language
does: they raise an error that stops the run and prints a message naming the
cause. Each page below states what its functions raise.

A raised error carries a short code as well as a message. The code is what a
`catch` binds and what `catch "code"` filters on; the message is what an uncaught
error prints. A `throw` is its own code, so `throw "no such user";` is caught as
`no such user`. A `try` catches what a library function raises just as it
catches a `throw` written in the block. See
[error handling](../language/error-handling.md) for the mechanism and
[the error catalogue](../reference/errors.md) for the codes.
