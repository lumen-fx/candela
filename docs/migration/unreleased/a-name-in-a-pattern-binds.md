# A lowercase name in a match pattern binds, and patterns are one grammar

A `match` arm, `is`, `let`, `for` and parameters now read the same pattern
grammar: literals, `_`, names, ranges, tuples, structs and variants, nested in
one another, with `|` alternatives and `if` guards on match arms. In it a name
that starts with a lowercase letter or `_` always binds. Before, a bare name in
an arm of a match on a number or a string compared the value with the variable
of that name; now it binds a new one and matches everything.

This affects you if a match arm is a lone lowercase name meant to compare with
a variable. Where arms follow such an arm, the compiler reports
`pattern_binds_name`. A `let` pattern that holds a literal, such as
`let (a, 1) = t;`, now reports `let_pattern_can_fail` rather than
`tuple_pattern_element`; give it an `else` block or drop the literal.

Before:

```rust
let limit = 4;
match n {
    limit => print("at the limit"),
    _ => print("other"),
}
```

After:

```rust
let limit = 4;
match n {
    x if x == limit => print("at the limit"),
    _ => print("other"),
}
```
