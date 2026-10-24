# js

The page's own JavaScript, reached from candela: global functions and values
by dotted path, modules loaded at run time and held by handle, and DOM events.

```rust
import "js";
```

Every function works in a browser, through the JavaScript module the package
ships. Anywhere else each one raises `browser_only`, so a program written for
a page compiles and runs on the desktop too; code that should do something
else there picks a side with [`@cfg(web)`](../language/conditional-compilation.md).

```rust
import "js";

fn main() {
    let numbers: any[] = [3, 9, 4];
    print(js::call("Math.max", numbers));
    print(js::get("navigator.language"));
    let url: any[] = ["/hello.txt"];
    js::call_async("fetch", url, fn(answer) {
        match answer {
            Ok(_) => print("fetched"),
            Err(why) => print(why),
        }
    });
}
```

A path is a dotted walk from the global object, so `Math.max` is
`globalThis.Math.max`, and a function found there is called with the object it
was found on as `this`. Values cross the way JSON carries them; anything that
cannot cross, such as a module, stays in JavaScript and the program holds a
number, a handle, that names it.

A function that returns a promise is called through the `_async` form, and
what the promise settles to arrives in the function passed to the call, as a
`Result`: `Ok` with the value, or `Err` with the reason. Calling one through
the plain form raises. The arguments go in an `any[]`, so a list written for
them is declared as one, as `numbers` is above; a list literal passed straight
in is typed by its elements and does not fit.

## Globals

| Function | Returns | Behaviour |
| --- | --- | --- |
| `call(path, args)` | any | Calls the global function at `path` with the `any[]` `args` |
| `call_async(path, args, on_done)` | nothing | The same for a function that returns a promise |
| `get(path)` | any | The global value at `path`, such as `navigator.language` |
| `set(path, value)` | nothing | Sets the global value at `path` |

## Modules

| Function | Returns | Behaviour |
| --- | --- | --- |
| `load(url, on_done)` | nothing | Imports the JavaScript module at `url`; `on_done` receives a handle to it |
| `invoke(handle, method, args)` | any | Calls `method` on what `handle` holds |
| `invoke_async(handle, method, args, on_done)` | nothing | The same for a method that returns a promise |
| `release(handle)` | bool | Lets go of a handle; false when it held nothing |

`load` imports modules from the page's own origin. Another URL loads only when
it starts with one of the prefixes in the package's `allow` list, which the
host that serves the page reads from its own configuration; see that host's
documentation.

## DOM events

| Function | Returns | Behaviour |
| --- | --- | --- |
| `listen(target, event, on_event)` | `Option<int>` | Calls `on_event` with each DOM `event` on `window`, `document`, or the element with the id `target`; `None` when there is no such element |
| `unlisten(handle)` | bool | Stops the listener `listen` answered `handle` for; false when there was none |

`on_event` receives the event described as a map: its `type`, and whichever of
`key`, `code`, `button`, `clientX`, `clientY`, `deltaX`, `deltaY`, `detail`,
`value` and `target` it has.
