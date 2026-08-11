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
condition of another type compiles but is not interpreted, and takes the true
branch for every value except the boolean `false`. The same holds for `else if`,
for `while`, and for the expression form below.

### if as an expression

An `if` written where a value is expected produces a value. Each branch is a
single expression with no semicolon, and an `else` branch is required so that
every path yields something.

```rust
fn main() {
    let size = if 3 > 2 { "big" } else { "small" };
    print(size);
}
```

## while

`while` repeats a block for as long as its condition holds. The condition is a
`bool`, exactly as in an `if`, and a value of another type behaves the same way
it does there.

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

A range is written `start..end` and covers `start` up to but not including
`end`. Leaving the start out begins at zero.

```rust
fn main() {
    for i in 0..3 {
        print(i);
    }
    for i in ..2 {
        print(i);
    }
}
```

Iterating a list binds each element in turn; iterating a map binds each key.

```rust
fn main() {
    for word in ["a", "b"] {
        print(word);
    }

    let counts = {"x": 1, "y": 2};
    for key in counts {
        print(key, counts.get(key));
    }
}
```

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
work in `for`, `while`, and `loop`.

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

`match` compares a value against a list of arms and runs the first that fits.
Arms are written `pattern => { ... }`, and `_` is the catch-all. A `match` needs
at least one arm that is not the wildcard, and the wildcard comes last.

```rust
fn main() {
    let code = 2;
    match code {
        1 => { print("one"); }
        2 => { print("two"); }
        _ => { print("something else"); }
    }
}
```

Arms match by equality, so any type you can compare works, strings included.

```rust
fn main() {
    match "b" {
        "a" => { print("first"); }
        "b" => { print("second"); }
        _ => { print("other"); }
    }
}
```

Matching an enum matches on the variant instead, and binds the payload; see
[Enums](enums.md).

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

Loop bodies, `if` branches, and `match` arms are blocks and scope the same way.
See [Variables](variables.md) for the scoping rules.

A block holds statements. A `fn` declaration written inside one is a compile
error, since functions belong at the top level of the file. See
[Functions](functions.md).
