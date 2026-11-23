# `process::start` and `archive::extract` take named arguments and answer a handle

The `Options` structs are gone. Each option is a parameter with a default,
given by name when it is not the default, and the closure that receives the
events comes last, so it can follow the call. The call answers a handle value
instead of an `int`: `process::Child`, whose `stop()` replaces
`process::stop(handle)`, and `archive::Job`. A handle's `id` is the event
source, for `event::wait(handle.id)`.

This affects you if a program starts a program or extracts an archive.

Before:

```rust
let handle = process::start("git", ["status"], process::Options { cwd: "repo", .. }, fn(e) {
    print(e);
});
process::stop(handle);
archive::extract("a.zip", "out", archive::Options { include: ["*.so"] }, fn(e) { print(e); });
```

After:

```rust
let child = process::start("git", ["status"], cwd: "repo") fn(e) {
    print(e);
};
child.stop();
archive::extract("a.zip", "out", include: ["*.so"]) fn(e) { print(e); };
```
