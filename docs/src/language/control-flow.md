# Control flow

Candela has `if`, three loops, and `match`. Every body is a brace-delimited
block, and braces are never optional.

## if / else

```rust
fn main() {
    let temperature = 30;
    if temperature > 25 {
        print("warm");
    } else if temperature > 10 {
        print("mild");
    } else {
        print("cold");
    }
}
```

The condition is not parenthesised. It is a `bool` expression: candela has no
truthiness, so compare explicitly rather than testing a number or a string. A
condition of another type is a compile error naming the type it has, and a
condition typed `any` is checked when it runs and raises `bad_downcast` on
anything but a bool; see [errors](../reference/errors.md). The same holds for
`else if`, for `while`, and for the expression form below.

A condition that tests a value with `is` gives its body more than a bool: a
tested variable has the type it was tested for, and a variant test such as
`opt is Some(x)` binds the payload. See
[testing a type](types.md#testing-a-type-with-is).

### if as an expression

An `if` written where a value is expected produces a value. Each branch is a
block whose last expression, written without a semicolon, is its value; the
statements before it run first. An `else` branch is required so that every
path yields something. A [match](#match-as-an-expression) and a plain
[block](#blocks-and-scope) work the same way.

```rust
fn main() {
    let n = 7;
    let size = if n > 5 {
        let excess = n - 5;
        excess * 2
    } else {
        0
    };
    print(size);
}
```

A branch can leave instead of giving a value: one that ends in `return`,
`throw`, `break` or `continue` gives nothing, and the other branches decide
the type.

```rust
fn half(n: int) -> int {
    let even = if n % 2 == 0 { n } else { return -1; };
    return even / 2;
}

fn main() {
    print(half(8), half(3));
}
```

## while

`while` repeats a block for as long as its condition holds. The condition is a
`bool`, exactly as in an `if`, and a value of another type is rejected the same
way it is there.

```rust
fn main() {
    let countdown = 3;
    while countdown > 0 {
        print(countdown);
        countdown -= 1;
    }
}
```

## for

`for` walks a range or a collection.

A [range](../standard-library/range.md) is written `start..end` and covers
`start` up to but not including `end`; `start..=end` covers `end` too. Leaving
the start out, `..end`, begins at zero. A range written in place counts in a
register, and a range held in a variable loops the same way, in either
direction.

```rust
fn main() {
    for i in 0..3 {
        print(i);
    }
    for i in ..2 {
        print(i);
    }
    let countdown = (1..=3).rev();
    for i in countdown {
        print(i);
    }
}
```

Iterating a list binds each element in turn; iterating a map binds each key.
Iterating a string binds each character as a one-character string.

```rust
fn main() {
    for word in ["a", "b"] {
        print(word);
    }

    let counts = {"x": 1, "y": 2};
    for key in counts {
        print(key, counts[key]);
    }
}
```

A [pattern](#patterns) after `for` takes each element apart the way a
[`let` pattern](variables.md#taking-a-value-apart) does: one name per place of
a tuple, `_` to skip a place, a nested pattern for a tuple inside it, and a
struct pattern for a struct. The pattern has to fit every element.

```rust
fn main() {
    let scores = [("ada", 3), ("bob", 5)];
    for (name, points) in scores {
        print(name, points);
    }
    for (_, (x, y)) in [("a", (1, 2))] {
        print(x + y);
    }
}
```

A name a pattern binds to a struct is that element, not a copy, so changing a
field through it changes the list.

The loop variable belongs to the loop and is not in scope after it.

## loop

`loop` repeats until something breaks out of it.

```rust
fn main() {
    let n = 0;
    loop {
        n += 1;
        if n == 3 {
            break;
        }
        print(n);
    }
}
```

## break and continue

`break` leaves the innermost loop; `continue` skips to its next iteration. Both
work in `for`, `while`, and `loop`, and only in a loop of their own function:
one with no loop around it is a compile error, even when the function is
called from inside a loop.

```rust
fn main() {
    for i in 0..5 {
        if i == 1 {
            continue;
        }
        if i == 3 {
            break;
        }
        print(i);
    }
}
```

## match

`match` runs the first arm whose pattern fits the value. An arm is
`pattern => { ... }` with a block, or `pattern => expression,` with one
expression and a comma before the next arm; the comma after the last arm is
optional. `_` is the catch-all. A `match` needs at least one arm that is not
the wildcard, and the wildcard comes last.

```rust
fn main() {
    let code = 2;
    match code {
        1 => print("one"),
        2 => {
            print("two");
            print("and more");
        }
        _ => print("something else"),
    }
}
```

An expression arm can also be `break`, `continue`, `return` or `throw`, which
leave the loop or the function the way they do inside a block.

```rust
fn main() {
    for i in 0..10 {
        match i {
            1 => continue,
            4 => break,
            _ => print(i),
        }
    }
}
```

### Patterns

One pattern grammar serves every place that binds a name: `match` arms, `is`,
`let`, `for` and the parameters of functions and closures.

| Pattern | Fits |
| --- | --- |
| `_` | anything, and binds nothing |
| `n`, `rest` | anything, and binds it to the name |
| `3`, `-1.5`, `"go"`, `true`, `null` | a value equal to the literal |
| `1..10`, `1..=9` | a number in the range; `..=` holds the high end too |
| `(a, b)` | a [tuple](types.md#tuples), element by element |
| `Point { x, y: py, .. }` | a struct, field by field; `x` alone binds the field `x`, `..` skips the fields it does not name |
| `Some(v)`, `Shape::Circle(r)`, `Click { x, y }` | one [variant](enums.md), then its payload |
| `MAX`, `limits::TOP` | a value equal to the constant |
| `a \| b` | either one; both bind the same names at the same types |

A name that starts with a lowercase letter or `_` always binds. A name that
starts with a capital names a variant or a constant. To compare with a
variable, bind a name and test it in a guard: `if` after the pattern adds a
condition the arm also needs, and it reads what the pattern bound.

```rust
fn grade(score: int, pass: int) -> string {
    return match score {
        90..=100 => "top",
        s if s >= pass => "pass",
        0 | 1 => "barely tried",
        _ => "fail",
    };
}

fn main() {
    print(grade(95, 50), grade(60, 50), grade(1, 50), grade(20, 50));
}
```

Patterns nest, so one arm can take a value apart as deep as it needs.

```rust
fn describe(p: (int, int)) -> string {
    return match p {
        (0, 0) => "origin",
        (0, y) | (y, 0) => "on an axis at " + string(y),
        (x, y) => "at " + string(x) + ", " + string(y),
    };
}

fn main() {
    print(describe((0, 0)), describe((0, 4)), describe((2, 3)));
    match Some((1, "one")) {
        Some((0, _)) => print("zero"),
        Some((n, name)) => print(n, name),
        None => print("nothing"),
    }
}
```

An arm whose pattern always fits, with no guard, stands in for the `_` arm.
A lone name that a variable in scope already has, followed by more arms, is
an error: it would bind and never let the arms after it run.

### match as an expression

A `match` written where a value is expected produces a value: the value of the
arm that fits. Like an `if` used as a value, it has to give one on every path,
so it needs a `_` arm unless its arms with no guard cover every variant of an
enum, or one of them has a pattern that always fits, such as a tuple of names. A braced arm is a
block whose last expression is its value, and any arm can leave through
`break`, `continue`, `return` or `throw` instead.

```rust
fn main() {
    let n = 3;
    let size = match n {
        0 => "none",
        1 | 2 => "few",
        _ => {
            let label = "many";
            label
        }
    };
    print(size);
}
```

## Blocks and scope

A bare `{ ... }` is a block. It groups statements and scopes the variables
declared inside it, which is occasionally useful for keeping a temporary out of
the surrounding function.

```rust
fn main() {
    let total = 0;
    {
        let temp = 5;
        total += temp;
    }
    print(total);
}
```

A block written where a value is expected is a value too: its last
expression, written without a semicolon. Use it to compute a value with a few
temporaries that do not outlive it.

```rust
fn main() {
    let area = {
        let w = 4;
        let h = 3;
        w * h
    };
    print(area);
}
```

An `if` or a `match` that is the last thing in such a block gives the block
its value the same way. `{}` on its own is an empty
[map](collections.md), not an empty block.

Loop bodies, `if` branches, and `match` arms are blocks and scope the same way.
See [Variables](variables.md) for the scoping rules.

A block holds statements. A `fn` declaration written inside one is a compile
error, since functions belong at the top level of the file. See
[Functions](functions.md).
