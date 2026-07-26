---
icon: lucide/library-big
---
# Dynamic libraries
You can load functions from dynamic libraries by specifying each function's signature, with the following syntax at the top-level:

*[at the top-level]: Outside of any function.

```rust
dylib "dynamic_library_path" {
  function_return_type function_name(function_arg_type_1, function_arg_type_2, ..., function_arg_type_n);
}
```

For example:
```rust
dylib "my_test.dylib" {
    int add(int, int);
    float add(float, float);
    string add(string, string);
    int sum(int[], int);
}

fn fib(n) {
    if n <= 1 {
        return n;
    }
    return fib(my_test::add(n, -1)) + fib(my_test::add(n,-2));
}

fn main() {
    print(my_test::add(6, 1));
    print(fib(25));
}
```

## Logical names vs. paths

The string after `dylib` can be either a **logical library name** or a **path**,
and Candela resolves them differently so a single Candela file works across
platforms:

- A **bare logical name** (no path separator and no extension, e.g. `m`,
  `sqlite3`, `z`) is mapped to your OS's library filename convention: `libz.so`
  on Linux, `libz.dylib` on macOS, `z.dll` on Windows. Candela then looks for
  that file next to the importing Candela file, then in the current
  directory, and only then falls back to the system loader's normal search
  paths (`LD_LIBRARY_PATH`, `ld.so.cache`, and similar). This mirrors Windows,
  where `LoadLibrary` searches the application directory and the current
  directory before the system paths. Use a bare logical name both for system
  libraries you expect to be installed and for a library you build and ship
  next to your `.cdl` file.

  ```rust
  // Resolves to libz.so / libz.dylib / z.dll depending on the OS.
  dylib "z" {
      string zlibVersion();
  }
  fn main() { print(z::zlibVersion()); }
  ```

- A **path** (anything with a path separator, or an explicit extension) is
  resolved relative to the importing Candela file. If the extension is omitted,
  Candela chooses the correct one for your OS and also tries an
  architecture-specific build if one exists next to it, then falls back to the
  plain per-OS filename. This is the form for libraries you ship alongside your
  program.

  ```rust
  // On macOS, tries "./my_test-aarch64.dylib" (or "-x86_64", per your CPU),
  // then falls back to "./my_test.dylib". Same idea with ".so" on Linux and
  // ".dll" on Windows.
  dylib "./my_test" {
      int add(int, int);
      int sum(int[], int);
  }
  fn main() { print(my_test::add(6, 1)); }
  ```

An absolute path or an explicit filename (with extension) is honored as written.

## In `.cdlb` bytecode artifacts

When you `candela build` a program into a `.cdlb` artifact, dynamic-library
imports are captured **by reference, not by value**: the artifact records the
logical library name, each imported symbol, and its signature, never the shared
object's bytes. When `candela-vm` loads the artifact it re-opens the library
through the OS loader and re-binds the symbols by name, applying the same per-OS
name mapping described above. A `.cdlb` built on one platform therefore resolves
the right library file on another, as long as that library is installed on the
machine that runs it.