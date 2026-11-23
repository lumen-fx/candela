# `download::to_file` is now `http::download`

HTTP has one module, `std/http`. `download::to_file(url, path, options,
on_event)` is `http::download(url, path, on_event)` with the options as named
arguments: `checksum`, `timeout_ms` and `max_bytes`. Its events are
`http::Progress`: `Progress(received, total)` is `Received(received, total)`,
with the total an `int?` that is `None` when the server declared no size, and
`Done(path)` is `Saved(path)`; `Failed(message)` is unchanged. The call answers
an `http::Transfer`, whose `cancel()` replaces `download::cancel(handle)`, and
`download::set_max_concurrent` is `http::set_max_concurrent`.

This affects you if a program downloads with the `download` package.

Before:

```rust
import "download";

fn main() {
    let url = "https://example.com/pack.zip";
    let options = download::Options { checksum: "sha256:...", .. };
    let handle = download::to_file(url, "pack.zip", options, fn(e) {
        match e {
            Progress(received, total) => print(received),
            Done(path) => print("saved " + path),
            Failed(why) => print(why),
        }
    });
}
```

After:

```rust
import "std/http";

fn main() {
    let url = "https://example.com/pack.zip";
    let transfer = http::download(url, "pack.zip", checksum: "sha256:...") fn(e) {
        match e {
            Received(received, _) => print(received),
            Saved(path) => print("saved " + path),
            Failed(why) => print(why),
        }
    };
}
```
