//! Static tables for keywords and built-in functions/methods.
//!
//! These are not derived from the frontend: candela's lexer keywords are
//! fixed tokens (`src/parser/lexer.rs`) and its built-ins are Rust match
//! arms in `src/compiler/functions/builtin/{builtin_functions,
//! builtin_methods}.rs`, neither of which is exposed as a runtime-queryable
//! table. The lists and descriptions below are transcribed from those files
//! and from `docs/docs/standard-library/built-in-functions.md`, and must be
//! kept in sync by hand if candela's built-ins change.

/// Reserved words, from `Token` in `src/parser/lexer.rs`.
pub const KEYWORDS: &[&str] = &[
    "fn", "let", "struct", "if", "else", "match", "while", "for", "in", "loop", "return", "break",
    "continue", "try", "catch", "import", "as", "host", "dylib", "true", "false", "null",
];

/// Free-standing built-in functions, from
/// `src/compiler/functions/builtin/builtin_functions.rs`.
pub const BUILTIN_FUNCTIONS: &[(&str, &str)] = &[
    ("print", "print(T)\n\nPrints anything."),
    (
        "type",
        "type(T) -> string\n\nReturns the type of the object as a string.",
    ),
    (
        "float",
        "float(string | int) -> float\n\nReturns the string or int interpreted as a float. Crashes at runtime if the string cannot be converted.",
    ),
    (
        "int",
        "int(string | float) -> int\n\nReturns the string or float interpreted as an int. Crashes at runtime if the string cannot be converted.",
    ),
    (
        "str",
        "str(T) -> string\n\nReturns the given object as a string.",
    ),
    (
        "bool",
        "bool(s: string) -> bool\n\nReturns `s` interpreted as a boolean. Crashes at runtime if `s` cannot be converted.",
    ),
    (
        "input",
        "input() -> string\ninput(p: string) -> string\n\nAsks the user for input, optionally printing prompt `p` first.",
    ),
    (
        "range",
        "range(j: int) -> int[]\nrange(i: int, j: int) -> int[]\n\nReturns an array containing the numbers from 0 or `i` to `j`-1.",
    ),
    (
        "the_answer",
        "the_answer() -> int\n\nPrints \"The answer to the Ultimate Question of Life, the Universe, and Everything is 42.\" and returns 42.",
    ),
    (
        "argv",
        "argv() -> string[]\n\nReturns the arguments passed to the script, excluding the interpreter path and script name.",
    ),
    (
        "exit",
        "exit()\nexit(exit_code: int)\n\nExits the program with the given exit code, or 0 if not provided.",
    ),
    (
        "throw",
        "throw(error: string)\n\nThrows an error, catchable with a `try`/`catch` block.",
    ),
];

/// Method-call built-ins (`<value>.method(...)`), from
/// `src/compiler/functions/builtin/builtin_methods.rs`.
pub const BUILTIN_METHODS: &[(&str, &str)] = &[
    (
        "uppercase",
        "<string>.uppercase() -> string\n\nReturns the given string as uppercase.",
    ),
    (
        "lowercase",
        "<string>.lowercase() -> string\n\nReturns the given string as lowercase.",
    ),
    (
        "starts_with",
        "<string>.starts_with(s: string) -> bool\n\nReturns whether the string starts with `s`.",
    ),
    (
        "ends_with",
        "<string>.ends_with(s: string) -> bool\n\nReturns whether the string ends with `s`.",
    ),
    (
        "replace",
        "<string>.replace(a: string, b: string) -> string\n\nReturns the string with all occurrences of `a` replaced with `b`.",
    ),
    (
        "len",
        "<string | T[]>.len() -> int\n\nReturns the length of the given collection.",
    ),
    (
        "contains",
        "<string>.contains(e: string) -> bool\n<T[]>.contains(e: T) -> bool\n\nReturns whether the collection contains `e`.",
    ),
    (
        "trim",
        "<string>.trim() -> string\n\nReturns the string with leading and trailing whitespace removed.",
    ),
    (
        "trim_sequence",
        "<string>.trim_sequence(s: string) -> string\n\nReturns the string with `s` removed from its start and end.",
    ),
    (
        "trim_left",
        "<string>.trim_left() -> string\n\nReturns the string with leading whitespace removed.",
    ),
    (
        "trim_right",
        "<string>.trim_right() -> string\n\nReturns the string with trailing whitespace removed.",
    ),
    (
        "trim_sequence_left",
        "<string>.trim_sequence_left(s: string) -> string\n\nReturns the string with `s` removed from its start.",
    ),
    (
        "trim_sequence_right",
        "<string>.trim_sequence_right(s: string) -> string\n\nReturns the string with `s` removed from its end.",
    ),
    (
        "find",
        "<string>.find(e: string) -> int\n<T[]>.find(e: T) -> int\n\nReturns the index of `e`, or -1 if not found.",
    ),
    (
        "repeat",
        "<string>.repeat(n: int) -> string\n<T[]>.repeat(n: int) -> T[]\n\nReturns the collection repeated `n` times.",
    ),
    (
        "push",
        "<T[]>.push(e: T)\n\nAdds `e` to the end of an array, in place.",
    ),
    (
        "remove",
        "<T[]>.remove(n: int)\n\nRemoves the n-th element from an array, in place.",
    ),
    (
        "sqrt",
        "<float>.sqrt() -> float\n\nReturns the square root.",
    ),
    (
        "round",
        "<float>.round() -> float\n\nRounds to the nearest int (still a float).",
    ),
    ("floor", "<float>.floor() -> float\n\nFloors a float."),
    (
        "abs",
        "<float>.abs() -> float\n<int>.abs() -> int\n\nReturns the absolute value.",
    ),
    (
        "reverse",
        "<T[]>.reverse()\n<string>.reverse() -> string\n\nReverses a collection in place (arrays) or returns the reversed string.",
    ),
    (
        "split",
        "<string>.split(separator: string) -> string[]\n\nSplits a string on `separator`.",
    ),
    (
        "partition",
        "<T[]>.partition(separator: T) -> T[][]\n\nPartitions a collection on `separator`.",
    ),
    (
        "join",
        "<string[]>.join() -> string\n<string[]>.join(separator: string) -> string\n\nJoins all elements into a single string.",
    ),
    (
        "sort",
        "<T[]>.sort()\n\nSorts an array in place. Supports ints, floats, and strings.",
    ),
    (
        "get",
        "<{K: V}>.get(key: T) -> V\n\nReturns the value for `key`. Raises `unknown_map_key` if absent.",
    ),
    (
        "insert",
        "<{K: V}>.insert(key: T, value: V)\n\nInserts or updates a key-value pair in the map.",
    ),
    (
        "is_float",
        "<string>.is_float() -> bool\n\nReturns whether the string represents a float.",
    ),
    (
        "is_int",
        "<string>.is_int() -> bool\n\nReturns whether the string represents an int.",
    ),
];
