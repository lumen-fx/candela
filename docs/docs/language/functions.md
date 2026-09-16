# Functions

A function is declared with `fn` at the top level of a file. Everything a
program does happens inside one.

## Declaring and calling

```rust
fn greet(name) {
    print("hello " + name);
}

fn main() {
    greet("Ada");
}
```

A parameter left as a plain name takes its type from the call. candela compiles
a separate specialisation of the function for each combination of argument types
a call site uses, so one definition serves every type it can work with.

```rust
fn same(x) {
    return x;
}

fn main() {
    print(same(1), same("a"), same(1.5));
    print(type(same(1)), type(same("a")));
}
```

Writing `name: type` instead pins that parameter, and an argument of any other
type is a compile error. Pin the ones you want fixed and leave the rest open;
the two styles mix freely in one signature.

```rust
fn repeat(text: string, times) {
    let out = "";
    for _ in 0..times {
        out = out + text;
    }
    return out;
}

fn main() {
    print(repeat("ab", 3));
}
```

An annotation is worth writing when a function is called from Rust rather than
from the script, since there is no call site to take the types from; see
[embedding](../integration/embedding.md). A parameter annotated `any` stays
dynamic and accepts a value of any type.

Annotating every parameter also decides when the body is checked. A function
whose parameters are all annotated is compiled at those types whether or not
anything calls it, so an error in it is reported at compile time. `any` is one
of those types, and it carries no operations of its own: a body that adds,
indexes or compares an `any` parameter has to name the type it wants, either by
annotating the parameter concretely or by converting inside the body.

```candela
fn plus_one(x: any) -> int {
    return as_int(x) + 1;
}
```

A function with a bare parameter has no declared type to compile against, so its
body is first checked by the call that reaches it.

Declaration order does not matter, so a function may call one declared further
down the file. Two functions cannot share a name: there is no overloading, and a
repeated name is a compile error.

A declaration may take the name of a [built-in](../standard-library/builtins.md),
and the declaration wins. `fn str(n: int) -> int` makes `str` the program's own
function wherever the file calls it, with its own argument and return types, and
the built-in conversion is out of reach there. An imported name counts as
declared, so a module that exports `read` is the `read` its importers call. The
built-in methods are the exception: one of their names in an `impl` block never
resolves, as [methods](methods.md) describes.

`fn` belongs at the top level of a file. A declaration inside a block, including
inside another function's body, is a compile error; move it out to the top
level.

## main

`main` is the entry point. Running a file calls it, and only the `main` of the
file you run counts; a `main` in an imported module is ignored. Running a file
that declares none is an error, as is building it or compiling it from an
embedding host; `candela check` compiles it, which is how a library whose entry
has nothing to run checks.

A `return` inside `main` ends the program, wherever it sits. Nothing is waiting
on a value from `main`, so `return expr;` there drops the value; use `exit(code)`
to choose the exit status.

## Returning

`return expr;` hands a value back. `return;` on its own, and reaching the end of
the body, both return `null`.

```rust
fn describe(n) {
    if n > 0 {
        return "positive";
    }
    return "not positive";
}

fn main() {
    print(describe(3), describe(-1));
}
```

The return type is inferred from the body. `-> Type` after the parameter list
declares it instead, and a body that hands back anything else is a compile
error. Since each set of argument types is specialised separately, a declared
return type has to hold for every call.

```rust
fn area(w: int, h: int) -> int {
    return w * h;
}

fn main() {
    print(area(3, 4));
}
```

There is no way to write `null` as a type, so a function that returns nothing
leaves the annotation off.

Recursion works as you would expect.

```rust
fn factorial(n) {
    if n <= 1 {
        return 1;
    }
    return n * factorial(n - 1);
}

fn main() {
    print(factorial(5));
}
```

## Anonymous functions

`fn(params) { ... }` written in expression position is a value you can store in
a variable or pass to another function.

```rust
fn main() {
    let double = fn(x) { return x * 2; };
    print(double(21));
}
```

An anonymous function does not capture the variables around it. Its body sees
only its own parameters, so pass in everything it needs.

```rust
fn scale(factor, xs) {
    let out = [];
    for x in xs {
        out.push(x * factor);
    }
    return out;
}

fn main() {
    print(scale(3, [1, 2]));
}
```

## Functions as arguments

A parameter that receives a function is called like any other function, which is
all a higher-order function needs.

```rust
fn apply_all(f, xs) {
    let out = [];
    for x in xs {
        out.push(f(x));
    }
    return out;
}

fn twice(x) {
    return x * 2;
}

fn main() {
    print(apply_all(twice, [1, 2, 3]));
    print(apply_all(fn(v) { return v + 1; }, [1, 2]));
}
```

Both forms work as arguments: the name of a declared function, and an anonymous
function written at the call site. A declared function's name is usable as a
value only in an argument position; to keep one in a variable, wrap it in an
anonymous function.

```rust
fn twice(x) {
    return x * 2;
}

fn main() {
    let f = fn(x) { return twice(x); };
    print(f(4));
}
```

The standard library's list helpers take functions this way, so `map`, `filter`,
and `reduce` are ordinary calls; see [Collections](collections.md).
