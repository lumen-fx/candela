# An import binds the module's name instead of merging its names into scope

`import "std/json";` used to merge every function, struct and enum the module
declares into the importing file's scope. It now binds the module under the
last segment of its path, so its names are reached through it:
`json::parse(s)`. `as` still picks another name, so `import "std/json" as
json;` can drop the `as`. To write some names with no prefix, list them:
`import "std/assert" { eq, ne };` brings in those items, and a type named this
way brings the functions its `impl` blocks declare. There is no form that
brings in everything.

A call to a name the file now reaches only through the module's name is a
compile error, `unknown_function`, whose help names both ways to write it. A
type written bare is `unknown_struct` or `unknown_namespace` with the same
help. A path whose last segment is not a name, such as `"./my-lib.cdl"`, needs
`as`; without it the import is `import_name_not_identifier`.

This affects you if a program imports a module without `as` and calls or names
what it declares with no prefix, or writes a generic type such as `Set<int>`
from a module it imported with `as`.

Before:

```rust
import "std/assert";
import "std/json" as json;

fn main() {
    assert_eq(json::stringify([1, 2]), "[1,2]");
}
```

After:

```rust
import "std/assert";
import "std/json";

fn main() {
    assert::eq(json::stringify([1, 2]), "[1,2]");
}
```

Or, naming the items:

```rust
import "std/assert" { eq };
import "std/json" { stringify };

fn main() {
    eq(stringify([1, 2]), "[1,2]");
}
```
