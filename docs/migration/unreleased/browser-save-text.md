# `browser::download_text` is now `browser::save_text`

"Download" means fetching a URL, which `std/http` does, so the browser call
that saves text as a file is `browser::save_text(filename, mime, text)`. It
behaves as `download_text` did. The browser half answers
`candela_browser_save_text` in place of `candela_browser_download_text`, so a
host that carries its own copy of `browser.js` takes the one in
`libs/std/web/`.

This affects you if a page saves text through the browser.

Before:

```rust
browser::download_text("notes.txt", "text/plain", text);
```

After:

```rust
browser::save_text("notes.txt", "text/plain", text);
```
