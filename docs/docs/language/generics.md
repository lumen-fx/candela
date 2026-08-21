# Generics

A type parameter lets one declaration work for several types without giving up
the type check. Write it in angle brackets after the name, then use it wherever
a type goes.

```rust
struct Cell<T> {
    value: T,
}

impl Cell<T> {
    fn get(self) -> T {
        return self.value;
    }
}

fn main() {
    let c = Cell<int>{ value: 3 };
    print(c.get());
}
```

`Cell<int>` and `Cell<string>` are separate types. A value of one is not
accepted where the other is expected, and each gets its own copy of the methods,
so a method body sees the concrete type it was compiled for.

## Generic functions

A function declares its type parameters after its name and may use them in the
parameter annotations and in the return annotation.

```rust
fn first<T>(items: T[]) -> T {
    return items[0];
}

fn main() {
    let nums = [10, 20, 30];
    print(first(nums));
    print(first<int>(nums));
}
```

Both calls work. Leaving the type argument off falls back to the inference every
un-annotated parameter gets; naming it pins the parameters written in terms of
it, so `first<int>` rejects a `float[]`.

Name the type argument when the type does not appear in the arguments at all:

```rust
struct Signal<T> {
    name: string,
}

fn signal<T>(name: string) {
    return Signal<T>{ name: name };
}
```

`Signal<T>` never stores a `T`. An unused type parameter is legal, and here it
is the whole point: `signal<int>("count")` and `signal<float>("ratio")` take the
same argument and hand back different types.

## Generic structs

A struct names its type parameters after the struct name and uses them as field
types.

```rust
struct Pair<K, V> {
    key: K,
    value: V,
}

fn main() {
    let p = Pair<string, int>{ key: "width", value: 40 };
    print(p.key, p.value);
}
```

Written without type arguments, a literal takes each parameter from the value in
the field declared with it, so `Pair{ key: "width", value: 40 }` is the same
type as the one above. A parameter that no field pins is `any`.

Type arguments nest, and a generic type is an ordinary type anywhere a type
goes: a field, a parameter, a return annotation.

```rust
struct Cell<T> {
    value: T,
}

struct Row {
    cells: Cell<int>[],
}

fn deepen(c: Cell<int>) -> Cell<Cell<int>> {
    return Cell<Cell<int>>{ value: c };
}
```

## Generic enums

An enum declares type parameters the same way, and its variants carry them as
payload types.

```rust
enum Slot<T> {
    Filled(T),
    Empty,
}

fn unwrap(s: Slot<int>) -> int {
    match s {
        Filled(x) => { return x; }
        _ => { return 0; }
    }
}

fn main() {
    print(unwrap(Slot<int>::Filled(9)));
    print(unwrap(Slot<int>::Empty));
}
```

Name the instantiation in front of the variant (`Slot<int>::Filled(9)`) to say
which one you mean. A bare `Filled(9)` resolves by variant name, which is enough
when only one instantiation of the enum exists in the program.

## impl blocks

An `impl` block on a generic type names type arguments in its header, and there
are two kinds.

A block that introduces a parameter applies to every instantiation and binds the
parameter for its methods:

```rust
impl Cell<T> {
    fn get(self) -> T {
        return self.value;
    }
}
```

A block that names concrete types applies to that instantiation alone, which is
how one generic type gets different behaviour per type argument:

```rust
impl Signal<int> {
    fn get(self) -> int {
        return app::signal_get_int(self.name);
    }
}

impl Signal<float> {
    fn get(self) -> float {
        return app::signal_get_float(self.name);
    }
}
```

Every instantiated type gets each method once. A generic block and a concrete
block that both define `get` for `Signal<int>` is the same error as defining a
function twice. An `impl` on a generic type has to name its type arguments:
`impl Cell` alone is rejected.

## Type parameters on a method

A method declares its own type parameters after its name, on top of whatever the
`impl` header binds for the receiver. They behave like a function's: leave them
off and each is inferred from the arguments, or name them at the call.

```rust
struct Store<T> {
    seed: T,
}

impl Store<T> {
    fn tagged<U>(self, extra: U) -> U {
        return extra;
    }
}

fn main() {
    let s = Store<int>{ seed: 1 };
    print(s.tagged("hi"));
    print(s.tagged<string>("hi"));
}
```

Name the argument when nothing in the call pins it, which is what picks the
instantiation the body builds:

```rust
struct Kind<T> {
    n: int,
}

impl Kind<int> {
    fn name(self) -> string {
        return "int";
    }
}

impl Kind<string> {
    fn name(self) -> string {
        return "string";
    }
}

impl Store<T> {
    fn name_of<U>(self) -> string {
        return Kind<U>{ n: 0 }.name();
    }
}

fn main() {
    let s = Store<int>{ seed: 1 };
    print(s.name_of<int>());
    print(s.name_of<string>());
}
```

## Leaving type arguments off

A missing type argument is never an error. A call falls back to inference, a
struct literal takes its arguments from its field values, and a generic type
named in a type position without arguments is the dynamic `any` slot:

```rust
fn describe(c: Cell) {
    return "some cell";
}
```

Reach for the arguments when you want the check; leave them off while the shape
is still moving.

## When `<` is a comparison

Type arguments have no separate spelling, so `<` after a name is read as a type
argument list only when the whole list parses as types and closes with `>`
immediately followed by `(`, `{` or `::`. Everything else stays the comparison
it has always been, including `a < b`, `a < b && c > d` and `a < b > c`.

After a `.` the rule is the same, narrowed to the one form a method call can
take: `p.get<int>()` is a call with a type argument, and only `(` closes the
list.

The one shape that reads both ways is a comparison whose right-hand side is
parenthesised or braced, as in `f(a < b, c > (d))`. That parses as a call to
`a` with type arguments, and `p.x < a > (b)` likewise as a call to a method `x`.
Parenthesise the comparisons to keep them: `f((a < b), (c > (d)))`.

## Limits

Type parameters have no bounds: a parameter accepts any type, and a body that
does not work for the type it is given is reported against the instantiation
that produced it, not against the declaration. Choosing an `impl` block by an
exact type argument is the only form of specialisation. A generic type is named
by its own name, without a module path in front of the type arguments.

Each distinct type argument produces its own instantiation, so a type whose
fields name a deeper instantiation of itself (`struct L<T> { next: L<L<T>> }`)
has no end and is rejected.

Only methods that declare type parameters take type arguments. The built-in
methods on strings, arrays, maps and numbers are not generic and reject them,
and the standard library's collection methods (`map`, `filter`, `reduce`, and
the rest) declare none, so a type argument on one is reported the same as on
any plain function.
