---
icon: lucide/dice-5
---
# Random library

!!! warning

    This is highly subject to change.

Import this library with `import "std/random";` at the top-level.

*[at the top-level]: Outside of any function.

## Random
`random() -> float`<br/>
Returns a random float within \[0;1\[.

## RandomRange
`random_range(min: float, max: float) -> float`<br/>
Returns a random float within the given extrema.

## RandomInt
`random_int() -> int`<br/>
Returns a random int.

## RandomIntRange
`random_int_range(min: int, max: int) -> int`<br/>
Returns a random int within the given extrema.

## Seed
`seed(seed: int)`<br/>
Seeds the RNG with `seed`.