# random

A seedable pseudo-random generator for ints and floats.

```rust
import "std/random" as random;
```

The generator is a PCG32 instance shared by the whole program. On the first call
that draws a number it seeds itself from the clock, so a run without an explicit
`seed` differs each time. Call `seed` to make a run repeatable.

The sequence is not suitable for cryptography.

The module binds a small dynamic library, so it is one of the three std modules
that need that library present at run time. A `.cdlb` built from a program that
imports `std/random` records the binding by name and re-opens it when the
artifact runs; see [artifacts](../reference/artifacts.md).

## seed

```rust
random::seed(s)
```

- `s`: an int.
- Returns: nothing.

Restarts the generator from `s`. Two runs seeded with the same value draw the
same sequence.

## random_int

```rust
random::random_int()
```

- Returns: an int drawn from the generator's full 32-bit range, so the result can
  be negative.

## random_int_range

```rust
random::random_int_range(min, max)
```

- `min`, `max`: ints.
- Returns: an int between `min` and `max`, with both ends included.

```rust
import "std/random" as random;

fn main() {
    random::seed(42);
    print(random::random_int_range(1, 6));
}
```

## random

```rust
random::random()
```

- Returns: a float in the half-open range from 0 up to 1.

## random_range

```rust
random::random_range(min, max)
```

- `min`, `max`: floats.
- Returns: a float between `min` and `max`, including `min` and excluding `max`.
