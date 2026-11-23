# A bare variant name reaches only the enums its own file can name

A variant written on its own, `Done(x)`, used to resolve against every enum in
the program, so the first module to declare a `Done` won inside every other
module, and a file could name the variants of an enum it never imported. Now a
bare variant reaches the enums the file declares, the enums it imports by name,
and the prelude's `Option` and `Result`.

This affects you if a file writes a bare variant of an enum declared in a
module it imports under a namespace. The error names the path to write.

Before:

```rust
import "./events.cdl";
fn main() { let e = Done("x"); }
```

After, through the module and the enum:

```rust
import "./events.cdl";
fn main() { let e = events::Event::Done("x"); }
```

or import the enum by name:

```rust
import "./events.cdl" { Event };
fn main() { let e = Done("x"); }
```
