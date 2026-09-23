# Conditional compilation

`@cfg(...)` in front of a declaration or a statement compiles it only when its
condition holds. Use it when one program runs in more than one place and some
of its code belongs to only one of them: a browser build that calls into
JavaScript, a desktop build that reads files.

```rust
@cfg(web)
import "js";

@cfg(web)
fn track(event: string) { js::call("gtag", ["event", event]); }
@cfg(not(web))
fn track(event: string) {}

fn main() {
    track("opened");
}
```

Code whose condition is false is dropped before any name in it is looked up.
The `js` module above does not have to exist on a desktop, and the two `track`
functions do not collide, because only one of them is ever compiled.

## Where the flags come from

candela names no flags of its own. The program that compiles your code decides
which are on: a host building for the browser turns on `web`, for example. Read
that host's documentation to learn which flags it sets.

On the command line, `--cfg` turns one on for `candela run`, `check` and
`build`:

```sh
candela run --cfg web
candela build --cfg web --cfg mode=release app.cdl
```

A flag nobody turned on is false. A misspelt flag is therefore not an error; it
is a condition that never holds.

## Conditions

| Condition | Holds when |
| --- | --- |
| `name` | the flag `name` is on |
| `key = "value"` | the pair `key = "value"` is on; a key can carry several values at once |
| `not(c)` | `c` does not hold |
| `any(a, b, ...)` | at least one of them holds; `any()` never holds |
| `all(a, b, ...)` | every one of them holds; `all()` always holds |

The combinators nest: `@cfg(all(web, not(mode = "release")))`. Several
attributes in a row must all hold, so `@cfg(web) @cfg(debug)` is the same as
`@cfg(all(web, debug))`.

## What takes it

- Top-level declarations: `fn`, `import`, `struct`, `enum`, `impl`, `dylib`
  and `host` blocks.
- A method inside an `impl` block.
- A statement inside a block, including a `{ ... }` block standing on its own,
  which is how a group of statements shares one condition.

```rust
impl Store {
    @cfg(web)
    fn save(self) { js::call("localStorage.setItem", ["store", self.encode()]); }
    @cfg(not(web))
    fn save(self) { print(self.encode()); }
}

fn main() {
    @cfg(debug)
    {
        print("starting");
        print(argv());
    }
}
```

Dropped code is still parsed, so a syntax error in it is an error under every
configuration. Nothing past the syntax is checked, and a
[macro](macros.md) inside it is not expanded, since its expander may exist only
where the code is compiled.

When two declarations of one name both hold, the compile fails with the error
two declarations of that name always give.
