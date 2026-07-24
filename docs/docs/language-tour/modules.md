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
The `libs/` folder located next to the Candela executable (currently in `/Library/Candela/` on macOS and `/usr/local/lib/candela/` on Linux) is checked by the `import` keyword if the file isn't found locally, making global Candela libraries possible. For example, the `math`, `time`, `random` libraries are located in `libs/std/`. As such, by placing `.cdl` files in the `libs/` folder, you can make libraries available globally.
```rs
import "std/math.cdl";
import "std/time.cdl";
import "std/random.cdl";

fn main() {
    print(math::cos(3.14159265359));
    print(random::random_range(10.0,20.0));
    print(time::format(time::now(), "%x - %X %p"));
}
```