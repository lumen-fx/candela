<!--
This file is derived from keel (https://github.com/horacehoff/keel),
Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
It has been modified by the candela authors. See the NOTICE file.
-->
# time

The current unix time, and formatting a timestamp.

```rust
import "std/time";
```

The module binds a small dynamic library that wraps the platform time functions,
so it is one of the four std modules that need that library present at run time.
A `.cdlb` built from a program that imports `std/time` records the binding by
name and re-opens it when the artifact runs; see
[artifacts](../reference/artifacts.md).

## now

```rust
time::now()
```

- Returns: the number of seconds since 1 January 1970, 00:00:00 UTC, as an int.

## format

```rust
time::format(timestamp, pattern)
```

- `timestamp`: seconds since the epoch, as an int.
- `pattern`: a `strftime` pattern.
- Returns: the timestamp rendered in the machine's local time zone, as a string.

The pattern is passed to the platform's `strftime`, so the directives are the C
ones: `%Y` for the four-digit year, `%m` for the month, `%d` for the day of the
month, `%H`, `%M`, `%S` for hours, minutes, and seconds, `%A` and `%B` for the
day and month names, `%%` for a literal percent sign. The formatted result has to
fit in 127 bytes; a pattern that produces more than that does not render.

```rust
import "std/time";

fn main() {
    print(time::format(0, "%Y-%m-%d"));
    print(time::format(time::now(), "%Y-%m-%d %H:%M:%S"));
}
```

The value `now` returns is a signed count of seconds, held in an `int`; see
[types](../language/types.md).

## now_ms

```rust
time::now_ms()
```

- Returns: the milliseconds since the unix epoch, an `int`.

## clock_ns

```rust
time::clock_ns()
```

- Returns: nanoseconds on a clock that only moves forward, whatever the wall
  clock does. Subtract two readings to time something; the count itself starts
  at the first reading and means nothing alone.

```rust
import "std/time";

fn main() {
    let start = time::clock_ns();
    time::sleep_ms(10);
    print(string((time::clock_ns() - start) / 1000000) + " ms");
}
```

## sleep_ms

```rust
time::sleep_ms(ms)
```

Pauses the program for `ms` milliseconds. Event handlers do not run
meanwhile; to do something later while handlers keep running, use `after`.

## format_utc

```rust
time::format_utc(t, pattern)
```

- Returns: `t`, seconds since the epoch, formatted in UTC with the same
  `strftime` directives as `format`, which uses the local time zone.

## parse_iso

```rust
time::parse_iso(text)
```

- Returns: the moment an ISO 8601 date or date-time names, in seconds since the
  epoch, as an `int?`: `2026-11-22`, `2026-11-22T10:30:00Z`, or
  `2026-11-22 10:30:00+01:00`. Fractional seconds are allowed and dropped, and
  a time with no offset is taken as UTC. `None` for any other text.

There is no calendar arithmetic or time zone database here: add seconds to a
timestamp, and format it to read it.

## after

```rust
time::after(ms, f)
```

Calls `f`, a `fn()`, once, `ms` milliseconds from now, on the program's thread
like any [event](event.md) handler. Returns a `time::Timer`. The program keeps
running until `f` has run, unless the timer is cancelled.

```rust
import "std/time";

fn main() {
    time::after(100) fn() { print("later"); };
    print("now");
}
```

## every

```rust
time::every(ms, f)
```

Calls `f` every `ms` milliseconds until the timer is cancelled. Returns a
`time::Timer`; the program keeps running while it runs.

## Timer

A timer `after` or `every` started. Its `id` is the event source its ticks
arrive under, for [`event::wait`](event.md#wait).

### cancel

```rust
timer.cancel()
```

Stops the timer before its next tick. Returns false when it had already
stopped.

## In a browser

The WebAssembly build reads the browser's clock, and `format` renders in the
browser's time zone. It knows the directives of the C `strftime` in the "C"
locale apart from the ISO week ones (`%G`, `%g`, `%V`), which it writes out as
they stand. The browser has no time zone names, so `%Z` renders the offset from
UTC the way `%z` does, as in `+0200`.

`now_ms`, `clock_ns`, `format_utc` and `parse_iso` work there too. A page's
program cannot pause or wait for its own timer, so `sleep_ms`, `after` and
`every` raise in a browser.
