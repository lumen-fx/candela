# cli

Reading a command line: flags, options with values, and the arguments left
over.

```rust
import "std/cli";
```

```rust
import "std/cli";

fn main() {
    let args = cli::parse(argv(), takes_value: ["out", "o"])?;
    if args.flag("verbose") {
        print("verbose");
    }
    print(args.option("out") else "a.out");
    print(args.positional);
}
```

`--name` and `-n` are flags, and `-abc` is the three flags `a`, `b` and `c`. A
name listed in `takes_value` is an option instead: its value is the next
argument, or follows `=` as in `--out=file`. `--` ends the options, and
everything after it is positional, as is `-` alone.

## parse

```rust
cli::parse(args, takes_value = [])
```

- `args`: the arguments, usually [`argv()`](builtins.md).
- `takes_value`: the names, without dashes, that take a value.
- Returns: a `cli::Args`, in an `Args!string`; `Err("cli_missing_value")` for
  an option at the end with no value after it.

## Args

| Member | Is |
| --- | --- |
| `positional` | The arguments that are neither flags nor options, in order |
| `flags` | The flags given, by name, without their dashes |
| `options` | The options given, by name, each with the last value it was given |
| `args.flag(name)` | True when the flag `name` was given |
| `args.option(name)` | The value of the option `name`, as a `string?` |
