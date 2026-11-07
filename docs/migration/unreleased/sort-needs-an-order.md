# Sorting a list whose elements have no order is an error

`xs.sort()` used to compile for a list of any element type and leave a list of
structs, enums or bools in the order it was in. A list of structs or enums now
sorts by the `<` its type defines as an operator method, and a list whose
element type has no order is the compile error `sort_without_order`. An `any[]`
list is checked when it sorts: one whose elements are not all ints, all floats
or all strings raises `sort_without_order`, where it used to come back in an
order nobody chose.

This affects you if a program calls `sort` on a list of structs, enums or bools,
or on an `any[]` holding values of more than one type.

Before:

```rust
struct Task { priority: int }

fn main() {
    let tasks = [Task { priority: 2 }, Task { priority: 1 }];
    tasks.sort(); // did nothing
}
```

After, giving the type an order:

```rust
struct Task { priority: int }

impl Task {
    fn <(self, other: Task) -> bool => self.priority < other.priority;
}

fn main() {
    let tasks = [Task { priority: 2 }, Task { priority: 1 }];
    tasks.sort();
}
```

Or keep the order out of the type and sort with a function:

```rust
let sorted = tasks.sort_by(fn(a, b) => a.priority < b.priority);
```
