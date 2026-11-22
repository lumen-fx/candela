# The process, archive, websocket, browser and js packages are standard library modules

The first-party registry packages now ship inside the toolchain, in the
release archive and the MSI, with their native libraries under
`libs/std_src/`. Import them by their standard library path, and drop them
from `candela.toml`: `candela add` no longer installs them, and the registry
publishes no new release of them. The `download` package is now
`http::download`; see its own note.

| Before | After |
| --- | --- |
| `import "process";` | `import "std/process";` |
| `import "archive";` | `import "std/archive";` |
| `import "websocket";` | `import "std/websocket";` |
| `import "browser";` | `import "std/browser";` |
| `import "js";` | `import "std/js";` |

A `.cdlb` that imports one records the native library as the standard
library's own, so `candela-vm` finds it under `libs/std_src/` beside its
binary or under `CANDELA_LIB_PATH`, the way it finds `std/math`'s. Rebuild
artifacts that imported the packages. A host that runs candela in a browser
loads `libs/std/web/browser.js`, `js.js` and `websocket.js` in place of each
package's `web/` module, and binds them by the same `host` block names.

This affects you if a project depends on one of these packages.

Before:

```toml
[dependencies]
process = "0.2"
```

```rust
import "process";
```

After, with no dependency:

```rust
import "std/process";
```

The calls changed shape too: options are named arguments, and handles are
values with methods. See the notes for each module.
