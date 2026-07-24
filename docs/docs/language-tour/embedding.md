---
icon: lucide/cpu
---
# Embedding Candela
!!! warning

    This API is highly subject to change, particularly once instruction and memory limits are implemented.

Candela can be embedded in other programs through a C ABI.
You can download the `libcandela-*` artifact of your choice from [the latest release](https://github.com/lumen-fx/candela/releases/latest), or you can build it from source as a dynamic library: `#!sh cargo build --profile embed --features embed`.

Two functions are exposed:
```c
// Runs the code and returns the output
extern char* candela_run(const char* code);
// Frees the returned string
extern void candela_free_output(char* output);
```

Errors are returned in the output string and don't crash the host.