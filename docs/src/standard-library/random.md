<!--
This file is derived from keel (https://github.com/horacehoff/keel),
Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
It has been modified by the candela authors. See the NOTICE file.
-->
# random

A seedable pseudo-random generator for ints and floats.

```rust
import "std/random";
```

The generator is a PCG32 instance shared by the whole program. On the first call
that draws a number it seeds itself from the clock, so a run without an explicit
`seed` differs each time. Call `seed` to make a run repeatable.

The sequence is not suitable for cryptography.

In the WebAssembly build the generator is built in, the same PCG32, so a seeded
run draws the same sequence in a browser as on a desktop. An unseeded one seeds
itself from the browser's `crypto.getRandomValues`.

The module binds a small dynamic library, so it is one of the four std modules
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

## int

```rust
random::int()
```

- Returns: an int drawn from the generator's full range, so the result can be
  negative.

## int_range

```rust
random::int_range(min, max)
```

- `min`, `max`: ints.
- Returns: an int between `min` and `max`, with both ends included.
- Raises: `random_min_above_max` when `min` is greater than `max`.

```rust
import "std/random";

fn main() {
    random::seed(42);
    print(random::int_range(1, 6));
}
```

## float

```rust
random::float()
```

- Returns: a float in the half-open range from 0 up to 1.

## float_range

```rust
random::float_range(min, max)
```

- `min`, `max`: floats.
- Returns: a float between `min` and `max`, including `min` and excluding `max`.

## uuid

```rust
random::uuid()
```

- Returns: a random version 4 UUID, as 36 lowercase characters in the form
  `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`. It comes from this module's
  generator, so after `seed` it repeats; it is for ids, not for secrets.

## choice

```rust
random::choice(xs)
```

- Returns: one element of the list `xs`, each equally likely, as a `T?`;
  `None` when the list is empty.

## shuffle

```rust
random::shuffle(xs)
```

Puts the elements of the list `xs` in a random order, in place.
