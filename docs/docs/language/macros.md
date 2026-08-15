# Macros

A macro is a piece of syntax that some other language fills in. It is written
`name!( ... )`, and what goes between the parentheses is not candela: it is raw
text handed to the program that embeds candela, which turns it into candela
source and gives it back. That source is parsed where the macro stands.

```rust
fn view() -> string {
    return lmn!(<p>Hello</p>);
}
```

candela does not know what `lmn!` means, and it has no macros of its own. It
finds where a region begins and ends, calls out, and parses what comes back.
Everything else, the syntax inside the parentheses and the code it becomes, is
decided by the host. Read that host's documentation to learn which macros exist
and what they accept.

## Where a macro goes

A macro stands where an expression stands, and it expands to exactly one
expression. It can be assigned, returned, passed as an argument, or used inside
a larger expression.

```rust
fn main() {
    let markup = lmn!(<p>Hello</p>);
    print(width(lmn!(<b/>)));
}
```

It cannot stand at the top level of a file, where declarations go, and it does
not expand to a statement, a block, or a declaration.

## Where a region ends

The region ends at the parenthesis that balances the one that opened it, so
parentheses inside it nest freely. Two things are not counted while looking for
that parenthesis: anything inside a candela string literal, and the rest of a
line after `//`.

```rust
let a = lmn!(f(g(1)));          // ends at the last ')'
let b = lmn!("a)b");            // the ')' is inside a string
let c = lmn!(x // ) not the end
);                              // the ')' was in a comment
```

Nothing else about the region is interpreted, so a macro's own syntax is free
to look nothing like candela.

## An identifier is not a macro

The `!` and the `(` must follow the name immediately for it to be a macro.
`lmn` on its own is an ordinary identifier and `!` on its own is still the
negation operator.

```rust
let lmn = 4;
if !ready { print(lmn); }
```

## When a macro fails

A macro that no expander is registered for is a compile error naming it. This
usually means the script is being run by something other than the program it was
written for; the macro is not misspelt candela, it is missing from the host.

An expander can also reject the region it was given, for example when the markup
inside it is malformed. That is a compile error too, reported at the position in
the region the expander points at.

Tools that read candela without being the host, such as the language server, are
allowed to compile an unregistered macro as `null` instead of failing, so
opening a file in an editor does not report every macro in it as an error. See
[embedding](../integration/embedding.md) for both sides of this.
