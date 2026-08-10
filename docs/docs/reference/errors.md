# Errors

candela reports errors in two places: while a program is compiled, and while it
runs. This page describes the classes of both, what triggers them, and the
identifiers a `catch` clause matches on.

## How a report is printed

Every error is printed with the offending source line, the span underlined, and
a message under the underline. The expected thing is coloured blue, the
offending expression red. A compile error also carries a second span when there
is a related location worth showing, such as the declaration of the function you
called.

Compilation stops at the first error. Fixing one and recompiling is how you find
the next; there is no list.

candela emits no warnings. Everything it reports stops the program.

## Parse errors

Raised while the file is read, before any type is known.

| Class | Triggered by |
| --- | --- |
| Unexpected token | A token that cannot appear where it does, including a missing type name, a bad function or variable name, and a struct field name that is not an identifier |
| Unexpected end of file | The file ends inside a construct |
| Unknown token | Characters that do not lex, such as a stray symbol |
| Unclosed delimiter | A `(`, `[` or `{` that is never closed; the report points at the opener |
| Missing semicolon | A statement not ended with `;` |
| Missing separator | Array elements, arguments or parameters not separated by `,` |
| Inline `if` without `else` | An `if` used as an expression must produce a value on every path |
| `try` without `catch` | A `try` block needs at least one `catch` clause |
| `match` without arms | A `match` with no arms, or with only a `_` arm |
| Bad import path | An import path whose extension is neither absent nor `.cdl`, or the removed `import std::string;` form |
| Constant arithmetic | Division or remainder by a literal zero, or an integer raised to a negative literal exponent |

## Compile errors

Raised by the type checker once the file parses.

**Unknown names.** A variable, function, method, type, struct, enum variant or
namespace that does not resolve. These reports suggest the closest name in scope
when there is one.

**Struct and field errors.** Reading a field a struct does not declare, building
a struct literal that supplies an unknown field or omits a required one, and
assigning a value of the wrong type to a field.

**Arity and argument errors.** Calling a function with too few or too many
arguments, or with an argument whose type the parameter does not accept. The
report labels the declaration as well as the call.

**Operator errors.** An operator applied to operand types it does not accept,
including mixed `int` and `float` arithmetic and a non-`bool` operand of `&&`,
`||` or `!`. See [operators](operators.md).

**Type errors.** The general mismatch: a condition that is not a `bool`, an
index that is not an `int`, indexing or iterating a type that supports neither,
a field access on something that is not a struct.

**Collection literal errors.** Arrays and maps are homogeneous, so an element or
value of a different type is rejected, as is a duplicate map key or a map key
that is not a literal.

**Control-flow errors.** An `if` used as an expression with no `else` branch.

**Enum and match errors.** An unknown variant, a variant pattern with the wrong
number of payload bindings, a pattern that is not a variant when the scrutinee
is an enum, and a `match` that does not cover every variant. The
non-exhaustive report lists the variants you left out.

**Declaration errors.** Defining a function name twice. The report shows both
definitions.

**Import and library errors.** An import path that cannot be read, a bare import
whose symbols collide with names already in scope, a `dylib` library that cannot
be opened, and a symbol the library does not export. See
[modules](../language/modules.md) and [C libraries](../integration/c-libraries.md).

A program with no `main` function is also rejected at this stage.

## Runtime errors

Raised while the program runs. Each has a kind, which is the string a `catch`
clause matches and the string bound to the catch variable. See
[error handling](../language/error-handling.md).

### Collections

| Kind | Raised by |
| --- | --- |
| `index_out_of_bounds` | An array or string index outside the value |
| `slice_out_of_bounds` | A slice whose bounds fall outside the value, or whose start is past its end |
| `unknown_map_key` | Reading a map key that is not present |

### Arithmetic

| Kind | Raised by |
| --- | --- |
| `division_by_zero` | Integer division by zero |
| `modulo_by_zero` | Integer remainder by zero |

Float division and remainder do not raise; they produce an infinity or `NaN`.

### Conversion

| Kind | Raised by |
| --- | --- |
| `invalid_int` | `int()` on a string that is not an integer |
| `invalid_float` | `float()` on a string that is not a number |
| `invalid_bool` | `bool()` on a string that is neither `true` nor `false` |
| `bad_downcast` | `as_int()`, `as_float()`, `as_str()`, `as_bool()`, `as_list()` or `as_map()` on an `any` value holding a different type |
| `json_parse_error` | `json::parse` on text that is not valid JSON; the message names the reason |

### Files

Every filesystem builtin maps the operating system's failure to one kind:
`fs_not_found`, `fs_permission_denied`, `fs_already_exists`,
`fs_is_a_directory`, `fs_not_a_directory`, `fs_invalid_filename`,
`fs_invalid_data`, `fs_file_too_large`, `fs_storage_full`,
`fs_read_only_filesystem`, `fs_out_of_memory`, `fs_timed_out`,
`fs_interrupted`, `fs_deadlock`.

### Dynamic libraries

| Kind | Raised by |
| --- | --- |
| `null_byte_in_string` | Passing a string containing an interior null byte to a C function |
| `c_array_return_type_not_supported` | A `dylib` signature that returns an array; C does not convey the length |
| `invalid_return_type` | A return type that has no C representation |

### Your own

`throw("message")` raises an error whose kind is the string you pass, so
`catch "message"` matches it. Choose short, stable identifiers for anything you
intend to catch.

## Catching

A `try` block runs under the innermost `catch`. The errors listed above are
caught where the block itself raises them, in an operator, a built-in function,
or a `throw`. When no `catch` matches, the error is re-raised to the next
enclosing `try`, and an error that reaches the top of the program is printed and
ends it.

```rust
try {
    let text = fs::read("port.txt");
    print(int(text) + 1);
} catch "fs_not_found" {
    print("no config");
} catch e {
    print("failed: " + e);
}
```

The catch variable holds the kind as a string.

A call to a function written in candela does not return when it sits inside a
`try` block, which covers your own functions and the standard library modules
written in candela. Raise and catch within the block itself.

Two things end a program without being catchable: `exit()` with a non-zero
status, and a type that cannot cross the C boundary in a `dylib` signature.

## Artifact load errors

Reported by `candela-vm` before a `.cdlb` runs, and printed as a single line
rather than a source report.

| Class | Meaning |
| --- | --- |
| Bad magic | The file is not a `.cdlb` |
| Truncated | The file is too short to hold a header |
| Unsupported version | The artifact was built by a different format version |
| Decode failure | The body does not decode |
| Library open failure | A dynamic library the artifact needs cannot be opened; the message gives the name as written and the filename it resolved to |
| Symbol not found | The library opened but does not export a symbol the artifact needs |
| Missing host function | The artifact declares a `host` block, which only an embedding runtime can supply |

See [artifacts](artifacts.md).

## Exit behaviour

`candela` and `candela-vm` exit with a non-zero status after reporting any
error, and with zero when the program finishes. Inside a host process, a
candela built with the `embed` feature unwinds instead of exiting, so the host
survives and receives a structured diagnostic. See
[embedding](../integration/embedding.md).
