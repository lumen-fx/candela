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

### Indexing and slicing

Index from zero with `xs[i]`. A slice `xs[start..end]` returns a new list from
`start` up to but not including `end`, and `xs[..end]` starts at the beginning.
An index past the end raises at runtime.

```rust
fn main() {
    let xs = [10, 20, 30, 40];
    print(xs[0], xs[3]);
    print(xs[1..3], xs[..2]);
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
    print(xs.filter(fn(x) { return x % 2 == 0; }));
    print(xs.reduce(0, fn(a, b) { return a + b; }));
    print(xs.sum(), xs.min(), xs.max(), xs.first(), xs.last());
    print(xs.take(2), xs.drop(2), xs.unique(), xs.chunk(2));
    print(xs.any(fn(x) { return x > 3; }), xs.all(fn(x) { return x > 0; }));
}
```

The same helpers are callable as free functions after importing the module,
which is the form to use when the receiver is not a list literal or variable:

```rust
import "std/list" as list;

fn main() {
    print(list::map([1, 2], fn(x) { return x + 1; }));
}
```

The functions you pass do not capture surrounding variables; see
[Functions](functions.md).

## Maps

A map is written in braces as `key: value` pairs. Keys share one type and values
share one type. `{}` is the empty map.

```rust
fn main() {
    let ages = {"ada": 36, "alan": 41};
    let by_number = {1: "one", 2: "two"};
    let empty = {};
    print(ages.len(), by_number.get(1), empty.len());
}
```

Repeating a key in a literal is a compile error.

### Reading and writing

```rust
fn main() {
    let scores = {"a": 1};
    scores.insert("b", 2);
    scores.insert("a", 10);
    print(scores.get("a"), scores.len());
    print(scores.contains("b"), scores.keys(), scores.values());
}
```

`insert` adds a pair or replaces the value of an existing key. `get` raises when
the key is absent, so test with `contains` first, or use `map::get_or` from the
standard library to supply a fallback.

### Iterating

Iterating a map walks its keys.

```rust
fn main() {
    let scores = {"a": 1, "b": 2};
    let total = 0;
    for key in scores {
        total += scores.get(key);
    }
    print(total);
}
```

## Sets

A set holds each value at most once. It comes from the `set` module, which
builds it out of a map.

```rust
import "std/set" as set;

fn main() {
    let s = set::new();
    set::add(s, 1);
    set::add(s, 2);
    set::add(s, 2);
    print(set::len(s), set::contains(s, 2), set::members(s));
}
```

`union`, `intersection`, and `difference` combine two sets into a new one.

```rust
import "std/set" as set;

fn main() {
    let a = set::new();
    set::add(a, 1);
    set::add(a, 2);
    let b = set::new();
    set::add(b, 2);
    set::add(b, 3);
    print(set::members(set::union(a, b)));
    print(set::members(set::intersection(a, b)));
}
```

Because a set is a map underneath, `members` gives you a list to iterate.
