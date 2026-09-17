# Dynamic C libraries

A candela program calls into a shared library by declaring the functions it
wants. The compiler opens the library, resolves each symbol, and builds the
calling interface, so a call costs no more than the marshalling of its
arguments.

## Declaring a library

```rust
dylib "z" {
    string zlibVersion();
}

fn main() {
    print(z::zlibVersion());
}
```

A `dylib` block names the library, then lists one signature per line. Each
signature is a return type, a name, and a parenthesised list of parameter
types, ending in a semicolon. Parameter names are not part of the grammar; give
the types only.

Omit the return type for a function that returns nothing:

```rust
dylib "mylib" {
    reset(int);
    int count();
}
```

The functions live in a namespace named after the library, so you call them as
`z::zlibVersion()` or `z.zlibVersion()`; both spellings name the same function.
A variable takes the name back: where `z` holds a value, `z.zlibVersion()` is
that value's own method and the library is reachable only through `z::`.
Wrapping each function in an ordinary candela function is the usual way to give
callers a tidier surface.

## Finding the library

The name you write is turned into a filename with the platform's convention:

| Platform | `dylib "foo"` resolves to |
| --- | --- |
| Linux | `libfoo.so` |
| macOS | `libfoo.dylib` |
| Windows | `foo.dll` |

Two forms bypass part of that. A name that already has an extension is used
exactly as written, on every platform, so `"libfoo.so.6"` and an absolute path
both pass through. A name containing a path separator gets the platform
extension appended but no `lib` prefix, so `"../native/mylib"` becomes
`../native/mylib.so` on Linux.

A plain name is looked for beside the importing file first, then in the working
directory, then through the operating system's own search path. A path form goes
straight to the loader, resolved relative to the importing file. This makes a
bare name behave the same way everywhere, since the loaders differ on whether
they consider the application directory at all.

A Rust host that embeds candela can name directories to look in ahead of all
that, which is what an application whose sources and libraries sit apart (`src/`
beside `lib/`) uses. Both forms honor them: a plain name is looked for there
first, and a relative path is resolved against them first. See [where a dylib
import looks](embedding.md#where-a-dylib-import-looks).

The root of every package the project depends on is searched too, after the
directories the host named and before the importing file's own. A package that
ships a shared library at its root therefore binds with nothing to configure,
and a `dylib` import inside that package finds its own library rather than one
from the program that pulled it in. A library the package keeps under `dist/`
is named by path from `src/`: `dylib "../dist/mylib"`.

For a path form with no extension, an architecture-suffixed build is preferred
when one sits beside it, so `"../native/mylib"` picks up `mylib-x86_64.so` or
`mylib-aarch64.so` before falling back to `mylib.so`. That is how you ship one
directory holding builds for several architectures.

## Types that cross

| candela | C |
| --- | --- |
| `int` | `int64_t` |
| `float` | `double` |
| `string` | `char *`, null-terminated |
| `T[]` | pointer to the packed elements |
| struct | the C struct of the same field types, by value |
| omitted return | `void` |

Anything else has no C representation and is rejected when the signature is
compiled: `bool`, enums, maps, union types and `any`.

Strings you pass in are copied into a null-terminated buffer that lives for the
duration of the call, so the C side must not keep the pointer. A string
containing an interior null byte raises `null_byte_in_string` rather than being
silently truncated.

A returned `char *` is copied into a candela string and never freed, so the C
side keeps ownership; return a static or otherwise owned buffer. A null pointer
comes back as `null`.

Arrays are passed as a pointer to the packed elements with no length alongside
them, so pass the length as a separate `int` parameter. An array cannot be
returned, because C does not convey how long it is; that signature raises
`c_array_return_type_not_supported`.

Calls use the platform's default C calling convention. There is no way to select
another.

## When the library is needed

The library is opened and every symbol resolved while the program is compiled,
so a missing library or a missing symbol is a compile error naming what it could
not find, not a surprise at the first call.

Building a `.cdlb` artifact records the library name, the symbol and the
signature, never the library's bytes. Loading the artifact re-opens the library
and re-resolves the symbol, so the library has to be present wherever the
artifact runs. A standard-library module's own library is the exception: it is
recorded relative to the `libs` directory rather than to the module, so the
runtime finds it through the toolchain. See
[artifacts](../reference/artifacts.md).

Dynamic libraries are not available when candela is compiled to WebAssembly.

## A worked example

The standard library's `time` module is a dynamic library binding. The C side is
one file with its exports marked visible:

```c
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#include <stdint.h>
#include <time.h>

EXPORT int64_t now(void) { return (int64_t)time(NULL); }

EXPORT const char *format(int64_t timestamp, const char *fmt) {
  static char buffer[128];
  time_t t = (time_t)timestamp;
  struct tm *info = localtime(&t);
  strftime(buffer, 128, fmt, info);
  return buffer;
}
```

Note the types: `int64_t` for candela's `int`, `double` for its `float`, and a
`static` buffer for the returned string, since candela copies it out and the C
side keeps ownership.

Build it as a shared library. The shipped ones are built like this:

```sh
# Linux
clang -O2 -fvisibility=hidden -shared -fPIC -o time.so time.c
# macOS
clang -O2 -fvisibility=hidden -dynamiclib -o time.dylib time.c
# Windows
clang -O2 -fuse-ld=lld -fvisibility=hidden -shared -o time.dll time.c
```

The candela side declares the two symbols and wraps them:

```rust
dylib "../std_src/time/time" {
    int now();
    string format(int, string);
}

fn now() { return time::now(); }
fn format(date, pattern) { return time::format(date, pattern); }
```

The path form is what makes `time.so` resolve without a `lib` prefix, and the
namespace comes from the last part of the path. Anyone importing the module
calls `time::now()` and never sees the binding.

## Host functions instead

A `host` block looks the same but binds to a Rust closure in an embedding
program rather than a C symbol, and accepts types that C cannot carry. See
[embedding](embedding.md).
