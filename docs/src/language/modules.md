# Modules

A module is a `.cdl` file. Anything declared at its top level, functions,
structs, enums, and `impl` blocks, becomes available to a file that imports it.
There is nothing to declare or register: writing the file is what makes it a
module.

## The import form

An import is the `import` keyword, a quoted path, and a semicolon, with either
`as name` or a list of items in braces before the semicolon when you want one.

```rust
import "std/json";
import "./helpers.cdl" as help;
import "std/assert" { eq };
```

The path decides where candela looks.

- A path ending in `.cdl` is a file import. It resolves next to the file doing
  the importing, so `"./helpers.cdl"` and `"./util/format.cdl"` mean what they
  look like.
- A path with no extension is a library import. Its first segment names a
  package the project depends on, when there is one by that name, and otherwise
  it resolves against the library directory shipped with the toolchain. So
  `"std/json"` loads that module wherever you run the program from, and
  `"shapes/circle"` loads a file out of the `shapes` package.

Any other extension is an error. Imports go at the top level of a file, among
the other declarations, and are conventionally written first. An import under
a [`@cfg`](conditional-compilation.md) whose condition is false is dropped, so
it may name a module that exists only on another target.

## Imports bind the module's name

An import binds the module under the last segment of its path, without the
`.cdl`: `import "std/json";` binds `json`, `import "./geometry.cdl";` binds
`geometry`, and `import "shapes";` binds `shapes`. Reach what the module
declares through that name with `::`.

```rust
import "std/json";

fn main() {
    print(json::stringify([1, 2, 3]));
}
```

The name in front of every call says where the function comes from, and two
modules can declare the same name without getting in each other's way.

`as` binds the module under a name you choose instead:

```rust
import "std/json" as j;

fn main() {
    print(j::stringify([1, 2, 3]));
}
```

A path whose last segment is not a name, such as `"./my-lib.cdl"`, needs `as`:
`import "./my-lib.cdl" as my_lib;`.

A variant of an enum the module declares takes the enum's name in between,
since the variant belongs to the enum and not to the module:
`shapes::Shape::Circle(1)`, both where a value is built and in a `match` arm.
The type itself is `shapes::Shape`. See [enums](enums.md).

A [generic](generics.md) type or function keeps its type arguments on its own
name, behind the module's: `shapes::Slot<int>` wherever a type goes,
`shapes::first<int>(xs)` at a call, and `shapes::Slot<int>::new()` for a
function its `impl` block declares.

The methods a module's `impl` blocks declare travel with their type, however
the module was imported. A value of `geometry::Point` calls `p.area()`, and an
`impl string` block in an imported module adds methods every string has:
`"hi".shout()`, with no prefix.

## Importing items by name

A list of names in braces after the path brings those items into the file's
own scope, so they are written with no prefix.

```rust
import "std/assert" { eq, is_true };

fn main() {
    eq(2 + 2, 4);
    is_true(1 < 2);
}
```

An item is a function, a struct or an enum the module declares; naming a type
brings the functions its `impl` blocks declare along with it. This form binds
no module name, and there is no form that brings in everything. Naming an item
the module does not declare is a compile error that lists the ones it does. The
same module may be imported both ways, one import under its name and one with
a list.

`as` and a list do not combine on one import; write two imports of the same
path instead.

## Name collisions

An item brought in by name that already exists in the importing file is a
compile error naming the item and both sources, whether the existing one is a
declaration in that file or came from another import's list.

```rust
import "./geometry.cdl" { area };

fn area(w, h) {
    return 0;
}
```

If `geometry.cdl` declares `area`, that program does not compile. Leave `area`
out of the list and call `geometry::area`, or rename yours. The same item
arriving twice by different routes is not a collision, so two imports of one
module that both name it are fine.

Two imports that would bind one name to two different modules are an error as
well, the way `import "./geometry.cdl";` and `import "./old/geometry.cdl";`
would both bind `geometry`. Give one of them another name with `as`.

Types are separate per module. Two modules can each declare a `Plain`, whether
struct, enum, or generic, and a program can import both under their names and
use both: each keeps its own fields or variants and its own `impl` methods. The
two are different types, so one is not accepted where the other is expected,
and a message about either puts the module in front of the name to say which
one it means.

A call to a name the file reaches only through a module's name, such as
`stringify(x)` after `import "std/json";`, is a compile error whose help names
both ways to write it: `json::stringify(x)`, or
`import "std/json" { stringify };`.

## Importing your own files

Split a program by putting declarations in a file next to it and importing the
path.

`geometry.cdl`:

```rust
fn area(width, height) {
    return width * height;
}
```

`main.cdl`:

```rust
import "./geometry.cdl";

fn main() {
    print(geometry::area(3, 4));
}
```

What a module imports stays its own. `main.cdl` above sees `area` through
`geometry`, but not the modules `geometry.cdl` imports or the items it named in
a list; `main.cdl` imports those itself when it needs them.

Only the `main` of the file you run is the entry point. A `main` in an imported
module is ignored, which lets a module keep one for its own checks.

## What a module sees

Each file resolves names in its own scope: what it declares, the items its
imports named, and the modules its imports bound. That holds inside function
bodies as much as in a signature, and it does not depend on how the file was
imported, so a module written against its own declarations behaves the same
whichever way a file imports it. Names travel one way: a module does not see
the importing file's declarations or imports, and an importing file sees what a
module declares, not what the module imports.

A module that several files import is loaded once: it is read and its
declarations registered a single time, however many files reach it and whether
they spell its path as a library import or as a path beside them. Every one of
those files still binds it in its own form, under a name or by a list of items.

## The standard library

Standard library modules are library imports: `import "std/json";`,
`import "std/math";`, and so on. They ship as candela source beside the
toolchain and compile into your program like any other module. The
[Standard library](../standard-library/overview.md) section lists what each one
provides.

The collection and enum modules (`list`, `string`, `map`, `option`, `result`)
are the prelude: every program has them with no import, so `xs.map(f)`, `s.capitalize()` and `Some(1)` work in a file with no
imports at all. Their helpers are methods in `impl` blocks that resolve on the
receiver's type. Importing one of them by name still works and reaches the same
module. A call to a name an unimported module declares is a compile error whose
help names the import, so `sqrt(x)` points at `import "std/math";` and the
call `math::sqrt(x)`.

## Packages

A package a project depends on is another place a library import reads from.
The first segment names the package, which on its own reads the package's
entry point; the rest is a path beside that entry.

```rust
import "shapes";        // the package's entry, src/main.cdl, as shapes
import "shapes/circle"; // src/circle.cdl inside the package, as circle
```

Nothing else changes: the same one import form, the same `as` and lists of
items, the same collision rules. A package is a directory that a library path
can resolve against, not a new kind of import.

Packages are listed in `candela.toml` and fetched by `candela add`. See
[projects and packages](../getting-started/packages.md).
