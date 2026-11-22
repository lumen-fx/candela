# os

The process a program runs in: its environment variables, its working
directory, and the platform under it.

```rust
import "std/os";
```

```rust
import "std/os";

fn main() {
    print(os::env("HOME") else "no HOME");
    print(os::platform() + " on " + os::arch());
}
```

The command line, `exit`, `input` and `print` are [built-ins](builtins.md),
not part of this module. The environment a started program gets is
`process::start`'s `env` argument; `set_env` changes this program's own, which
programs it starts afterwards inherit.

In a browser only `platform` and `arch` answer, as `web` and `wasm32`.

## env

```rust
os::env(name)
```

- Returns: the value of the environment variable `name` as a `string?`;
  `None` when it is not set.

## set_env

```rust
os::set_env(name, value)
```

Sets the variable `name` to `value` for this program and the programs it
starts from now on. A name that is empty or holds `=` is left alone.

## env_vars

```rust
os::env_vars()
```

- Returns: every environment variable, as a `{string: string}` by name.

## cwd

```rust
os::cwd()
```

- Returns: the working directory as an absolute path, in a `string!string`.

## set_cwd

```rust
os::set_cwd(path)
```

Makes `path` the working directory. Returns `Ok(())`, or the `fs_*` error
that says why it could not.

## platform

```rust
os::platform()
```

- Returns: `linux`, `macos`, `windows`, or `web` in a browser.

## arch

```rust
os::arch()
```

- Returns: the processor architecture: `x86_64`, `aarch64`, or `wasm32` in a
  browser.

## home_dir

```rust
os::home_dir()
```

- Returns: the user's home directory as a `string?`, from `HOME`, or
  `USERPROFILE` on Windows; `None` when the environment names none.

## temp_dir

```rust
os::temp_dir()
```

- Returns: the directory for temporary files.
