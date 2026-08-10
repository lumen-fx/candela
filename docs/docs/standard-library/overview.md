# Standard library overview

The candela standard library is a set of modules you import by path. It sits on
top of a smaller layer of built-in functions and methods that are always in
scope without an import.

## How std ships

The library ships as candela source. The toolchain installs a `libs` directory
beside the `candela` executable:

- `libs/std/` holds one `.cdl` file per module: `assert.cdl`, `convert.cdl`,
  `json.cdl`, `list.cdl`, `map.cdl`, `math.cdl`, `option.cdl`, `random.cdl`,
  `result.cdl`, `set.cdl`, `string.cdl`, `time.cdl`.
- `libs/std_src/` holds the C sources and the compiled dynamic libraries for the
  three modules that call into native code: `math`, `random`, and `time`.

Set `CANDELA_LIB_PATH` to point at a different `libs` directory. The variable
names the directory that contains `std/` and `std_src/`, and it overrides the
default location for both library imports and the automatic list prelude.

Because the modules are ordinary source files, the compiler links the ones you
import into your program. A `.cdlb` artifact built from a program that imports
only pure-candela modules runs under `candela-vm` with no library directory
present. The `math`, `random`, and `time` modules bind a dynamic library, so a
`.cdlb` that uses them records the binding recipe and the library has to be
present when the artifact runs. See
[artifacts](../reference/artifacts.md).

## Importing a module

A library import is a quoted path with no file extension. The resolver appends
`.cdl` and looks the file up in the shipped library directory only, so it works
from any working directory:

```rust
import "std/string" as string;

fn main() {
    print(string::capitalize("hello"));
}
```

A bare import merges the module's functions into the importing file's own scope,
so you call them unqualified:

```rust
import "std/string";

fn main() {
    print(capitalize("hello"));
}
```

Two bare imports that export the same name are a compile error; several modules
export `map`, `count`, `find`, `len`, `is_empty`, `contains`, and `unwrap`. Use
`as` to keep them apart. The import form is covered in full in
[modules](../language/modules.md).

## Built-ins and modules

Built-in functions and methods are part of the language. `print`, `str`,
`json_parse`, `throw`, `arr.push(x)`, and `s.uppercase()` need no import and are
available under `candela-vm` with nothing installed. They are listed in
[built-in functions](builtins.md).

Standard library modules are candela code layered on those built-ins. They add
names that read as a library rather than as syntax: `list::sum`, `set::union`,
`option::unwrap_or`, `math::sqrt`.

One module is special. `std/list` loads automatically as a `list` namespace, so
its helpers work as array methods with no import at all:

```rust
fn main() {
    let xs = [1, 2, 3, 4];
    print(xs.map(fn(x) { return x * 2; }));
    print(list::sum(xs));
}
```

The automatic prelude is skipped when the library directory is missing, and when
your file already binds the name `list`.

## The modules

| Module | What it gives you |
| --- | --- |
| [assert](assert.md) | Assertions that raise an error when a check fails |
| [builtins](builtins.md) | The always-available functions and methods (no import) |
| [convert](convert.md) | `to_int`, `to_float`, `to_string`, `to_bool` wrappers over the built-in conversions |
| [json](json.md) | Parse a json string into candela values, and serialise back |
| [list](list.md) | Reductions, slicing, and higher-order helpers over arrays |
| [map](map.md) | Free-function spellings of the map methods, plus `get_or` |
| [math](math.md) | Trigonometry, logarithms, roots, rounding, and the constants |
| [option](option.md) | The `Option` enum: `Some(x)` or `None`, with helpers |
| [random](random.md) | A seedable pseudo-random generator for ints and floats |
| [result](result.md) | The `Result` enum: `Ok(v)` or `Err(e)`, with helpers |
| [set](set.md) | A set of unique values, with union, intersection, and difference |
| [string](string.md) | Substrings, padding, capitalisation, line splitting, counting |
| [time](time.md) | The current unix time, and formatting a timestamp |

## Errors

Standard library functions report failure the same way the rest of the language
does: they raise an error that stops the run and prints a message naming the
cause. Each page below states what its functions raise.

A raised error carries a short code as well as a message. The code is what a
`catch` binds and what `catch "code"` filters on; the message is what an uncaught
error prints. A `throw` is its own code, so `throw("no such user")` is caught as
`no such user`. The imported modules are written in candela, and a call to one
does not return when it sits inside a `try` block. See
[error handling](../language/error-handling.md) for the mechanism and
[the error catalogue](../reference/errors.md) for the codes.
