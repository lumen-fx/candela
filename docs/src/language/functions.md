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

`return expr;` hands a value back. A function that returns no value anywhere
returns `null`, whether it leaves through a bare `return;` or by reaching the end
of its body. A function that returns a value on one path returns one on every
path: a bare `return;` in such a body, or a path that reaches the end of it, is
a compile error at the function. A path that ends in a `throw`, an `exit`, or a
loop nothing breaks out of never comes back and needs no return.

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

A declared return type is the type of every call to the function, even where
the body returns something narrower. That is how a declaration widens a value:
a body may return an `int[]` from a function declared `-> any[]`, or a
`{string: string}` from one declared `-> {any: any}`, and the caller sees the
declared type.

```rust
fn row() -> {any: any} {
    return {"name": "a"};
}

fn main() {
    let rows = [as_map(json_parse("{\"id\": 1}"))];
    rows.push(row());
    print(rows.len());
}
```

A generic function declared to return a type parameter the call leaves unbound
is the exception: `first([1, 2])` has the type the body returns, since the
declaration names no type until the call does.

A body that returns an `any` value from a function declared `int`, `float`,
`string`, `bool`, a list or a map checks the value on the way out. A value of
another type raises `bad_downcast`, which a `catch` can take, the same way
`as_int` and the other downcasts do.

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

Calls stand up to 1000000 deep. A recursion that goes further raises
`call_depth_exceeded`, naming the call it stopped at, so a recursion that never
reaches its base case ends with an error a `catch` can take rather than eating
memory until the machine stops it. See [errors](../reference/errors.md).

## Anonymous functions

`fn(params) { ... }` written in expression position is a value you can store in
a variable or pass to another function.

```rust
fn main() {
    let double = fn(x) { return x * 2; };
    print(double(21));
}
```

The parameter list may be empty. `fn() { ... }` is the shape of a callback that
takes nothing, and it is called the same way.

```rust
fn main() {
    let answer = fn() { return 42; };
    print(answer());
}
```

An anonymous function captures: its body reads its own parameters and the
variables of the function it is written in, and an assignment inside it writes
the same variable that function reads. An anonymous function that captures is a
closure. One written in a loop body captures that turn of the loop.

```rust
fn main() {
    let total = 0;
    let add = fn(x) { total = total + x; };
    add(3);
    add(4);
    print(total);
}
```

A captured variable lives as long as the closures that hold it, so a closure
that outlives the call it was made in goes on reading and writing what it
captured. Every closure written in one scope shares that variable, and each
call of the surrounding function captures a fresh one.

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

A closure written inside such a function calls the function it was handed, and
goes on calling it after the enclosing call has returned: the parameter is one
of the variables the closure captured.

```rust
fn compose(f, g) {
    return fn(x) { return f(g(x)); };
}

fn main() {
    let inc = fn(x) { return x + 1; };
    let dbl = fn(x) { return x * 2; };
    print(compose(inc, dbl)(5));
}
```

A function that returns an anonymous function, or a list that holds one, is
called where it stands; there is no need to bind it with `let` first.

```rust
fn doubler() {
    return fn(x) { return x * 2; };
}

fn main() {
    print(doubler()(21));
    let fs = [fn(x) { return x + 1; }];
    print(fs[0](41));
}
```

A struct field holds one the same way, and calling the field is the same dot a
method call uses. Declare the field `fn(params) -> result`; see
[Function types](#function-types) below.

```rust
struct Button {
    label: string,
    on_press: fn(int) -> int,
}

fn main() {
    let b = Button { label: "ok", on_press: fn(x) { return x * 2; } };
    print(b.on_press(21));
}
```

A method of that name wins: the field is reached only when the receiver's type
has no such method, so adding a method never changes which call an existing
program makes. See [Methods](methods.md).

Both forms work as arguments: the name of a declared function, and an anonymous
function written at the call site. A declared function's name is the function
itself wherever a value goes, so a `let`, a return, a list or map element and a
struct field each take it by name, and the call goes to what the name handed
over. The value carries the signature the function was declared with, so the
call is checked against it.

```rust
fn twice(x: int) -> int {
    return x * 2;
}

fn main() {
    let f = twice;
    print(f(4));

    let fs = [twice, fn(x) { return x + 1; }];
    print(fs[1](4));
}
```

The standard library's list helpers take functions this way, so `map`, `filter`,
and `reduce` are ordinary calls; see [Collections](collections.md).

## Function types

`fn(A, B) -> R` is the type of a position that holds a function taking an `A`
and a `B` and returning an `R`. Leave the arrow off for one that returns
nothing: `fn()` and `fn(int)`. Write it wherever a type goes: on a parameter, on
a struct field, as a type argument, or as the element type of a list.

```rust
fn apply(f: fn(int) -> int, x: int) -> int {
    return f(x);
}

fn double(n: int) -> int {
    return n * 2;
}

fn main() {
    print(apply(double, 21));
    print(apply(fn(y) { return y + 1; }, 41));
}
```

Any function of that shape fits: an anonymous one written at the call site, one
bound to a variable, or the name of a declared function. The declaration is what
the body is compiled against, so a function whose body does not work at those
parameter types, or hands back something else, is reported where it was written
rather than where it is called.

A list, a map or a variable that holds more than one function needs no
annotation. Each value carries which function it is, so the call goes to the one
the index, the key or the last assignment put there.

```rust
fn main() {
    let fs = [fn(x) { return x + 1; }, fn(x) { return x * 10; }];
    print(fs[0](4), fs[1](4));

    let ops = {"inc": fn(x) { return x + 1; }, "ten": fn(x) { return x * 10; }};
    print(ops.get("ten")(4));
}
```

A closure calls itself by the name it is bound to, and two closures call each
other through the variables they captured.

```rust
fn main() {
    let fact = fn(n) {
        if n <= 1 {
            return 1;
        }
        return n * fact(n - 1);
    };
    print(fact(5));
}
```

Where the compiler can name the function a call reaches, it emits a direct
call rather than dispatching on the value. A closure held in a variable, a list
holding one function, and a higher-order call are all included.

A function value prints as `<fn>`, on its own and inside a list, a map or a
struct field, since a function carries no signature at run time to tell one from
another.

A function held in a value carries one body, so it is compiled for one set of
argument types. Calling the same value with an `int` in one place and a
`string` in another is a compile error naming the types it was compiled for;
write a function for each.
