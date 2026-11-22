# Attributes

An attribute is a note in front of a declaration that the program embedding
candela reads. It is written `@` and its name, behind the module that declares
it, with its arguments in parentheses:

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
- It goes on a kind of declaration its declaration lists: a function, a
  struct, a field, and so on.
- Each parameter gets at most one argument, and every parameter without a
  default is given.
- Each value has its parameter's type. A type name resolves, so a misspelt
  `writes: [Positon]` is an unknown type; a function name names a declared
  function, so `after: [movment]` is an error too.

Whether the arguments make sense together (that a system writes what it reads,
say) is the host's to check, when it loads the program.

## Writing arguments

Arguments are written the way a call writes them: in order first, then by name,
`name: value`, in any order. `@range(1, max: 10)` gives the first parameter 1
and `max` 10. A parameter the declaration gives a default may be left out, and
so may one it marks optional; an attribute whose parameters can all be left
out may be written with no parentheses at all.

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

An attribute goes in front of any declaration: a function, a method, a
parameter, a constant, a struct, a field, an enum, a variant or an `impl`
block. Several may stand in a row, `@cfg` among them.

```rust
import "ui";

@ui::panel(title: "Player")
struct Player {
    @ui::slider(0, 100) health: int,
    @ui::hidden id: int,
}

@ui::actions
impl Player {
    @ui::button("Heal")
    fn heal(self, @ui::slider(1, 10) amount: int) {
        self.health += amount;
    }
}

enum Team {
    @ui::label("Red team") Red,
    Blue,
}

@ui::setting
const MAX_PLAYERS: int = 8;
```

A declaration with type parameters cannot carry one, and neither can its
parts or the methods of an `impl<T>` block, since it stands for one
declaration per set of type arguments. An `import`, a statement, a closure's
parameter and the fields of a struct declared inside a function take only
`@cfg`.

When `@cfg` drops a declaration, a field, a variant or a parameter, the
attributes in front of it are dropped with it and not checked.

## Declaring an attribute

A `host` block declares the attributes a script may write, next to the
functions and structs it declares:

```rust
host "ecs" {
    attribute component on struct;
    attribute system(writes: type[] = [], after: fn[] = []) on fn;
}
```

The `on` clause lists the kinds of declaration the attribute goes on, one or
several: `fn`, `method`, `param`, `const`, `struct`, `field`, `enum`,
`variant` and `impl`. `attribute doc(text: string) on fn, method;` goes on
functions and methods both; a `fn` inside an `impl` block is a `method`, not a
`fn`. Each parameter has a name, a type from the table above, and optionally `= value` as
its default. An enum a parameter takes is declared like any other; a parameter
typed `enum` takes a variant of whichever enum the script names.

A `?` after the type makes a parameter optional with no default, the way `T?`
says a value may be absent anywhere else: a script may leave it out, and the
host then finds no argument for it rather than a default value.

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
