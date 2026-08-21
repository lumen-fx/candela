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
| 5 | `+` `-` | left |
| 6 | `*` `/` `%` | left |
| 7 | `^` | right |

`2 ^ 3 ^ 2` is `2 ^ (3 ^ 2)`.

Four forms bind tighter than every binary operator: a function call `f(x)`, an
index or slice `a[i]`, a field access `p.x`, and a method call `p.len()`. They
attach to the term they follow, so `a.b[0] ^ 2` raises `a.b[0]` to the power of
two.

The prefix operators `-` and `!` bind tighter than `^` and looser than the
postfix forms. `-a ^ 2` is `(-a) ^ 2` and `!ok == done` is `(!ok) == done`. Use
parentheses when you want the other reading.

## Types are never mixed

Both operands of an arithmetic or comparison operator must already have the same
type. candela does not promote `int` to `float`, so `1 + 2.0` is a compile
error. Convert first with `float()` or `int()`:

```rust
let total = float(count) + 2.0;
```

`int` is a signed 32-bit integer and `float` is a 64-bit binary floating-point
number. See [types](../language/types.md).

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

## Comparison

`<`, `<=`, `>` and `>=` take two `int` operands or two `float` operands and
produce a `bool`. There is no ordering on strings, arrays, maps, structs or
enums.

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
| map | by identity, not by contents |

Array, struct and enum comparison recurses, so nested collections compare all
the way down, and a map nested inside one of them compares by its contents.

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

## Assignment

`=` assigns to a variable, an array element, or a struct field:

```rust
count = 4;
row[0] = 9;
point.x = 3;
```

Assignment is a statement, not an expression, so it produces no value and cannot
be chained.

The compound forms `+=`, `-=`, `*=`, `/=`, `%=` and `^=` apply the matching
binary operator to the current value and assign the result. `x += 1` is
`x = x + 1`, with the same type rules, and all three assignable targets are
allowed:

```rust
total += price;
counts[i] += 1;
point.x *= 2;
```

Use `let` to introduce a name and `=` to change one; see
[variables](../language/variables.md).

## Indexing and slicing

`a[i]` reads one element of an array, or one byte of a string as a
one-character string. The index must be an `int` and must be within bounds; a
negative index does not count from the end, and an out-of-range index raises the
catchable `index_out_of_bounds`.

`a[i..j]` takes a slice from `i` up to but not including `j`. `a[..j]` starts at
zero. Both bounds must be `int`. The end may equal the length, the start may
not, and a slice that falls outside the value raises `slice_out_of_bounds`.
There is no `a[i..]` form; give the upper bound.

```rust
let word = "candela";
print(word[0]);      // c
print(word[0..4]);   // cand
```

Strings index and slice by byte, so a multi-byte character does not survive
being taken apart this way.

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

- `|` separates the members of a union type, as in `int | string`. It is not a
  bitwise operator; candela has no bitwise operators.
- `...` marks a variadic host function in a `host` block. See
  [embedding](../integration/embedding.md).
- `->` gives the return type in a `dylib` or `host` signature, and `=>`
  separates a `match` pattern from its body.

## Constant expressions

An expression built only from literals is folded while the file is parsed, so it
costs nothing at run time. Folding applies the same type rule as everything
else: `2.0 ^ 3` is rejected for mixing a `float` with an `int`, exactly as it
would be if the operands were variables.

Folding also turns three mistakes into compile errors rather than runtime ones:
dividing by a literal `0`, taking the remainder by a literal `0`, and raising an
integer to a negative literal exponent. Those apply to integer arithmetic. A
`float` divided or remaindered by `0.0` follows IEEE 754 and produces an
infinity or `NaN`, so it is folded rather than rejected.
