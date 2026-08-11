# Variables

A variable binds a name to a value. Candela infers the type from the value, so
a declaration is one keyword, a name, and an expression.

## Declaring

Use `let`. Every statement ends with a semicolon.

```rust
fn main() {
    let name = "Ada";
    let year = 1843;
    let ratio = 0.5;
    let ready = true;
    print(name, year, ratio, ready);
}
```

`let` takes no type annotation; the value decides the type. Write `let n = 0;`,
not `let n: int = 0;`. Function parameters are the other way round and may carry
one, because a parameter has no initialising value to take a type from; see
[Functions](functions.md). See [Types](types.md) for the types a value can have.

## Variables live inside functions

The top level of a file holds declarations only: functions, structs, enums,
`impl` blocks, and imports. There are no global variables, so every `let`
belongs to a function body or a block inside one. A function declaration is the
reverse: it belongs at the top level, and writing one inside a block is a
compile error.

```rust
fn total() {
    let subtotal = 40;
    let shipping = 2;
    return subtotal + shipping;
}

fn main() {
    print(total());
}
```

## Assigning

Assign to an existing variable with `=`. The compound operators `+=`, `-=`,
`*=`, `/=`, `%=`, and `^=` apply an operation in place.

```rust
fn main() {
    let count = 10;
    count = 12;
    count += 5;
    count *= 2;
    print(count);
}
```

An assignment can change a variable's type. The name keeps whatever type the
most recent value gave it, and later code is checked against that type.

```rust
fn main() {
    let value = 1;
    value = "one";
    print(value);
}
```

## Shadowing

Declaring the same name again with `let` starts a fresh variable. This is the
usual way to convert a value and keep the name that describes it.

```rust
fn main() {
    let reply = "42";
    let reply = int(reply);
    print(reply + 1);
}
```

## Scope

A pair of braces introduces a scope. A variable declared inside one is gone at
the closing brace, and a `let` inside a block shadows an outer variable only for
the rest of that block.

```rust
fn main() {
    let depth = 1;
    {
        let depth = 2;
        print(depth);
    }
    print(depth);
}
```

The body of an `if`, a loop, or a `match` arm is a block and scopes the same
way. A `for` loop's variable belongs to the loop and is not visible after it.

```rust
fn main() {
    for step in 0..3 {
        let doubled = step * 2;
        print(doubled);
    }
}
```

Reading a name that is not in scope is a compile error, so a typo or a variable
used past the end of its block is caught before the program runs.
