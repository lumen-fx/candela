<!--
This file is derived from keel (https://github.com/horacehoff/keel),
Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
It has been modified by the candela authors. See the NOTICE file.
-->
# time

The current unix time, and formatting a timestamp.

```rust
import "std/time" as time;
```

The module binds a small dynamic library that wraps the platform time functions,
so it is one of the three std modules that need that library present at run time.
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
import "std/time" as time;

fn main() {
    print(time::format(0, "%Y-%m-%d"));
    print(time::format(time::now(), "%Y-%m-%d %H:%M:%S"));
}
```

The value `now` returns is a signed count of seconds, held in an `int`; see
[types](../language/types.md).

## In a browser

The WebAssembly build reads the browser's clock, and `format` renders in the
browser's time zone. It knows the directives of the C `strftime` in the "C"
locale apart from the ISO week ones (`%G`, `%g`, `%V`), which it writes out as
they stand. The browser has no time zone names, so `%Z` renders the offset from
UTC the way `%z` does, as in `+0200`.
