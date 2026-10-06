# Collections

Candela has two built-in collections, lists and maps, plus a set built on top of
maps in the standard library. All three are passed by reference: handing one to
a function lets that function change it.

## Lists

A list is written in square brackets and holds elements of a single type.

```rust
fn main() {
    let numbers = [1, 2, 3];
    let words = ["alpha", "beta"];
    let empty = [];
    print(numbers, words, empty.len());
}
```

Mixing types in one list is a compile error: `[1, "a"]` does not compile.

### The empty list

`[]` names no element type, so the declaration that holds it says what it will
hold: `let xs: int[] = [];` for a local, `cells: Cell[]` for a parameter,
`-> Cell[]` for what a function returns. A list that names an element type is
still checked against the declaration, so `width([1, 2])` below does not
compile. A local declared without an annotation takes its element type from the
first `push` instead.

```rust
enum Cell { Num(int), Text(string) }

fn width(cells: Cell[]) -> int {
    let w = 0;
    for c in cells {
        w = w + match c {
            Cell::Num(_) => 1,
            Cell::Text(t) => t.len(),
        };
    }
    return w;
}

fn main() {
    let row: Cell[] = [];
    print(width(row));
    row.push(Cell::Text("ab"));
    print(width(row));
}
```

### Indexing and slicing

Index from zero with `xs[i]`. A slice `xs[start..end]` returns a new list from
`start` up to but not including `end`. `xs[..end]` starts at the beginning and
`xs[start..]` runs to the end. An index past the end raises at runtime.

```rust
fn main() {
    let xs = [10, 20, 30, 40];
    print(xs[0], xs[3]);
    print(xs[1..3], xs[..2], xs[2..]);
    xs[0] = 99;
    print(xs);
}
```

Strings index and slice the same way, returning strings.

### Growing and reordering

```rust
fn main() {
    let xs = [3, 1, 2];
    xs.push(4);
    xs.sort();
    print(xs);
    xs.reverse();
    print(xs);
    xs.remove(0);
    print(xs);
}
```

`push`, `sort`, `reverse`, and `remove` change the list in place. `+`
concatenates two lists into a new one.

The rule holds for every collection method: a method that changes its receiver
returns nothing (`push`, `remove`, `sort` and `reverse` on a list, `remove` on
a map, `add` and `remove` on a set), and so does `m[k] = v`. Every other method leaves the
receiver as it was and returns a new value, so `xs.sort_by(f)` and
`"abc".reverse()` hand back a sorted list and a reversed string. `each`, which
exists to call a function for its effect, is the one method that returns nothing
without changing the list.

```rust
fn main() {
    print([1, 2] + [3]);
}
```

### Inspecting

```rust
fn main() {
    let xs = [10, 20, 30];
    print(xs.len(), xs.contains(20), xs.find(30));
    print(["a", "b"].join(", "));
    print([1, 0, 2, 0, 3].partition(0));
}
```

`find` returns the index of a value, or `-1` when it is absent. `join`
concatenates a list of strings, with an optional separator. `partition` splits a
list on a separator element.

### Iterating

```rust
fn main() {
    let xs = [1, 2, 3];
    for x in xs {
        print(x);
    }
    for i in 0..xs.len() {
        print(xs[i]);
    }
}
```

### Higher-order operations

Lists carry the standard library's `list` helpers as methods, with no import
needed.

```rust
fn double(x) {
    return x * 2;
}

fn main() {
    let xs = [1, 2, 3, 4];
    print(xs.map(double));
    print(xs.filter(fn(x) => x % 2 == 0));
    print(xs.reduce(0, fn(a, b) => a + b));
    print(xs.sum(), xs.min(), xs.max(), xs.first(), xs.last());
    print(xs.max().unwrap_or(0));
    print(xs.take(2), xs.drop(2), xs.unique(), xs.chunk(2));
    print(xs.any(fn(x) => x > 3), xs.all(fn(x) => x > 0));
}
```

A lookup that may find nothing answers an `Option`: `first`, `last`, `min` and
`max` give `Some` of the element, or `None` for an empty list. See
[option](../standard-library/option.md).

The functions you pass read the variables around them, so a predicate can test
against what the scope holds; see [Functions](functions.md).

## Maps

A map is written in braces as `key: value` pairs. Keys share one type and values
share one type. `{}` is the empty map.

```rust
fn main() {
    let ages = {"ada": 36, "alan": 41};
    let by_number = {1: "one", 2: "two"};
    let empty = {};
    print(ages.len(), by_number[1], empty.len());
}
```

Repeating a key in a literal is a compile error.

### The empty map

`{}` names neither a key type nor a value type, so the declaration that holds it
says what it will hold: `let cells: {string: Cell} = {};` for a local,
`cells: {string: Cell}` for a parameter, `-> {string: Cell}` for a return type.
A map that names its types is still checked against the declaration. A local
declared without an annotation takes its key and value types from the first
entry written into it instead.

```rust
enum Cell { Num(int), Text(string) }

fn width(cells: {string: Cell}) -> int {
    let w = 0;
    for name in cells {
        w = w + match cells[name] {
            Cell::Num(_) => 1,
            Cell::Text(t) => t.len(),
        };
    }
    return w;
}

fn main() {
    let sheet: {string: Cell} = {};
    print(width(sheet));
    sheet["a"] = Cell::Text("ab");
    print(width(sheet));
}
```

### Reading and writing

```rust
fn main() {
    let scores = {"a": 1};
    scores["b"] = 2;
    scores["a"] = 10;
    scores["a"] += 5;
    print(scores["a"], scores.len());
    print(scores.get("z"), scores.get("z").unwrap_or(0));
    print(scores.contains("b"), scores.keys(), scores.values());
    scores.remove("b");
    print(scores.len());
}
```

Brackets read and write an entry. `m[k]` reads the value under a key and raises
`unknown_map_key` when the key is absent; `m[k] = v` adds the entry or replaces
the value of an existing one. `m.get(k)` is the lookup that may find nothing: it
answers `Some` of the value or `None`, so `m.get(k).unwrap_or(fallback)` reads
with a fallback. `remove` takes the entry under a key back out, and a key the
map does not hold leaves it as it was.

### Iterating

Iterating a map walks its keys in the order they went in, and a literal's keys
go in as written. `keys`, `values`, and printing read the same order. Writing
a key that is already there replaces the value and leaves the entry where it
is; removing a key and writing it again puts it at the end. Two maps holding
the same entries are equal whatever order they were built in.

```rust
fn main() {
    let scores = {"a": 1, "b": 2};
    let total = 0;
    for key in scores {
        total += scores[key];
    }
    print(total);
}
```

## Sets

A set holds each value at most once. It comes from the `set` module as
`Set<T>`, a struct built out of a map. `Set<int>::new()` makes an empty one,
naming the member type.

```rust
import "std/set";

fn main() {
    let s = Set<int>::new();
    s.add(1);
    s.add(2);
    s.add(2);
    print(s.len(), s.contains(2), s.members());
}
```

`|`, `&`, `-`, and `^^` are union, intersection, difference, and symmetric
difference; each combines two sets into a new one, and each has a named method
too.

```rust
import "std/set";

fn main() {
    let a = Set<int>::new();
    a.add(1);
    a.add(2);
    let b = Set<int>::new();
    b.add(2);
    b.add(3);
    print((a | b).members());
    print(a.intersection(b).members());
}
```

A set is a struct, so it is not iterable itself; `members` gives you a list to
iterate, in the order the members were added. `a == b` compares the members and
ignores that order. The module is covered in full in
[set](../standard-library/set.md).
