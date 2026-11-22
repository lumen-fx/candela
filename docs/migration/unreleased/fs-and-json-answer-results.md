# `fs` and `json::parse` answer a `Result` instead of raising

The `fs` functions that can fail and `json::parse` now return a `Result` whose
error is the code they used to raise (`fs_not_found`, `json_parse_error`, and
the rest). `fs::read` answers a `string!string`, `fs::list` a
`string[]!string`, `fs::read_bytes` an `int[]!string`, and `json::parse` an
`any!string`. `fs::write`, `append`,
`delete`, `delete_dir`, `mkdir`, `copy` and `write_bytes`, which returned
nothing, answer `Ok(true)`, a `bool!string`.

This affects you if a program reads a value from these functions or catches
their errors.

Before:

```rust
fn main() {
    try {
        let text = fs::read("config.json");
        let config = json::parse(text) as {string: any};
        print(config["port"]);
    } catch e {
        print("failed: " + e);
    }
}
```

After:

```rust
fn load(path: string) -> {string: any}!string {
    let text = fs::read(path)?;
    return Ok(json::parse(text)? as {string: any});
}

fn main() {
    match load("config.json") {
        Ok(config) => print(config["port"]),
        Err(e) => print("failed: " + e),
    }
}
```

In `main`, `fs::read(path)?` ends the program with the error when the read
fails.
