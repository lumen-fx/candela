# process

Run another program and receive each line it writes and how it ended.

```rust
import "process";
```

```rust
import "process";

fn main() {
    process::start("git", ["status", "--short"], process::Options { .. }, fn(e) {
        match e {
            Stdout(line) => print(line),
            Stderr(line) => print("! " + line),
            Exit(code) => print("exit " + string(code)),
            Failed(why) => print(why),
        }
    });
}
```

The program runs with its own threads reading its output, so a chatty or
long-running child never holds up yours. Output arrives a line at a time,
without the newline; bytes that are not UTF-8 are replaced, and a line longer
than 64 KiB arrives in pieces. The child reads end of file from stdin; there is
no way to write to it.

## start

```rust
process::start(cmd, args, options, on_event)
```

- `cmd`: the program. A name with a path separator in it is a path, relative
  to the base directory; a bare name is looked up on `PATH`.
- `args`: its arguments, a `string[]`, passed as they are with no shell in
  between.
- `options`: a `process::Options`; `process::Options { .. }` takes every
  default.
- `on_event`: a `fn(process::Event)`, called for each event.
- Returns: a handle for `stop`.

| `Options` field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `cwd` | string | `""` | The directory the program starts in; empty is the base directory or the working directory |
| `env` | `{string: string}` | `{}` | Variables set on top of the environment it inherits |
| `end_at_exit` | bool | `false` | End the program when yours exits, instead of leaving it running |

| `Event` variant | Meaning |
| --- | --- |
| `Stdout(line)` | One line the program wrote to stdout |
| `Stderr(line)` | One line it wrote to stderr |
| `Exit(code)` | It ended, with its exit code, or 128 plus the signal that ended it. Always last |
| `Failed(message)` | It could not be started; nothing else follows |

Every line arrives before the `Exit`.

## stop

```rust
process::stop(handle)
```

Ends the program started under `handle`, and every program it started. On
Linux and macOS it is asked to end with `SIGTERM` and killed two seconds later
if it has not; on Windows it ends at once. Its `Exit` still arrives last.

- Returns: false when it had already ended.
