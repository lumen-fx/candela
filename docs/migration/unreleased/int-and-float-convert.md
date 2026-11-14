# int and float convert to each other where one is expected

An `int` now goes wherever a `float` is expected, and a `float` wherever an
`int` is, truncated toward zero with an `implicit_float_to_int` warning. Mixed
arithmetic and comparison work in floats. Programs that compiled before mostly
behave the same; these change:

`1 == 1.0` was `false`, because values of different types were never equal. It
is now `true`, and `1 < 1.5` compiles. Values typed `any` still compare as
they are.

A variable declared without a type no longer changes between `int` and
`float` when one is assigned to it. It keeps the number type of its first
value, and the assigned number converts:

Before:

```rust
let speed = 0;
speed = 2.5;
print(speed);   // 2.5
```

After:

```rust
let speed = 0;
speed = 2.5;    // warning: implicit_float_to_int
print(speed);   // 2
```

Start such a variable with the type it should hold, `let speed = 0.0;`, or
declare it, `let speed: float = 0;`. An assignment still changes the type of
such a variable to anything other than the other number type.

`2.0 / 0` and `2.0 % 0` were type errors; they now divide as floats, giving an
infinity and `NaN`.

Code that converted by hand keeps working: `float(n)` and `int(x)` are
unchanged, and the warning goes away where `int(x)` is written.
