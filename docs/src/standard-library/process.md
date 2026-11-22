# process

Run another program and receive each line it writes and how it ended.

```rust
import "std/process";
```

```rust
import "std/process";

fn main() {
    process::start("git", ["status", "--short"]) fn(e) {
        match e {
            Stdout(line) => print(line),
            Stderr(line) => eprint(line),
            Exit(code) => print("exit " + string(code)),
            Failed(why) => eprint(why),
        }
    };
}
```

The program runs with its own threads reading its output, so a chatty or
long-running child never holds up yours. Output arrives a line at a time,
without the newline; bytes that are not UTF-8 are replaced, and a line longer
than 64 KiB arrives in pieces. The child reads end of file from stdin; there is
no way to write to it.

The module binds the `process` native library, which ships in the toolchain's
`libs/std_src/process/`. A browser has no programs to start, so the module
does not run there.

## start

```rust
process::start(cmd, args = [], cwd = "", env = {}, end_at_exit = false, on_event)
```

- `cmd`: the program. A name with a path separator in it is a path, relative
  to the base directory; a bare name is looked up on `PATH`.
- `args`: its arguments, a `string[]`, passed as they are with no shell in
  between.
- `cwd`: the directory it starts in; empty is the base directory the host set,
  or the working directory.
- `env`: a `{string: string}` of variables set on top of the environment it
  inherits.
- `end_at_exit`: with `true`, the program ends when yours exits instead of
  running on.
- `on_event`: a `fn(process::Event)`, called for each event. It is the last
  argument, so it can follow the call.
- Returns: a `process::Child`.

```rust
import "std/process";

fn main() {
    process::start("make", ["test"], cwd: "build", env: {"VERBOSE": "1"}) fn(e) {
        print(e);
    };
}
```

| `Event` variant | Meaning |
| --- | --- |
| `Stdout(line)` | One line the program wrote to stdout |
| `Stderr(line)` | One line it wrote to stderr |
| `Exit(code)` | It ended, with its exit code, or 128 plus the signal that ended it. Always last |
| `Failed(message)` | It could not be started; nothing else follows |

Every line arrives before the `Exit`.

## Child

The program `start` started. Its `id` is the event source its events arrive
under: `event::wait(child.id)` runs handlers until it has ended; see
[event](event.md#wait).

### stop

```rust
child.stop()
```

Ends the program, and every program it started. On Linux and macOS it is asked
to end with `SIGTERM` and killed two seconds later if it has not; on Windows it
ends at once. Its `Exit` still arrives last.

- Returns: false when it had already ended.
