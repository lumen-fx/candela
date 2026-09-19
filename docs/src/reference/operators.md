# Operators

Every operator candela has, with its precedence, the operand types it accepts,
and the value it produces.

## Precedence

Binary operators bind in this order, loosest first. All are left-associative
except `^`.

| Level | Operators | Associativity |
| --- | --- | --- |
| 1 | `\|\|` | left |
| 2 | `&&` | left |
| 3 | `==` `!=` | left |
| 4 | `<` `<=` `>` `>=` | left |
| 5 | `\|` | left |
| 6 | `^^` | left |
| 7 | `&` | left |
| 8 | `<<` `>>` | left |
| 9 | `+` `-` | left |
| 10 | `*` `/` `%` | left |
| 11 | `^` | right |

`2 ^ 3 ^ 2` is `2 ^ (3 ^ 2)`.

The order is Rust's. The bitwise operators bind tighter than a comparison, so
`flags & mask == mask` asks whether `flags & mask` equals `mask`, and the shifts
bind looser than `+`, so `1 << n + 1` shifts by `n + 1`.

Four forms bind tighter than every binary operator: a function call `f(x)`, an
index or slice `a[i]`, a field access `p.x`, and a method call `p.len()`. They
attach to the term they follow, so `a.b[0] ^ 2` raises `a.b[0]` to the power of
two, and they chain: `adder(1)(2)` calls what `adder(1)` returned, and
`fs[0](x)` calls the element it indexed.

The prefix operators `-`, `!` and `~` bind tighter than `^` and looser than the
postfix forms. `-a ^ 2` is `(-a) ^ 2` and `!ok == done` is `(!ok) == done`. Use
parentheses when you want the other reading.

## Types are never mixed

Both operands of an arithmetic, bitwise or comparison operator must already have
the same type. candela does not promote `int` to `float`, so `1 + 2.0` is a
compile error. Convert first with `float()` or `int()`:

```rust
let total = float(count) + 2.0;
```

`int` is a signed 64-bit integer and `float` is a 64-bit binary floating-point
number. Arithmetic on `int` wraps at the ends of that range. See
[types](../language/types.md).

That rule is about the built-in types. A struct or an enum of your own settles
what an operator on it accepts, including a second operand of another type; see
[user types](#user-types) below.

## Arithmetic

| Operator | Operand types | Result |
| --- | --- | --- |
| `+` | `int`, `int` | `int` |
| `+` | `float`, `float` | `float` |
| `+` | `string`, `string` | `string`, the two joined |
| `+` | `T[]`, `T[]` | `T[]`, a new array holding both |
| `-` `*` `/` `%` `^` | `int`, `int` | `int` |
| `-` `*` `/` `%` `^` | `float`, `float` | `float` |

`+` on arrays builds a new array and leaves both operands untouched:

```rust
let a = [1, 2];
let b = a + [3];
print(a);   // [1, 2]
print(b);   // [1, 2, 3]
```

Integer division truncates towards zero, and `%` takes the remainder with the
sign of the left operand. Integer division or remainder by zero raises the
runtime errors `division_by_zero` and `modulo_by_zero`, both catchable; see
[error handling](../language/error-handling.md). Float division and remainder
follow IEEE 754 and produce infinities or `NaN` instead of raising.

`^` raises the left operand to the power of the right. With `int` operands the
exponent must not be negative: a negative literal is rejected before the program
runs, and a negative value only known at run time raises the catchable
`negative_exponent`. Use floats when you want a fractional result.

Integer arithmetic wraps on overflow.

An `any` value cannot be used as an arithmetic operand. Narrow it first with
`as_int()` or `as_float()`.

## Bitwise

`&`, `|`, `^^`, `~`, `<<` and `>>` work on the bits of an `int` and produce an
`int`. `^` is the power operator, so xor takes the doubled spelling `^^`, the
way `&&` and `||` take theirs.

| Operator | Operand types | Result |
| --- | --- | --- |
| `&` | `int`, `int` | `int`, the bits set in both |
| `\|` | `int`, `int` | `int`, the bits set in either |
| `^^` | `int`, `int` | `int`, the bits set in one of the two |
| `~` | `int` | `int`, every bit flipped |
| `<<` | `int`, `int` | `int`, the bits moved up |
| `>>` | `int`, `int` | `int`, the bits moved down |

No other operand type is accepted, and the types are never mixed: `1.5 & 1` is
the same compile error `1.5 * 1` is, and an `any` value has to be narrowed with
`as_int()` first.

```rust
let read = 4;
let write = 2;
let mode = read | write;
print(mode & read == read);   // true
print(~0);                    // -1
```

`int` is signed, so `>>` is arithmetic: the vacated places take the sign bit,
and `-16 >> 2` is `-4`. `<<` fills with zeroes and wraps like the rest of
integer arithmetic, so `1 << 63` is the smallest `int`.

An `int` holds 64 bits, so both shifts take a count between 0 and 63. A count
written as a literal outside that range is refused before the program runs; a
count only known at run time raises the catchable `shift_count_out_of_range`.

## Comparison

`<`, `<=`, `>` and `>=` produce a `bool`. Both operands have to be the same
type, and that type has to be one of three:

| Operator | Operand types | Result |
| --- | --- | --- |
| `<` `<=` `>` `>=` | `int`, `int` | `bool` |
| `<` `<=` `>` `>=` | `float`, `float` | `bool` |
| `<` `<=` `>` `>=` | `string`, `string` | `bool` |

Strings order by their bytes, which is the order `sort` puts a list of strings
in. A string that prefixes another comes first, so `"ab" < "abc"`. Bytes are
not letters: every uppercase ASCII letter comes before every lowercase one, so
`"Z" < "a"`, and every ASCII character comes before a character that takes more
than one byte. Lowercase both sides when you want a case-insensitive order.

```rust
fn main() {
    let names = ["pear", "Apple", "fig"];
    names.sort();
    print(names);            // ["Apple","fig","pear"]
    print("fig" < "pear");   // true
}
```

There is no ordering on arrays, maps, structs or enums.

## Equality

`==` and `!=` produce a `bool` and accept operands of any type. Two values of
different types are never equal, so `"5" == 5` is false and `"5" != 5` is true.
That holds whether the mismatch is visible at compile time or only shows up at
run time through a parameter whose type comes from the call. Comparing values of
the same type is the case worth writing.

| Operand type | How it compares |
| --- | --- |
| `int`, `float`, `bool`, `null` | by value |
| `string` | by contents |
| `T[]` | by length, then element by element |
| struct | by type, then field by field |
| enum | by variant, then payload |
| map | by length, then key by key |

Comparison recurses, so nested collections compare all the way down.

Floats compare by their exact representation: `0.0 == -0.0` is false, and any
comparison involving `NaN` is false.

## Logical

`&&`, `||` and `!` take `bool` operands and produce a `bool`. No other type is
accepted, so there is no truthiness; write the comparison out:

```rust
if count > 0 && !done {
    print("working");
}
```

`&&` and `||` short-circuit wherever they appear: the right operand is not
evaluated when the left one already settles the answer. That holds in a
condition, in a `let`, in an argument, and in a returned expression alike, so a
right operand with a side effect runs only when it is reached.

## User types

A struct or an enum defines an operator by declaring a method named with the
symbol, and the operator on a value of that type calls it. See
[methods](../language/methods.md).

```rust
impl Vec2 {
    fn +(self, other: Vec2) -> Vec2 {
        return Vec2 { x: self.x + other.x, y: self.y + other.y };
    }
}
```

| Operators | Form |
| --- | --- |
| `+` `-` `*` `/` `%` `^` `&` `\|` `^^` `<<` `>>` | `fn op(self, other)` |
| `==` `<` `<=` | `fn op(self, other) -> bool` |
| `-` `~` | `fn op(self)` |

`-` is the one symbol with both forms, and the parameter count says which is
being declared. `!=`, `>` and `>=` have no method: they come from `==`, `<` and
`<=`. A derived comparison swaps its operands, so it evaluates the right one
first.

The other operand is of the receiver's type unless the method declares
otherwise. Only the left operand picks the method, so `2.0 * v` stays a type
error however `Vec2` defines `*`. The built-in types are closed: an operator
method inside `impl int` or `impl string` is a compile error. `&&`, `||`, `!`,
`=` and the compound assignments are not defined by a type; the compound forms
apply the matching operator instead.

## Assignment

`=` assigns to a variable, an array element, or a struct field:

```rust
count = 4;
row[0] = 9;
point.x = 3;
```

Assignment is a statement, not an expression, so it produces no value and cannot
be chained.

The compound forms `+=`, `-=`, `*=`, `/=`, `%=`, `^=`, `&=`, `|=`, `^^=`, `<<=`
and `>>=` apply the matching binary operator to the current value and assign the
result. `x += 1` is `x = x + 1`, with the same type rules, and all three
assignable targets are allowed:

```rust
total += price;
counts[i] += 1;
point.x *= 2;
flags |= read;
```

A compound form on a value of your own type applies the operator that type
defines, so `v += w` reaches its `+` method and needs nothing of its own.

Use `let` to introduce a name and `=` to change one; see
[variables](../language/variables.md).

## Indexing and slicing

`a[i]` reads one element of an array, or one character of a string as a
one-character string. The index must be an `int` and must be within bounds; a
negative index does not count from the end, and an out-of-range index raises the
catchable `index_out_of_bounds`.

`a[i..j]` takes a slice from `i` up to but not including `j`. `a[..j]` starts at
zero. Both bounds must be `int`. Either bound may equal the length, and a slice
that falls outside the value raises `slice_out_of_bounds`. There is no `a[i..]`
form; give the upper bound.

```rust
let word = "candela";
print(word[0]);      // c
print(word[0..4]);   // cand
```

Positions on a string count characters, so `word[0..4]` on a four-letter word
ending in an accented vowel takes the whole word, and `word[3]` reads the
accented vowel. A character is a Unicode scalar value, whatever it takes to
store.

Maps are not indexed with `[]`. Use the `get` method; see
[collections](../language/collections.md).

## Access and calls

`.` reads a struct field or calls a method on a value. `::` separates the parts
of a namespaced name: a module bound with `import ... as`, an enum variant, or a
function inside either.

```rust
print(point.x);
print(name.len());
print(name.capitalize());
let c = Colour::Red;
```

A `host` or `dylib` block's name takes either one, so `app.rows(id)` and
`app::rows(id)` are the same call, and a variable of that name takes the dot
back for its own methods. See [C libraries](../integration/c-libraries.md).

See [methods](../language/methods.md), [enums](../language/enums.md) and
[modules](../language/modules.md).

## Ranges

`..` builds the range of a `for` loop and appears nowhere else. Both bounds must
be `int`, and the end is exclusive. `for i in ..n` starts at zero.

```rust
for i in 0..3 {
    print(i);       // 0, 1, 2
}
```

See [control flow](../language/control-flow.md).

## Symbols that are not operators

- `|` is not an operator where a type is read: it separates the members of a
  union type, as in `int | string`. Where an expression is read it is bitwise
  or. Nothing is ambiguous about that, because a type and an expression never
  stand in the same place.
- `...` marks a variadic host function in a `host` block. See
  [embedding](../integration/embedding.md).
- `->` gives the return type in a `dylib` or `host` signature, and `=>`
  separates a `match` pattern from its body.

## Constant expressions

An expression built only from literals is folded while the file is parsed, so it
costs nothing at run time. Folding applies the same type rule as everything
else: `2.0 ^ 3` is rejected for mixing a `float` with an `int`, exactly as it
would be if the operands were variables.

Folding also turns four mistakes into compile errors rather than runtime ones:
dividing by a literal `0`, taking the remainder by a literal `0`, raising an
integer to a negative literal exponent, and shifting by a literal count outside
0 to 63. Those apply to integer arithmetic. A
`float` divided or remaindered by `0.0` follows IEEE 754 and produces an
infinity or `NaN`, so it is folded rather than rejected.
