# Macros

A macro writes code for you. It is used as `name!( ... )`, and the `!` marks
every place one is involved, so a reader always knows. Nothing in candela
needs a macro: the standard library and every binding work without one. Reach
for one when a function cannot do the job, for example when an argument must
not be evaluated, or when the code has to assign to a variable the caller
names.

A macro comes from one of two places. A program declares one with `macro`,
and the program embedding candela can register one written in its own
language.

## Declaring a macro

`macro`, a name, a pattern in parentheses, and after `=>` the candela the use
expands to:

```rust
macro square($x: expr) => $x * $x;
macro swap($a: ident, $b: ident) => { let t = $a; $a = $b; $b = t; };

fn main() {
    let a = 1;
    let b = 2;
    print(square!(a + 1));
    swap!(a, b);
    print(a, b);
}
```

The pattern lists fragments, each a `$`, a name and the kind of code it takes,
and any other tokens, which a use writes as they are. In the template, `$x`
stands for the code the use gave fragment `x`.

| Kind | Takes |
| --- | --- |
| `expr` | an expression, up to the token after it in the pattern |
| `ident` | one name |
| `type` | a type, up to the token after it in the pattern |
| `block` | a block, `{ ... }` |

An `expr` or a `type` runs to the token that follows it in the pattern, so a
fragment of either kind is followed by a token such as `,` or `=>`, or ends the
pattern. Tokens other than `,` give a macro its own syntax:

```rust
macro pair($a: expr => $b: expr) => ($a, $b);

fn main() {
    print(pair!(1 => "one"));
}
```

An expression fragment keeps its own grouping where the template puts it:
`square!(a + 1)` is `(a + 1) * (a + 1)`, not `a + 1 * a + 1`. Each use
evaluates the fragment as many times as the template names it, so `square!`
of a call makes the call twice; bind it with `let` in the template when it
should run once.

A macro expands to one expression, which may be a block, an `if` or a `match`.
A use written as a whole statement, `swap!(a, b);`, may expand to a block or
an `if` with no value, the way the same code would stand as a statement.

The template ends at the first `;` outside its brackets, and a macro is
declared at the top level of a file, above its first use. A use above the
declaration is a compile error that says so.

## Names in a template

A name the template binds, with `let`, `for`, a closure parameter, an
`else |e|` binder or a `match` arm, belongs to that one expansion. It does not
see a variable of the same name at the use, and the code around the use does
not see it:

```rust
macro add_ten($x: expr) => { let n = 10; $x + n };

fn main() {
    let n = 1;
    print(add_ten!(n));
}
```

prints `11`: the `n` the use passes is the caller's, the `n` the template binds
is its own. Every other name in the template, a function, a type, a constant
or another macro, is read where the macro is used, so a template that calls a
function needs that function in scope at the use.

## Importing a macro

A module's macros import the way its functions do: behind the module's name,
or by name in a list, and `export import` passes them on.

```rust
import "./util.cdl";
import "./util.cdl" { twice };

fn main() {
    print(util::twice!(3), twice!(4));
}
```

## Macros from the host

A host registers a macro in its own language. What goes between the
parentheses of such a macro is not candela: it is raw text handed to the
program that embeds candela, which turns it into candela source and gives it
back. That source is parsed where the macro stands, and may use a macro
itself, up to 32 levels deep; a longer chain is a compile error instead of an
expansion that never ends.

```rust
fn view() -> string {
    return lmn!(<p>Hello</p>);
}
```

candela does not know what `lmn!` means. It finds where the region begins and
ends, calls out, and parses what comes back. Everything else, the syntax inside
the parentheses and the code it becomes, is decided by the host. Read that
host's documentation to learn which macros exist and what they accept. A macro
the program declares or imports is used before a host's macro of the same
name.

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
not expand to a declaration.

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

## An identifier is not a macro

The `!` and the `(` must follow the name immediately for it to be a macro.
`lmn` on its own is an ordinary identifier and `!` on its own is still the
negation operator.

```rust
let lmn = 4;
if !ready { print(lmn); }
```

## When a macro fails

A use that does not match its macro's pattern is a compile error that shows the
pattern. An error inside the code a macro expanded to is reported at the use,
and the editor's hover over a use shows what it expanded to.

A macro that is neither declared, imported, nor registered by the host is a
compile error naming it. For a host's macro this usually means the script is
being run by something other than the program it was written for.

An expander can also reject the region it was given, for example when the markup
inside it is malformed. That is a compile error too, reported at the position in
the region the expander points at.

Tools that read candela without being the host, such as the language server, are
allowed to compile an unregistered macro as `null` instead of failing, so
opening a file in an editor does not report every host macro in it as an error.
See [embedding](../integration/embedding.md) for both sides of this.

A macro inside code that a [`@cfg`](conditional-compilation.md) drops is never
expanded.
