# First-party packages

Capabilities a program opts into rather than carries: running other programs,
downloading, unpacking archives, WebSocket connections, and a browser page's
own APIs. They ship from the candela repository as registry packages, released
with candela itself at the same version, so the runtime stays small and a
program that needs none of them pays for none of them.

| Package | What it gives you |
| --- | --- |
| [process](process.md) | Run another program and receive its output and exit |
| [download](download.md) | Download a URL to a file, with progress and checksum verification |
| [archive](archive.md) | Unpack zip, tar and tar.gz archives |
| [websocket](websocket.md) | WebSocket connections with text frames |
| [browser](browser.md) | Popups, messages, sharing, downloads, media queries, a file picker, fullscreen |
| [js](js.md) | The page's own JavaScript: global functions, modules, DOM events |

Add one to a project and import it by name:

```sh
candela add download
```

```rust
import "download";
```

## Results arrive as events

A call that starts work in the background answers at once with a handle, and
everything after that arrives in the function you passed to the call, as a
value of the package's `Event` enum. The program keeps running after `main`
returns until the work it started has finished and its last event has been
handled; see [event](../standard-library/event.md).

```rust
import "download";

fn main() {
    download::to_file("https://example.com/a.zip", "a.zip", download::Options { .. }, fn(e) {
        match e {
            Done(path) => print("saved " + path),
            Failed(why) => print(why),
            _ => {}
        }
    });
}
```

## Where they run

process, download, archive and websocket carry a native library for each
desktop target, built for Linux and macOS on x86_64 and aarch64 and for
Windows on x86_64. A program that imports one needs that library beside the
package, which the registry client fetches with it.

websocket, browser and js also run in a browser, through a JavaScript module
each ships under `web/`, named by its manifest's [`[web]`](../reference/candela-toml.md#web)
table. browser and js only run there: anywhere else each of their functions
raises `browser_only`, so a program written for a page compiles on the
desktop too.

A relative path a package is given resolves the way `std/fs` resolves one:
against the working directory, or the directory the embedding host set.
