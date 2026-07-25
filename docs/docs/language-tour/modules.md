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

Candela libraries are ordinary Candela files.
The `libs/` folder located next to the Candela executable (currently in `/Library/Candela/` on macOS and `/usr/local/lib/candela/` on Linux) is checked by the `import` keyword if the file isn't found locally, making global Candela libraries possible. As such, by placing `.cdl` files in the `libs/` folder, you can make libraries available globally.

Set the `CANDELA_LIB_PATH` environment variable to override where this lookup goes: it names the `libs/` directory to search (the one that holds `std/` and, for the C-backed libraries, `std_src/`), which is useful when running from a source checkout rather than an install.

### Standard library

The standard library lives in `libs/std/`. The `math`, `time`, and `random` libraries are backed by a C library loaded across Candela's dynamic-library FFI. The `string`, `list`, `convert`, and `assert` libraries are written in Candela and use no dynamic library, so a program that imports them builds to a `.cdlb` artifact that runs under `candela-vm` with the module bytecode inlined.

```rs
import "std/math.cdl";
import "std/string.cdl";
import "std/list.cdl";

fn main() {
    print(math::cos(3.14159265359));
    print(string::capitalize("hello"));
    print(list::sum([1, 2, 3, 4]));
}
```