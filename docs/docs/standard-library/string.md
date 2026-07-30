---
icon: lucide/type
---
# String library

Helpers that build on Candela's string operators and built-in methods. Import
the library with `import "std/string";` at the top-level.

*[at the top-level]: Outside of any function.

This module is written in Candela and uses no dynamic library, so a program that
imports it builds to a `.cdlb` artifact that runs under `candela-vm` with the
module bytecode inlined.

The built-in string methods (`split`, `uppercase`, `lowercase`, `trim`,
`replace`, `starts_with`, `ends_with`, `repeat`, `find`, `contains`, `len`, and
the `trim_*` family) remain available on any string; see
[Built-in functions](built-in-functions.md). This module adds helpers that those
methods do not cover.

### Substring
`substring(s: string, start: int, count: int) -> string` -- the substring of `s`
starting at `start` (0-based) and spanning `count` characters.
### Char at
`char_at(s: string, i: int) -> string` -- the character at index `i`, as a
one-character string.
### Is empty
`is_empty(s: string) -> bool` -- true when `s` has no characters.
### Capitalize
`capitalize(s: string) -> string` -- `s` with its first character upper-cased and
the rest unchanged.
### Lines
`lines(s: string) -> string[]` -- the lines of `s`, split on newline boundaries.
### Pad left
`pad_left(s: string, width: int, fill: string) -> string` -- `s` padded on the
left with the one-character string `fill` until it is at least `width` wide.
### Pad right
`pad_right(s: string, width: int, fill: string) -> string` -- `s` padded on the
right with `fill` until it is at least `width` wide.
### Count
`count(s: string, needle: string) -> int` -- the number of non-overlapping
occurrences of `needle` in `s`.
