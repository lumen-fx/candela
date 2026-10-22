# browser

What a browser offers a page outside its own document: popup windows and the
messages they send back, sharing, downloads, the leave-page prompt, media
queries, a file picker and fullscreen. Navigation is not here; pages and links
own it.

```rust
import "browser";
```

Every function works in a browser, through the JavaScript module the package
ships. Anywhere else each one raises `browser_only`, so a program written for
a page compiles and runs on the desktop too; code that should do something
else there picks a side with [`@cfg(web)`](../language/conditional-compilation.md).

```rust
import "browser";

fn main() {
    if browser::match_media("(prefers-color-scheme: dark)") {
        print("dark");
    }
    browser::watch_media("(prefers-color-scheme: dark)", fn(dark) {
        print("now dark: " + string(dark));
    });
}
```

An answer the browser gives later arrives in the function passed to the call,
as a `Result`: `Ok` with the answer, or `Err` with the reason. A picker, a
share sheet and fullscreen open only right after the visitor clicked or
pressed a key; asked at another moment, they answer `Err` saying so.

## Popups and messages

| Function | Returns | Behaviour |
| --- | --- | --- |
| `open_popup(url, name, features)` | bool | Opens a popup window under `name`, with `window.open` features such as `"width=500,height=600"`; false when the browser blocked it |
| `close_popup(name)` | bool | Closes the popup opened under `name`; false when there is none |
| `post_message(target, data, origin)` | bool | Sends `data` to `opener`, `parent` or a popup by name, for a window at `origin` (`"*"` for any); false when there is no such window |
| `close_window()` | nothing | Closes this window, which the browser allows for a window a script opened |
| `on_message(handler)` | nothing | Calls `handler` with a `browser::Message` for every message another window posts to this one |

A `Message` has `origin`, the sending window's origin; `data`, what it sent,
as JSON reads it (`any`); and `source`: `opener`, `parent`, a popup's name, or
empty for a window this page did not open.

## Sharing and saving

| Function | Returns | Behaviour |
| --- | --- | --- |
| `share(data, on_done)` | nothing | Opens the system share sheet for the `title`, `text` and `url` in `data`; `on_done` receives `Ok(true)` once shared |
| `can_share(data)` | bool | Whether the browser can share this `data` |
| `download_text(filename, mime, text)` | nothing | Saves `text` as a file through the browser's download |
| `guard_unload(on)` | nothing | While `on`, leaving or reloading the page asks the visitor to confirm |

## Media queries

| Function | Returns | Behaviour |
| --- | --- | --- |
| `match_media(query)` | bool | Whether a media query matches, such as `"(prefers-color-scheme: dark)"` |
| `watch_media(query, on_change)` | nothing | Calls `on_change` with whether `query` matches each time that changes |

## Files and fullscreen

| Function | Returns | Behaviour |
| --- | --- | --- |
| `pick_file(accept, on_done)` | nothing | Asks the visitor for a text file, `accept` as in `<input accept>`; `on_done` receives a `browser::PickedFile` |
| `request_fullscreen(id, on_done)` | nothing | Shows the element with this id fullscreen, or the whole page for `""`; `on_done` receives `Ok(true)` once it is |
| `exit_fullscreen()` | nothing | Leaves fullscreen |
| `is_fullscreen()` | bool | Whether something is fullscreen |
| `on_fullscreen_change(handler)` | nothing | Calls `handler` with the id of the element shown, empty for the whole page, and whether fullscreen started or ended |

A `PickedFile` has `name`, `mime` (empty when the browser does not know it),
`size` in bytes, and `text`.
