---
icon: lucide/repeat
---
# Convert library

Type-conversion helpers with names that read as verbs, wrapping Candela's
built-in `int`, `float`, `str`, and `bool` conversions. Import the library with
`import std::convert;` at the top-level.

*[at the top-level]: Outside of any function.

This module is written in Candela and uses no dynamic library, so a program that
imports it builds to a `.cdlb` artifact that runs under `candela-vm` with the
module bytecode inlined.

### To int
`to_int(x) -> int` -- parses a string or truncates a float to an `int`.
### To float
`to_float(x) -> float` -- parses a string or widens an `int` to a `float`.
### To string
`to_string(x) -> string` -- renders any value as its string form.
### To bool
`to_bool(x: string) -> bool` -- parses the strings "true" and "false" into a
`bool`.
