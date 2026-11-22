# event

Results that arrive from work running somewhere else, such as a native
library's thread or a browser promise, delivered to a function you choose.

```rust
import "std/event";
```

Work runs under a source: an `int` you get from `event::source()`. Whatever
runs the work posts events to the source, each carrying one string, JSON by
convention. `event::on` names the function that receives a source's events.
The program runs it on its own thread, one event at a time, in the order the
events were posted, so a handler reads and writes your variables like any
other code.

Most programs never call this module themselves. A package that starts work
in the background, such as [process](../packages/process.md) or
[download](../packages/download.md), uses it to deliver its results to the
function you pass to its calls. Write one when you are writing such a package:
the source goes to the native library, which posts to it from its own threads;
see [posting events from a library](../integration/c-libraries.md#posting-events-from-a-library).

```rust
import "std/event";

dylib "../dist/ticker" {
    fn start(source: int, ms: int);
}

fn every(ms: int, on_tick: fn(string)) -> int {
    let id = event::source();
    event::on(id, fn(payload) { on_tick(payload); });
    ticker::start(id, ms);
    return id;
}
```

## The program stays running

A source is held while the work under it is still running. After `main`
returns, `candela` and `candela-vm` keep running handlers for as long as a
source this program handles is held, and exit once none is and every event has
been handled. A program that starts a download in `main` therefore sees it
finish without waiting for it. To have the result before `main` goes on, call
`event::wait` with the source. `exit(code)` ends the program at once, held
sources or not.

A host that embeds candela runs its own loop instead: it learns that an event
arrived through the waker it sets, and runs the handlers with
`dispatch_events`. See [embedding](../integration/embedding.md#events).

An event for a source no handler was registered for is dropped. When the work
under a source releases its last hold, the handler receives every event posted
before that and is then let go.

## source

```rust
event::source()
```

- Returns: an `int` no other source in the process shares.

## on

```rust
event::on(id, handler)
```

- `id`: the source.
- `handler`: a `fn(string)`, called with the payload of each event posted
  under `id` from now on. A later `on` for the same source replaces it.

## dispatch

```rust
event::dispatch(budget, wait)
```

Runs the handlers of events that have arrived, at most `budget` of them, or
every one when `budget` is negative.

- `wait`: with `true`, waits for more while work under a source this program
  handles is still running; with `false`, returns as soon as nothing is
  queued.
- Returns: how many handlers ran.

The runtime calls this for you after `main`. Call it yourself from a `main`
that loops on its own and wants to handle what arrived in between.

## wait

```rust
event::wait(id)
```

Runs the handlers of events as they arrive, from every source, until the work
under `id` has finished and its last event has been handled. Answers at once
when nothing runs under `id`. The handles the standard library's background
calls answer carry their source as `id`:

```rust
import "std/event";
import "std/process";

fn main() {
    let child = process::start("git", ["status"]) fn(e) { print(e); };
    event::wait(child.id);
    print("git is done");
}
```

In a browser nothing arrives while a program waits, so there `wait` runs only
what has already arrived.

## answer

```rust
event::answer(read, on_done)
```

- `read`: a `fn(any) -> T` that turns the answer's value into the type the
  caller wants.
- `on_done`: a `fn(T!string)`, called once with the answer.
- Returns: a new source, for the work to post its answer to.

For work that answers once: it posts one JSON object, `{"value": v}`, which
`on_done` receives as `Ok(read(v))`, or `{"error": "why"}`, which it receives
as `Err("why")`. The standard library's own one-answer calls are built on
it.

```rust
import "std/event";

fn main() {
    let id = event::answer(fn(v) => v as int) fn(r) { print(r); };
    // hand `id` to the work that posts the answer
}
```
