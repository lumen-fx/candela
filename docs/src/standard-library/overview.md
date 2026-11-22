# Standard library overview

The candela standard library is a set of modules. The ones that give the
built-in types their methods, and the `Option` and `Result` enums, are the
prelude: every program has them with no import. The rest you import by path.

## How std ships

The library ships with the toolchain, in the same archive and installer. The
toolchain installs a `libs` directory beside the `candela` executable:

- `libs/std/` holds one `.cdl` file per module, the candela source of the
  whole library.
- `libs/std/web/` holds the browser halves of `browser`, `js` and
  `websocket`, one JavaScript module each.
- `libs/std_src/` holds the native libraries some modules call into, each in a
  directory of its own: `math`, `random` and `time`, with their C sources;
  `process` for [process](process.md); `archive` for [archive](archive.md);
  and `net` for [http](http.md) and [websocket](websocket.md), which share its
  one TLS stack.

Set `CANDELA_LIB_PATH` to point at a different `libs` directory. The variable
names the directory that contains `std/` and `std_src/`, and it overrides the
default location for library imports and for the native libraries a `.cdlb`
loads. A Rust host that embeds candela names it with `Engine::with_lib_dir`
instead; see [embedding](../integration/embedding.md). The prelude does not
depend on it: the compiler carries its own copy, so a program has the prelude
wherever it compiles.

Because the modules are ordinary source files, the compiler links the ones you
import into your program. A `.cdlb` artifact built from a program that imports
only pure-candela modules runs under `candela-vm` with no library directory
present. A module that binds a native library makes the `.cdlb` record the
binding recipe, and the artifact needs that library when it runs;
`candela-vm` finds it the way the compiler does, through `CANDELA_LIB_PATH` or
under `libs/` beside its own binary. A native library loads only when a
program imports a module that binds it, and none of them is part of
`candela-vm` itself. See [artifacts](../reference/artifacts.md).

## In a browser

The WebAssembly build has no file system and loads no dynamic library. It
carries `hash`, `math`, `random` and `time` inside itself and runs the native
half of the last three on its own functions, so `import "std/math";` works
there as it does on a desktop, with the same functions. The prelude is there
too.

A `.cdlb` built on a desktop runs in the browser with every module that needs
neither a file system nor a native library: `assert`, `csv`, `encoding`,
`event`, `hash`, `json`, `math`, `os` (its `platform` and `arch`), `path`,
`random`, `set`, `time` (all but `sleep_ms` and the timers) and the prelude.
The browser build compiles these from source too, so `import "std/csv";`
works in the candela prompt. `browser`, `js` and
`websocket` run there through their `host` blocks, which the host that runs
the page binds to the JavaScript modules in `libs/std/web/` by name. `fs`,
`process`, `http` and `archive` do not run in a browser.

## Importing a module

A library import is a quoted path with no file extension. The resolver appends
`.cdl` and looks the file up in the shipped library directory, never next to
the importing file, so it works from any working directory:

```rust
import "std/json";

fn main() {
    print(json::stringify(json::parse("[1, 2]")?));
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
- [`Range`](range.md), the type of `a..b`, with its methods.
- The free built-ins: `print`, `eprint`, `input`, `exit`, `argv`, `type` and the
  conversions. They are listed in [built-in functions](builtins.md).

```rust
fn main() {
    let xs = [1, 2, 3, 4];
    print(xs.map(fn(x) => x * 2));
    print(xs.max().unwrap_or(0));
    print({"a": 1}.get("b"));
}
```

The modules behind it are `list`, `string`, `map`, `option`, `result` and
`range`.
Importing one of them still works, reaches the same module, and binds its name
like any import; its names stay reachable with no prefix. A method nothing calls is never compiled, so the prelude adds nothing to
a program that does not use it. A program that declares its own enum with a
`Some` variant, or its own `Option`, gets its own when it names it.

Everything else is imported: `archive`, `assert`, `browser`, `csv`,
`encoding`, `event`, `fs`, `hash`, `http`, `js`, `json`, `math`, `os`, `path`,
`process`, `random`, `set`, `time` and `websocket`.

## The modules

| Module | What it gives you |
| --- | --- |
| [archive](archive.md) | Unpack zip, tar and tar.gz archives off the program's thread |
| [assert](assert.md) | Assertions that raise an error when a check fails |
| [browser](browser.md) | A page's popups, messages, sharing, saving, media queries, file picker and fullscreen |
| [builtins](builtins.md) | The always-available functions and methods (no import) |
| [csv](csv.md) | Parse and write comma-separated rows |
| [encoding](encoding.md) | UTF-8, hex, base64 and URL percent encoding |
| [event](event.md) | Results from work running elsewhere, delivered to a function, and waiting for them |
| [fs](fs.md) | Read, write, copy, move and delete files, and list and make directories |
| [hash](hash.md) | md5, sha1 and sha256 digests of a string or a file, as hex |
| [http](http.md) | Download a URL to a file, with progress and checksum verification |
| [js](js.md) | The page's own JavaScript: global functions, modules, DOM events |
| [json](json.md) | Parse a json string into candela values, and serialise back |
| [list](list.md) | Reductions, slicing, and higher-order methods on arrays |
| [map](map.md) | Map methods: `get`, which answers an `Option`, and `is_empty` |
| [math](math.md) | Trigonometry, logarithms, powers, roots, and the constants |
| [option](option.md) | The `Option` enum: `Some(x)` or `None`, with methods |
| [os](os.md) | Environment variables, the working directory, the platform |
| [path](path.md) | Join paths and take them apart, as strings |
| [process](process.md) | Run another program and receive its output and exit |
| [random](random.md) | A seedable pseudo-random generator: ints, floats, choices, shuffles, uuids |
| [result](result.md) | The `Result` enum: `Ok(v)` or `Err(e)`, with methods |
| [set](set.md) | `Set<T>`, a set of unique values, with union, intersection, difference, and symmetric difference |
| [string](string.md) | Substrings, characters, padding, capitalisation, line splitting, counting |
| [time](time.md) | Clocks, sleeping, timers, formatting and parsing timestamps |
| [websocket](websocket.md) | WebSocket connections with text frames, on the desktop and in a browser |

## Errors

A standard library function that can fail in the normal run of a program, such
as reading a file or parsing json, answers a `Result` whose error is a short
code naming the cause, `fs_not_found` or `json_parse_error`. Misuse that the
program should rule out, such as an index past the end, raises an error that
stops the run and prints a message naming the cause; only `main` can catch one.
Each page below states what its functions answer and raise. See
[error handling](../language/error-handling.md) for both and
[the error catalogue](../reference/errors.md) for the codes.

## Work that runs in the background

A call that starts work off the program's thread, such as `process::start`,
`http::download` or `archive::extract`, answers at once with a handle, and
everything after that arrives in the function you pass as the call's last
argument, which can follow the call. The program keeps running after `main`
returns until the work it started has finished and its last event has been
handled; `event::wait(handle.id)` waits for one piece of work before `main`
goes on. See [event](event.md).

```rust
import "std/http";

fn main() {
    http::download("https://example.com/a.zip", "a.zip") fn(e) {
        match e {
            Saved(path) => print("saved " + path),
            Failed(why) => eprint(why),
            _ => {}
        }
    };
}
```

A relative path such a call is given resolves the way `std/fs` resolves one:
against the working directory, or the directory the embedding host set.
