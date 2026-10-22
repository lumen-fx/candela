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
    start(int, int);
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
finish without waiting for it. `exit(code)` ends the program at once, held
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
