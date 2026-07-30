---
icon: lucide/omega
---
# Math library

!!! warning

    This is highly subject to change.

Import this library with `import "std/math";` at the top-level.

*[at the top-level]: Outside of any function.

### Acos
`acos(x: float) -> float`
### Asin
`asin(x: float) -> float`
### Atan
`atan(x: float) -> float`
### Atan2
`atan2(x: float, y: float) -> float`
### Cos
`cos(x: float) -> float`
### Sin
`sin(x: float) -> float`
### Tan
`tan(x: float) -> float`
### Acosh
`acosh(x: float) -> float`
### Asinh
`asinh(x: float) -> float`
### Atanh
`atanh(x: float) -> float`
### Cosh
`cosh(x: float) -> float`
### Sinh
`sinh(x: float) -> float`
### Tanh
`tanh(x: float) -> float`
### Exp
`exp(x: float) -> float`
### Expm1
`expm1(x: float) -> float`
### Log
`log(x: float) -> float`
### Log10
`log10(x: float) -> float`
### Log2
`log2(x: float) -> float`
### Log1p
`log1p(x: float) -> float`
### Logb
`logb(x: float) -> float`
### Ldexp
`ldexp(x: float, y: int) -> float`
### Ilogb
`ilogb(x: float) -> int`
### Scalbn
`scalbn(x: float, y: int) -> float`
### Cbrt
`cbrt(x: float) -> float`
### Hypot
`hypot(x: float, y: float) -> float`
### Erf
`erf(x: float) -> float`
### Erfc
`erfc(x: float) -> float`
### Sqrt
`sqrt(x: float) -> float`
### Pow
`pow(x: float, y: float) -> float` -- `x` raised to the power `y`.
### Floor
`floor(x: float) -> float` -- the largest integer value not greater than `x`.
### Ceil
`ceil(x: float) -> float` -- the smallest integer value not less than `x`.
### Round
`round(x: float) -> float` -- `x` rounded to the nearest integer, halves away from zero.
### Trunc
`trunc(x: float) -> float` -- `x` with its fractional part removed.
### Fmod
`fmod(x: float, y: float) -> float` -- the floating-point remainder of `x / y`.
### Copysign
`copysign(x: float, y: float) -> float` -- the magnitude of `x` with the sign of `y`.

## Constants

Constants are exposed as zero-argument functions.

### Pi
`pi() -> float` -- the ratio of a circle's circumference to its diameter.
### E
`e() -> float` -- the base of the natural logarithm.
### Tau
`tau() -> float` -- the ratio of a circle's circumference to its radius (`2 * pi`).