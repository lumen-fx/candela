---
icon: lucide/hourglass
---
# Time library

!!! warning

    This is highly subject to change.

Import this library with `import "std/time";` at the top-level.

*[at the top-level]: Outside of any function.

## Now
`now() -> int`<br/>
Returns the number of seconds since January 1, 1970, 00:00:00 UTC.

## Format
`format(date: int, format: string) -> string`<br/>
Formats `date` into a string according to the given `format` pattern.
```rust
print(format(now(), "%x - %X %p")); // prints "06/29/26 - 10:59:21 AM"
```