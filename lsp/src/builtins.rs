//! Static tables for keywords and built-in functions/methods.
//!
//! These are not derived from the frontend: candela's lexer keywords are
//! fixed tokens (`src/parser/lexer.rs`) and its built-ins are Rust match
//! arms in `src/compiler/functions/builtin/{builtin_functions,
//! builtin_methods}.rs`, neither of which is exposed as a runtime-queryable
//! table. The lists and descriptions below are transcribed from those files
//! and from `docs/src/standard-library/builtins.md`, and must be
//! kept in sync by hand if candela's built-ins change.

/// Reserved words, from `Token` in `src/parser/lexer.rs`.
pub const KEYWORDS: &[&str] = &[
    "fn", "let", "struct", "if", "else", "match", "while", "for", "in", "loop", "return", "break",
    "continue", "try", "catch", "throw", "import", "as", "is", "host", "dylib", "true", "false",
    "null",
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
        "string",
        "string(T) -> string\n\nReturns the given value as a string.",
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
        "trim_start",
        "<string>.trim_start() -> string\n\nReturns the string with leading whitespace removed.",
    ),
    (
        "trim_end",
        "<string>.trim_end() -> string\n\nReturns the string with trailing whitespace removed.",
    ),
    (
        "trim_chars",
        "<string>.trim_chars(chars: string) -> string\n\nReturns the string with every character in `chars` removed from its start and end.",
    ),
    (
        "trim_start_chars",
        "<string>.trim_start_chars(chars: string) -> string\n\nReturns the string with every character in `chars` removed from its start.",
    ),
    (
        "trim_end_chars",
        "<string>.trim_end_chars(chars: string) -> string\n\nReturns the string with every character in `chars` removed from its end.",
    ),
    (
        "index_of",
        "<string>.index_of(e: string) -> Option<int>\n<T[]>.index_of(e: T) -> Option<int>\n\nReturns the position of the first `e` as `Some`, or `None` when there is none.",
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
        "<T[]>.remove(n: int)\n<{K: V}>.remove(key: K)\n\nRemoves the n-th element from an array, or the entry under `key` from a map, in place. Removing a key a map does not hold changes nothing.",
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
        "<string>.split(separator: string) -> string[]\n<T[]>.split(separator: T) -> T[][]\n\nSplits a string or a list into the runs between each `separator`.",
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
        "parse",
        "<string>.parse<T>() -> Option<T>\n\nReads an int, a float or a bool out of the string, or `None` where it holds none.",
    ),
];
