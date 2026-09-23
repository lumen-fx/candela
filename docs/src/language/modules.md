# Modules

A module is a `.cdl` file. Anything declared at its top level, functions,
structs, enums, and `impl` blocks, becomes available to a file that imports it.
There is nothing to declare or register: writing the file is what makes it a
module.

## The import form

An import is the `import` keyword, a quoted path, and a semicolon.

```rust
import "std/string";
import "./helpers.cdl";
```

The path decides where candela looks.

- A path ending in `.cdl` is a file import. It resolves next to the file doing
  the importing, so `"./helpers.cdl"` and `"./util/format.cdl"` mean what they
  look like.
- A path with no extension is a library import. Its first segment names a
  package the project depends on, when there is one by that name, and otherwise
  it resolves against the library directory shipped with the toolchain. So
  `"std/string"` loads that module wherever you run the program from, and
  `"shapes/circle"` loads a file out of the `shapes` package.

Any other extension is an error. Imports go at the top level of a file, among
the other declarations, and are conventionally written first. An import under
a [`@cfg`](conditional-compilation.md) whose condition is false is dropped, so
it may name a module that exists only on another target.

## Bare imports

A bare import merges the module's symbols into the importing file's own scope.
Its functions are then called with no prefix, as if you had written them
yourself.

```rust
import "std/assert";

fn main() {
    assert_eq(2 + 2, 4);
}
```

This suits small, obviously-named helpers. It also carries a risk: two modules
that export the same name collide.

## Namespaced imports

`as` binds the module to a name instead, and its symbols are reached through
that name with `::`.

```rust
import "std/json" as json;

fn main() {
    print(json::stringify([1, 2, 3]));
}
```

This is the form to prefer when a module has common names such as `map`, `get`,
or `len`, and when a reader benefits from seeing where a function came from. The
alias is yours to choose; it need not match the file name.

A variant of an enum the module declares takes the enum's name in between,
since the variant belongs to the enum and not to the module:
`shapes::Shape::Circle(1)`, both where a value is built and in a `match` arm.
The type itself is `shapes::Shape`. See [enums](enums.md).

A [generic](generics.md) type or function keeps its type arguments on its own
name, behind the alias: `shapes::Slot<int>` wherever a type goes and
`shapes::first<int>(xs)` at a call.

## Name collisions

Merging a symbol that already exists in the importing file is a compile error
naming the symbol and both sources, whether the existing one is a declaration in
that file or came from an earlier bare import.

```rust
import "./geometry.cdl";

fn area(w, h) {
    return 0;
}
```

If `geometry.cdl` also declares `area`, that program does not compile. Import
one of the two with `as` to keep them apart, or rename yours. The same symbol
arriving twice by different routes is not a collision, so two modules that both
depend on a third are fine.

Types are separate per module. Two modules can each declare a `Plain`, whether
struct, enum, or generic, and a program can import both under aliases and use
both: each keeps its own fields or variants and its own `impl` methods. The two
are different types, so one is not accepted where the other is expected, and a
message about either puts the module in front of the name to say which one it
means.

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
import "./geometry.cdl" as geometry;

fn main() {
    print(geometry::area(3, 4));
}
```

A module's own bare imports come along with it: bare-importing a module gives
you the names it merged as well as the ones it declares.

Only the `main` of the file you run is the entry point. A `main` in an imported
module is ignored, which lets a module keep one for its own checks.

## What a module sees

Each file resolves names in its own scope: what it declares, what its bare
imports merged in, and the modules it bound with `as`. That holds inside
function bodies as much as in a signature, and it does not depend on how the
file was imported, so a module written against its own declarations behaves the
same bare-imported and behind an alias. Names travel one way: a module does not
see the importing file's declarations or imports, so it needs an import of its
own for anything it uses.

A module that several files import is loaded once: it is read and its
declarations registered a single time, however many files reach it and whether
they spell its path as a library import or as a path beside them. Every one of
those files still binds it in its own form, merged into scope by a bare import
and behind an alias by an `as` one.

## The standard library

Standard library modules are library imports: `import "std/string";`,
`import "std/map" as map;`, and so on. They ship as candela source beside the
toolchain and compile into your program like any other module. The
[Standard library](../standard-library/overview.md) section lists what each one
provides.

The collection, conversion, and enum modules (`list`, `string`, `map`,
`convert`, `option`, `result`)
define their helpers as methods in `impl` blocks, so importing them is enough;
the methods then resolve on the receiver's type. The `list` module goes one step
further and loads automatically, so `xs.map(f)` works in a file with no imports
at all.

## Packages

A package a project depends on is another place a library import reads from.
The first segment names the package, which on its own reads the package's
entry point; the rest is a path beside that entry.

```rust
import "shapes" as shapes;        // the package's entry, src/main.cdl
import "shapes/circle" as circle; // src/circle.cdl inside the package
```

Nothing else changes: the same one import form, the same `as`, the same
collision rules on a bare import. A package is a directory that a library path
can resolve against, not a new kind of import.

Packages are listed in `candela.toml` and fetched by `candela add`. See
[projects and packages](../getting-started/packages.md).
