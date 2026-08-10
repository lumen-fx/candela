# string

Substrings, padding, capitalisation, line splitting, and counting.

```rust
import "std/string" as string;
```

These helpers sit on top of the built-in string methods and the indexing and
slicing operators. The built-in methods (`len`, `split`, `trim`, `uppercase`,
`replace`, `find`, and the rest) are listed in
[built-in functions](builtins.md).

Indices and lengths count bytes, not characters, so a string of non-ASCII text
does not index by character. A four-letter word ending in an accented vowel has a
length of 5, and slicing it by character position cuts the accented letter in
half.

Slicing raises `slice_out_of_bounds` when the end runs past the string, and also
when the start index reaches the length. Every helper here that slices therefore
raises on an empty string.

The module is pure candela, so it compiles into a `.cdlb` artifact and runs under
`candela-vm` with no dynamic library.

## substring

```rust
string::substring(s, start, count)
```

- `start`: the 0-based index to start at.
- `count`: how many bytes to take.
- Returns: the substring spanning `count` bytes from `start`.
- Raises: `slice_out_of_bounds` when `start + count` runs past the end, when
  `start` reaches the length, or when `s` is empty.

```rust
import "std/string" as string;

fn main() {
    print(string::substring("hello world", 6, 5));
}
```

## char_at

```rust
string::char_at(s, i)
```

- `i`: the 0-based index.
- Returns: the character at `i`, as a one-character string.
- Raises: `slice_out_of_bounds` when `i` reaches the length, or when `s` is
  empty.

## is_empty

```rust
string::is_empty(s)
```

- Returns: a bool, true when `s` has no characters.

## capitalize

```rust
string::capitalize(s)
```

- Returns: `s` with its first character upper-cased and the rest left as it is.
  An empty string returns unchanged.

## lines

```rust
string::lines(s)
```

- Returns: a list of the lines of `s`, split on newline boundaries. A trailing
  newline produces a final empty string, and a carriage return stays on the end
  of the line it belongs to.

## pad_left

```rust
string::pad_left(s, width, fill)
```

- `width`: the minimum width, as an int.
- `fill`: a one-character string.
- Returns: `s` prefixed with `fill` until it is at least `width` wide. A string
  already that wide returns unchanged.

```rust
import "std/string" as string;

fn main() {
    print(string::pad_left("7", 3, "0"));
}
```

A `fill` longer than one character can overshoot `width`, because the loop stops
at the first length that reaches it.

## pad_right

```rust
string::pad_right(s, width, fill)
```

- `width`: the minimum width, as an int.
- `fill`: a one-character string.
- Returns: `s` followed by `fill` until it is at least `width` wide. A string
  already that wide returns unchanged.

## count

```rust
string::count(s, needle)
```

- `needle`: the string to look for.
- Returns: the number of non-overlapping occurrences of `needle` in `s`, as an
  int. An empty `needle` returns 0.

Occurrences are counted without overlap, so `string::count("aaaa", "aa")` is 2.
