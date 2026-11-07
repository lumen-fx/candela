# An artifact no longer finds a dynamic library through the working directory

Loading a `.cdlb` used to hand a library's name to the operating system's
loader after the host's directories, so a relative path such as
`dylib "./native/mylib"` resolved against whatever directory the process was
started in, and `candela-vm` did not look beside the artifact at all. Now
`candela-vm` looks beside the artifact first, a relative path resolves only
against the directories searched (the artifact's own, the ones a host names
with `set_dylib_dirs`, and the toolchain's `libs` for the standard library),
and the working directory is never searched.

This affects you if you run an artifact with its library in the working
directory and not beside the artifact, or if a Rust host loads an artifact
whose `dylib` block names a relative path and relies on the working directory
to resolve it.

Before:

```sh
cd build && candela-vm ../dist/app.cdlb   # libmylib.so in build/
```

After, keep the library beside the artifact:

```sh
cp build/libmylib.so dist/ && candela-vm dist/app.cdlb
```

A Rust host names the directory that holds its libraries:

```rust
candela_vm::set_dylib_dirs(vec![app_root.join("native")]);
```
