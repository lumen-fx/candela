---
icon: lucide/package
---
# Modules

## Importing

Import other Candela code with the `import` keyword at the top-level. An
import names a quoted path:

*[at the top-level]: Outside of any function.

- A path ending in `.cdl` imports that file, resolved relative to the
  importing file.
- A path without an extension imports a library from the shipped library
  directory: `import "std/string";` resolves to `std/string.cdl` there.

Imports can be nested, and circular imports are detected and produce an error.

A bare import merges the module's functions, structs, enums, and methods into
the importing file's own scope, so you call them directly:

```rust
import "std/math";
import "std/string";
import "std/list";

fn main() {
    print(cos(3.14159265359));
    print(capitalize("hello"));
    print(sum([1, 2, 3, 4]));
}
```

## Namespaced imports

Add `as` to keep a module behind a namespace instead. Its symbols are then
only reachable through the alias:

```rust
import "fibonacci_lib.cdl" as fib;
import "std/string" as string;

fn main() {
    print(fib::fibonacci(30));
    print(string::capitalize("hello"));
}
```

## Name collisions

A bare import that would redefine a name (against a local definition or
another bare import) is a compile-time error naming the symbol and both
sources. Nothing is shadowed silently. Resolve a collision by renaming the
local symbol or by giving one of the imports a namespace with `as`.

Two modules that both import the same third module bring in the same
underlying symbols; that is not a collision.

## How a library import resolves

An extensionless import maps to a `.cdl` file in the shipped library
directory: `"std/string"` resolves to `std/string.cdl` under that directory.
The library directory is `libs/` next to the Candela executable, which is
where the installer places it, so a normal install needs nothing set. Set
`CANDELA_LIB_PATH` to point the lookup at a different `libs/` directory (the
one holding `std/` and, for the C-backed libraries, `std_src/`); this is an
escape hatch for source checkouts and custom builds, not part of normal use.

A `.cdl` file import resolves next to the importing file first and falls back
to the shipped library directory.

Resolution happens at compile time, so a program built to a `.cdlb` artifact
has the imported module bytecode inlined and runs under `candela-vm` with no
library files present.

## Standard library

The standard library lives in the shipped `std/` directory. The `math`,
`time`, and `random` libraries are backed by a C library loaded across
Candela's dynamic-library FFI. The `string`, `list`, `convert`, and `assert`
libraries are written in Candela and use no dynamic library.
