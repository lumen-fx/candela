# Built-in functions

These functions and methods are part of the language. They need no import, and
they are available in a `.cdlb` artifact running under `candela-vm` with no
library directory installed.

Everything here raises errors the same way the language does. A raised error
carries a short code, and that code is what a `catch` binds and what
`catch "code"` filters on; the longer message is what an uncaught error prints.
The codes appear against each function below. See
[error handling](../language/error-handling.md) and
[the error catalogue](../reference/errors.md).

## Output and input

### print

```rust
print(value, ...)
```

Writes each argument to standard output, one per line. Takes any number of
arguments of any type and returns nothing. A list prints as `[1,2,3]`, a map as
`{a:1,b:2}`, a struct as `Name {field:value}`, and an enum value as its variant
name with any payload in brackets. A float with no fractional part prints
without one, so `print(3.0)` writes `3` where `str(3.0)` gives `"3.0"`.

A stream that refuses the write costs the line, not the run. `program | head -1`
leaves standard output with no reader, and the prints that follow go nowhere
while the program runs to the end as it otherwise would.

### input

```rust
input()
input(prompt)
```

Writes `prompt` to standard output without a trailing newline, reads one line
from standard input, and returns it as a string with the trailing newline (and a
carriage return, if present) removed. `prompt` is a string; with no argument the
prompt is empty.

## Conversion

### int

```rust
int(value)
```

Converts a string or a float to an int. A float truncates towards zero, so
`int(3.9)` is 3 and `int(-3.9)` is -3. A string that does not parse as an integer
raises `invalid_int`.

### float

```rust
float(value)
```

Converts a string or an int to a float. A string that does not parse as a
floating-point number raises `invalid_float`.

### str

```rust
str(value)
```

Renders any value as a string. Ints and floats use their shortest exact decimal
form, bools become `true` or `false`, a string is returned unchanged, and
collections, structs, and enum values use their literal form.

### bool

```rust
bool(value)
```

Converts the string `"true"` or `"false"` to a bool. Any other string raises
`invalid_bool`. The argument has to be a string; there is no truthiness
conversion from other types.

### type

```rust
type(value)
```

Returns the static type of the expression as a string, resolved at compile time:
`int`, `float`, `bool`, `string`, `null`, `int[]` for a list of ints, `a|b` for a
union, and the field list for a struct.

## Sequences

### range

```rust
range(end)
range(start, end)
```

Returns a list of ints from `start` (0 when omitted) up to but not including
`end`. Both arguments are ints. The result is empty when `start` is not less
than `end`.

## Dynamic values

A value typed `any` (a json parse result, an `Option` payload, a host function
return) carries its type at run time. These functions test and unwrap it.

### is_int, is_float, is_str, is_bool, is_list, is_map, is_null

```rust
is_int(value)
is_float(value)
is_str(value)
is_bool(value)
is_list(value)
is_map(value)
is_null(value)
```

Each returns a bool: true when the value's run-time type is the one named. They
never raise.

### as_int, as_float, as_str, as_bool, as_list, as_map

```rust
as_int(value)
as_float(value)
as_str(value)
as_bool(value)
as_list(value)
as_map(value)
```

Checked downcasts. Each returns the value typed concretely, so the result takes
part in ordinary typed expressions. A value whose run-time type differs raises
`bad_downcast`, with a message naming both the requested and the found type.
There is no `as_null`; use `is_null`.

`as_list` gives a list of `any`, and `as_map` a map with `any` keys and values,
because the entries of a dynamic collection are dynamic too. Such a collection
takes a `push` or an `insert` of any type, in any order, and an entry read back
out is an `any` that needs its own downcast.

```rust
fn main() {
    let v = json_parse("{\"n\": 7}");
    let m = as_map(v);
    print(as_int(m.get("n")) + 1);
    m.insert("name", "ada");
    print(as_str(m.get("name")));
}
```

## json

### json_parse

```rust
json_parse(text)
```

Parses a json string into candela values and returns the result typed `any`. An
object becomes a map keyed by strings, an array becomes a list, and a scalar
becomes an int, float, string, bool, or null. `text` has to be a string.
Malformed input raises `json_parse_error`, with a message carrying the reason.

### json_stringify

```rust
json_stringify(value)
```

Serialises any value to a json string.

Both are wrapped by the [json module](json.md), which gives them the shorter
names `json::parse` and `json::stringify`.

## Errors and process control

### throw

```rust
throw(message)
```

Raises a catchable error carrying `message`, which has to be a string. Nothing
after the `throw` in the enclosing block runs.

### exit

```rust
exit()
exit(code)
```

Stops the program. With no argument it ends the run normally; with an int `code`
the process exits with that status.

### argv

```rust
argv()
```

Returns the command-line arguments that follow the script path, as a list of
strings. Takes no arguments.

### the_answer

```rust
the_answer()
```

Prints the answer to the Ultimate Question of Life, the Universe, and Everything,
and returns 42.

## File system

The `fs` namespace is built in; it needs no import. Every function takes the path
as a string and raises a catchable error on failure. The code names the cause:
`fs_not_found`, `fs_permission_denied`, `fs_is_a_directory`, `fs_storage_full`,
and the rest.

### fs::read

```rust
fs::read(path)
```

Returns the whole file at `path` as a string.

### fs::exists

```rust
fs::exists(path)
```

Returns true when something exists at `path`.

### fs::write

```rust
fs::write(path, contents)
```

Writes `contents` to `path`, replacing what was there. Creates the file when it
does not exist. Returns nothing.

### fs::append

```rust
fs::append(path, contents)
```

Appends `contents` to the end of the file at `path`. Creates the file when it
does not exist. Returns nothing.

### fs::delete

```rust
fs::delete(path)
```

Deletes the file at `path`. Raises when the path does not exist or names a
directory. Returns nothing.

### fs::delete_dir

```rust
fs::delete_dir(path)
```

Deletes the empty directory at `path`. Raises when the directory is missing or
still has entries. Returns nothing.

## String methods

Called on a string receiver.

| Method | Returns | Behaviour |
| --- | --- | --- |
| `s.len()` | int | The length in bytes |
| `s.uppercase()` | string | `s` with every character upper-cased |
| `s.lowercase()` | string | `s` with every character lower-cased |
| `s.starts_with(prefix)` | bool | True when `s` begins with the string `prefix` |
| `s.ends_with(suffix)` | bool | True when `s` ends with the string `suffix` |
| `s.contains(needle)` | bool | True when the string `needle` occurs in `s` |
| `s.find(needle)` | int | The byte offset of the first occurrence of `needle`, or -1 |
| `s.replace(from, to)` | string | `s` with every occurrence of `from` replaced by `to` |
| `s.split(separator)` | string[] | `s` cut at each occurrence of the string `separator` |
| `s.trim()` | string | `s` without leading or trailing whitespace |
| `s.trim_left()` | string | `s` without leading whitespace |
| `s.trim_right()` | string | `s` without trailing whitespace |
| `s.trim_sequence(chars)` | string | `s` with any of the characters in `chars` stripped from both ends |
| `s.trim_sequence_left(chars)` | string | The same, from the start only |
| `s.trim_sequence_right(chars)` | string | The same, from the end only |
| `s.is_int()` | bool | True when `s` parses as an integer |
| `s.is_float()` | bool | True when `s` parses as a float but not as an integer |
| `s.repeat(n)` | string | `s` joined to itself `n` times |
| `s.reverse()` | string | A new string with the characters in reverse order |

`len` and `find` count bytes, and indexing and slicing a string are byte-based,
so a string of non-ASCII text does not index by character.

## List methods

Called on an array receiver. `push`, `remove`, `reverse`, and `sort` change the
list in place and return nothing; the rest return a new value.

| Method | Returns | Behaviour |
| --- | --- | --- |
| `arr.len()` | int | The number of elements |
| `arr.push(x)` | nothing | Appends `x` |
| `arr.remove(i)` | nothing | Removes the element at index `i`; raises `index_out_of_bounds` when `i` is outside the list |
| `arr.contains(x)` | bool | True when some element equals `x` |
| `arr.find(x)` | int | The index of the first element equal to `x`, or -1 |
| `arr.repeat(n)` | list | The elements of `arr` repeated `n` times |
| `arr.reverse()` | nothing | Reverses `arr` in place |
| `arr.sort()` | nothing | Sorts `arr` in place, ascending |
| `arr.join()` | string | The elements concatenated; the receiver has to be a list of strings |
| `arr.join(separator)` | string | The same, with `separator` between elements |
| `arr.partition(x)` | list[] | `arr` cut into sub-lists at each element equal to `x` |

`sort` picks its ordering from the first element: ints, floats, and strings sort
ascending, and a list of any other element type is left unchanged.

`arr.find(x)` is the index search. The same spelling with a function argument,
`arr.find(predicate)`, is the [list module](list.md) helper that returns the
matching element. The other higher-order methods (`map`, `filter`, `reduce`,
`each`, `any`, `all`, `sort_by`) and the reductions (`first`, `last`, `sum`,
`min`, `max`, and the rest) also come from that module through the automatic
prelude.

## Map methods

Called on a map receiver.

| Method | Returns | Behaviour |
| --- | --- | --- |
| `m.len()` | int | The number of entries |
| `m.get(k)` | value | The value stored under `k`; raises `unknown_map_key` when the key is absent |
| `m.insert(k, v)` | nothing | Stores `v` under `k`, replacing any existing value |
| `m.contains(k)` | bool | True when `k` is a key in `m` |
| `m.keys()` | list | The keys |
| `m.values()` | list | The values |

A map is a hash map, so the order of `keys`, `values`, and `for k in m` is
unspecified and is not insertion order. `keys` and `values` walk the map the same
way, so their results line up entry for entry.

An empty map literal `{}` takes its key and value types from the first `insert`,
the way an empty list takes its element type from the first `push`. Only a
literal written empty works that way. A map whose entries are typed `any`, which
is what `as_map` hands back, keeps taking entries of any type.

## Number methods

| Method | Receiver | Returns | Behaviour |
| --- | --- | --- | --- |
| `x.abs()` | int or float | same as receiver | The magnitude of `x` |
| `x.sqrt()` | float | float | The square root of `x` |
| `x.round()` | float | float | `x` rounded to the nearest whole number, halves away from zero |
| `x.floor()` | float | float | The largest whole number not greater than `x` |

The [math module](math.md) covers the rest of the numeric surface, including
`ceil`, `trunc`, the trigonometric functions, and the logarithms.
