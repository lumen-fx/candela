---
icon: lucide/package
---
# Modules

## Importing other Candela files

You can import other Candela files with the `import` keyword at the top-level.

Imports can be nested, and circular imports are detected and produce an error.

*[at the top-level]: Outside of any function.

```rust
import "fibonacci_lib.cdl"; // all functions/structs are available under fibonacci_lib::
import "other_lib.cdl" as mylib; // all functions/structs are available under mylib::

fn main() {print(mylib::my_func(42));}
```

## Importing libraries

Import a library by its namespaced name. `import std::string` binds the module's
functions under `string::`, and the resolver finds the shipped module for you, so
this works from any working directory with nothing to configure.

```rust
import std::math;
import std::string;
import std::list;

fn main() {
    print(math::cos(3.14159265359));
    print(string::capitalize("hello"));
    print(list::sum([1, 2, 3, 4]));
}
```

Add `as` to choose a different namespace: `import std::string as s;` makes the
functions available under `s::`.

### How a namespaced import resolves

A namespaced import maps to a `.cdl` file in the shipped library directory:
`std::string` resolves to `std/string.cdl` under that directory. The library
directory is `libs/` next to the Candela executable, which is where the installer
places it, so a normal install needs nothing set. Set `CANDELA_LIB_PATH` to point
the lookup at a different `libs/` directory (the one holding `std/` and, for the
C-backed libraries, `std_src/`); this is an escape hatch for source checkouts and
custom builds, not part of normal use.

Resolution happens at compile time, so a program built to a `.cdlb` artifact has
the imported module bytecode inlined and runs under `candela-vm` with no library
files present.

### Standard library

The standard library lives in the shipped `std/` directory. The `math`, `time`,
and `random` libraries are backed by a C library loaded across Candela's
dynamic-library FFI. The `string`, `list`, `convert`, and `assert` libraries are
written in Candela and use no dynamic library.