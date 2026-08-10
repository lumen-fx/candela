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

Parameters are plain names. You do not annotate them and you do not annotate the
return type: candela infers both, and it compiles a separate specialisation of
the function for each combination of argument types a call site uses. One
definition therefore serves every type it can work with.

```rust
fn same(x) {
    return x;
}

fn main() {
    print(same(1), same("a"), same(1.5));
    print(type(same(1)), type(same("a")));
}
```

Declaration order does not matter, so a function may call one declared further
down the file. Two functions cannot share a name: there is no overloading, and a
repeated name is a compile error.

## main

`main` is the entry point. Running a file calls it, and only the `main` of the
file you run counts; a `main` in an imported module is ignored. A program with
no `main` is an error.

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
