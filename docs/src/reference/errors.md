<!--
This file is derived from keel (https://github.com/horacehoff/keel),
Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
It has been modified by the candela authors. See the NOTICE file.
-->
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

A message names a type the way you declared it, so a mismatch on a list of your
own enum reads `Value[]` rather than the bare word `enum`.

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
| Number literal | An `int` or a `float` literal whose value is past what the type holds, or an exponent marker with no digits after it, as in `1e` |
| Unclosed delimiter | A `(`, `[` or `{` that is never closed; the report points at the opener |
| Missing semicolon | A statement not ended with `;` |
| Missing separator | Array elements, arguments or parameters not separated by `,` |
| Inline `if` without `else` | An `if` used as an expression must produce a value on every path |
| `try` without `catch` | A `try` block needs at least one `catch` clause |
| `match` without arms | A `match` with no arms, or with only a `_` arm |
| Bad import path | An import path whose extension is neither absent nor `.cdl`, or the removed `import std::string;` form |
| Constant arithmetic | Integer division or remainder by a literal zero, an integer raised to a negative literal exponent, or a shift by a literal count outside 0 to 63 |
| Nested declaration | A `fn` declaration written inside a block instead of at the top level |
| Operator method name | An operator symbol after `fn` in an `impl` block that a type cannot define, such as `>` or `&&`. The report lists the ones it can and says what derives `!=`, `>` and `>=` |
| Nesting too deep | Expressions, blocks or types nested more than 128 levels deep, counting every enclosing level. Move the inner part into a variable or a function |
| Attribute | An `@` name other than `cfg`, an attribute with nothing after it to apply to, or a `@cfg` condition that is not a flag, `key = "value"`, `not(...)` with one condition, `any(...)` or `all(...)`. See [conditional compilation](../language/conditional-compilation.md) |
| Macro | A macro no expander is registered for, a region the file ends before closing, an expander that rejects the region it was given, an expansion that is not a single expression, or macros expanding into one another more than 32 levels deep. See [macros](../language/macros.md) |

## Compile errors

Raised by the type checker once the file parses.

**Unknown names.** A variable, function, method, type, struct, enum variant or
namespace that does not resolve. These reports suggest the closest name in scope
when there is one. A namespace a `host` or `dylib` block declares resolves like
any other, so a call it has no function for is reported against the function,
naming the namespace it was looked for in.

**Struct and field errors.** Reading a field a struct does not declare, building
a struct literal that supplies an unknown field or omits a required one, and
assigning a value of the wrong type to a field.

**Arity and argument errors.** Calling a function with too few or too many
arguments, or with an argument whose type the parameter does not accept, whether
that type was declared with `name: type` or taken from another call. The report
labels the declaration as well as the call. A function that declares `-> Type`
and returns something else is reported against the annotation. A call has the
type its function declares, so an argument taken from a call is checked against
the declared return type, not against what the body returns.

**Operator errors.** An operator applied to operand types it does not accept,
including mixed `int` and `float` arithmetic and a non-`bool` operand of `&&`,
`||` or `!`. The report names the operator it is complaining about, and where an
operand is a struct or an enum it also names the method that type would define
to accept the expression. `==` and `!=` take any pair of built-in values and are
never an error there; on a type that defines its own `==`, an operand that
method does not accept is. See [operators](operators.md).

**Operator method errors.** A method named by an operator symbol carries its own
rules, each with an identifier an editor and an embedder read:
`operator_method_unknown` for a symbol a type cannot define, raised while the
file is read; `operator_method_arity` for a parameter count the operator does not
have; `operator_method_on_builtin` for an operator inside `impl int`, `impl list`
or another built-in type; `operator_method_receiver` for a first parameter
annotated as a type other than the one the `impl` block names; and
`operator_method_return_type` for a method the operator cannot take a value from,
which is one handing nothing back or an `==`, `<` or `<=` not answering a `bool`.
See [methods](../language/methods.md).

**Type errors.** The general mismatch: an index that is not an `int`, indexing
or iterating a type that supports neither, a field access on something that is
not a struct, and the condition of an `if`, an `else if`, a `while` or an `if`
expression whose type is not `bool`. See
[control flow](../language/control-flow.md).

**Collection literal errors.** Arrays and maps are homogeneous, so an element or
value of a different type is rejected, as is a duplicate map key or a map key
that is not a literal.

**Control-flow errors.** An `if` used as an expression with no `else` branch,
and a function that returns a value on some paths and nothing on others, whether
through a bare `return;` or by reaching the end of its body. See
[functions](../language/functions.md).

**Enum and match errors.** An unknown variant, a variant pattern with the wrong
number of payload bindings, a pattern that is not a variant when the scrutinee
is an enum, variant patterns on a value that is not one, and a `match` that does
not cover every variant. The non-exhaustive report lists the variants you left
out. A qualified pattern is also reported when its qualifier names an enum other
than the matched value's, or names nothing at all; the report says which enum
the pattern reaches and which one the match is on.

**Declaration errors.** Defining a function name twice. The report shows both
definitions. Two `impl` blocks that define one method for the same instantiated
generic type are reported the same way.

**Generic type errors.** Type arguments whose count does not match the
declaration, type arguments on a name that declares none, type arguments on a
built-in method, a name in a declaration that is neither a type nor one of its
type parameters, and a generic type whose own fields name a deeper instantiation
of itself without end. See [generics](../language/generics.md).

**Import and library errors.** An import path that cannot be read, a bare import
whose symbols collide with names already in scope, a `dylib` library that cannot
be opened, a symbol the library does not export, and a `dylib` signature naming
a type C has no representation for (`no_c_representation`). `candela check`
opens no library, so it reports neither a library it cannot open nor a missing
symbol. See
[modules](../language/modules.md) and [C libraries](../integration/c-libraries.md).

**No entry point.** A program is entered through `main`, so a run, a build and
an embedded compile all report a file that declares none. `candela check` does
not: compiling a file without a `main` is how a library entry checks. See
[functions](../language/functions.md).

## Warnings

A warning reports something the compile could not check, without stopping it.
`candela check` and `candela build` print each one as a report and still
succeed; an embedding host reads them from `Program::warnings`, and the language
server shows them in the editor. Each carries a code like an error does.

| Code | Meaning |
| --- | --- |
| `unannotated_host_parameter` | A function in the file being built that nothing in the program calls leaves a parameter bare, and the file declares `main`. A file with no `main` is a library, whose importers call its functions, so it gets no such warning. A host calling it by name passes that parameter as `any`, so its body is checked that way. The warning points at the function's name; annotate the parameter with the type the host passes |
| `no_host_entry_point` | The body of such a function does not compile with its bare parameters typed `any`. The message carries the error the body raised and the span points at it. A packaged `.cdlb` has no entry point for the function, so a host call to it fails as an unknown function |

## Runtime errors

Raised while the program runs. Each has a kind, which is the string a `catch`
clause matches and the string bound to the catch variable. See
[error handling](../language/error-handling.md).

### Collections

| Kind | Raised by |
| --- | --- |
| `index_out_of_bounds` | An array or string index outside the value |
| `slice_out_of_bounds` | A slice whose bounds fall outside the value, or whose start is past its end. A slice starting where the value ends is in range and produces an empty one |
| `unknown_map_key` | Reading a map key that is not present |

### Arithmetic

| Kind | Raised by |
| --- | --- |
| `division_by_zero` | Integer division by zero |
| `modulo_by_zero` | Integer remainder by zero |
| `negative_exponent` | An `int` raised to a negative `int` power, which has no `int` result |
| `shift_count_out_of_range` | An `int` shifted by a negative count, or by 64 or more. An `int` is 64 bits wide, so the count must be between 0 and 63 |

Float division and remainder do not raise; they produce an infinity or `NaN`,
and both print as such. A float raised to a negative power does not raise
either. The parser rejects the all-literal forms of these before the program
runs, and a shift is refused wherever its count is a literal out of range; the
kinds above cover the cases where a value is only known at run time.

### Calls

| Kind | Raised by |
| --- | --- |
| `call_depth_exceeded` | A call made with 1000000 calls already standing. The report names the call it stopped at. A recursion that never reaches its base case ends here |

### Conversion

| Kind | Raised by |
| --- | --- |
| `invalid_int` | `int()` on a string that is not an integer |
| `invalid_float` | `float()` on a string that is not a number |
| `invalid_bool` | `bool()` on a string that is neither `true` nor `false` |
| `bad_downcast` | `as_int()`, `as_float()`, `as_str()`, `as_bool()`, `as_list()` or `as_map()` on an `any` value holding a different type, a condition typed `any` holding anything but a bool, a `return` of an `any` value holding a type the function does not declare, and a `return` of an `any` value as a function type when the function it holds leaves a parameter unannotated |
| `not_a_string` | Joining a value onto a string, or comparing it with `<`, `<=`, `>` or `>=`, when the value is not a string. A variadic host function is the way this happens: it is not signature-checked, so its closure can return a type its `host` block does not declare |
| `json_parse_error` | `json::parse` on text that is not valid JSON; the message names the reason. Objects and arrays nest to a fixed depth, and text past it is rejected the same way |

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

### Host functions

| Kind | Raised by |
| --- | --- |
| `host_fn_error` | A function a `host` block declares failed. The message names the function and repeats what the host reported |

An embedding program decides when this happens: its closure returns an error
instead of a value. Catch it like any other kind, or let it end the run. See
[embedding](../integration/embedding.md).

### Your own

`throw("message")` raises an error whose kind is the string you pass, so
`catch "message"` matches it. Choose short, stable identifiers for anything you
intend to catch.

## Catching

A `try` block runs under the innermost `catch`. The errors listed above are
caught wherever the block raises them: in an operator, a built-in function, a
`throw`, or inside a function the block calls, at any depth. When no `catch`
matches, the error is re-raised to the next enclosing `try`, and an error that
reaches the top of the program is printed and ends it.

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

An error raised several frames below the `try` abandons the calls in between and
resumes at the `catch`, so a failure deep in a helper is handled where the work
was started. This covers your own functions and the standard library modules
written in candela.

`exit()` with a non-zero status ends a program without being catchable.

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
survives and receives a structured diagnostic, and the `Program` it called into
stays usable for the next call. See [embedding](../integration/embedding.md).
