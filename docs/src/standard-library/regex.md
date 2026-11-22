# regex

Regular expressions, with the syntax of Rust's regex crate.

```rust
import "std/regex";
```

```rust
import "std/regex";

fn main() {
    let digits = regex::compile("[0-9]+")?;
    print(digits.find_all("a1 b22 c333"));
    print(digits.replace_all("a1 b22", "#"));
}
```

The syntax covers classes such as `[a-z]` and `\d`, groups `(...)` and named
groups `(?<name>...)`, alternation `|`, anchors `^` and `$`, repetition with
`*`, `+`, `?`, `{n,m}` and their lazy forms, and flags such as `(?i)`. The
engine runs in time linear in the text, with no backtracking, so it has no
backreferences or look-around. Its classes are ASCII ones: `\w` and `\d` match
ASCII letters and digits only.

The module binds the `regex` native library, which ships in the toolchain's
`libs/std_src/regex/`. A browser runs no native library, so there every call
raises `regex_desktop_only`.

## compile

```rust
regex::compile(pattern)
```

- Returns: a `regex::Regex` for `pattern`, in a `Regex!string`;
  `Err("regex_invalid")` for a pattern the engine refuses.

## Regex

A compiled pattern. Its `pattern` field holds the text it was compiled from.

| Method | Returns |
| --- | --- |
| `re.is_match(s)` | True when the pattern matches somewhere in `s` |
| `re.find(s)` | The first match, as a `string?` |
| `re.find_all(s)` | Every match, left to right, none overlapping the one before |
| `re.captures(s)` | The groups of the first match as a `string[]?`: the whole match first, then each group, with an empty string for a group that took no part |
| `re.replace_all(s, with)` | `s` with every match replaced by `with`, in which `$1` or `${name}` stands for a group and `$$` for a dollar sign |
| `re.split(s)` | The pieces of `s` between matches |
