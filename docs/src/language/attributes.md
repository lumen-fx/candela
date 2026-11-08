# Attributes

An attribute is a note in front of a function or a struct that the program
embedding candela reads. It is written `@` and its name, behind the module that
declares it, with named arguments in parentheses:

```rust
import "ecs";

@ecs::component
struct Position { x: float, y: float }

@ecs::system(writes: [Position])
fn movement(rows: Mover[], time: Time) {
    for m in rows {
        m.pos.x += m.vel.x * time.dt;
    }
}
```

candela gives an attribute no meaning. The host that declares it does: an
engine might run every `@ecs::system` function once a frame, and copy back
the components its `writes:` names. Read that host's documentation to learn
which attributes exist and what they do. The one attribute candela reads itself
is [`@cfg`](conditional-compilation.md).

## What the compiler checks

The compiler checks every attribute against its declaration before anything
runs, under `candela check`, `candela build` and the editor alike:

- The attribute exists in the module it is named behind.
- It goes on the kind of item its declaration names: a function or a struct.
- Each argument names one of its parameters, at most once, and every parameter
  without a default is given.
- Each value has its parameter's type. A type name resolves, so a misspelt
  `writes: [Positon]` is an unknown type; a function name names a declared
  function, so `after: [movment]` is an error too.

Whether the arguments make sense together (that a system writes what it reads,
say) is the host's to check, when it loads the program.

## Writing arguments

Every argument names its parameter, `name: value`, in any order. A parameter
the declaration gives a default may be left out, and so may one it marks
optional; an attribute whose parameters can all be left out may be written with
no parentheses at all.

| Parameter type | A value is written as |
| --- | --- |
| `int`, `float`, `bool`, `string` | a literal: `3`, `-1`, `0.5`, `true`, `"move"` |
| `type` | a type: `Position`, `int[]`, `(int, string)`, `shapes::Circle` |
| `fn` | the name of a function: `movement`, `other::step` |
| an enum | one of its variants without a payload: `ecs::Schedule::Startup` |
| `enum` | a variant without a payload of any enum the script declares: `Phase::Over` |
| a list of one of these, `T[]` | the values in brackets: `[Position, Velocity]` |

A literal of one numeric type does not stand for the other: a `float`
parameter takes `1.0`, not `1`.

## Where attributes go

An attribute goes in front of a top-level `fn`, `struct` or `enum` declaration,
and several may stand in a row, `@cfg` among them. A function, struct or enum
with type parameters cannot carry one, since it stands for one per set of type
arguments, and neither can a method, an `import` or a statement.

When `@cfg` drops a declaration, the attributes in front of it are dropped with
it and not checked.

## Declaring an attribute

A `host` block declares the attributes a script may write, next to the
functions and structs it declares:

```rust
host "ecs" {
    attribute component on struct;
    attribute system(writes: type[] = [], after: fn[] = []) on fn;
}
```

`on fn`, `on struct` or `on enum` says what the attribute goes on. Each
parameter has a name, a type from the table above, and optionally `= value` as
its default. An enum a parameter takes is declared like any other; a parameter
typed `enum` takes a variant of whichever enum the script names.

A `?` after the type makes a parameter optional with no default: a script may
leave it out, and the host then finds no argument for it rather than a default
value.

```rust
host "game" {
    attribute state on enum;
    attribute system(in_state: enum[] = [], on_enter: enum?, run_if: fn[] = []) on fn;
}
```

A script names the attribute behind the module the block is written in: a
package `ecs` that holds the block above is imported with `import "ecs";` and
its attributes written `@ecs::system`. The file that holds the block writes them
bare, `@system`, and an import that lists the attribute by name
(`import "ecs" { system };`) brings it in bare as well.

How the host reads what a script wrote is in
[embedding](../integration/embedding.md#reading-attributes).
