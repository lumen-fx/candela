# math

Trigonometry, logarithms, roots, rounding, and the constants.

```rust
import "std/math" as math;
```

Every function here takes and returns floats, apart from the three that take or
return an int where the underlying operation is integral (`ldexp`, `ilogb`,
`scalbn`).

None of them raises. An argument outside a function's domain gives the platform's
floating-point answer. An infinity behaves like any other float and prints as
`inf` or `-inf`. A not-a-number result does not: it compares equal to itself, and
passing it to `print` stops the process. Check the domain before you call, rather
than testing the result afterwards.

The module binds a small dynamic library that wraps the platform maths library,
so it is one of the three std modules that need that library present at run time.
A `.cdlb` built from a program that imports `std/math` records the binding by
name and re-opens it when the artifact runs; see
[artifacts](../reference/artifacts.md).

Four common operations are also built-in methods on a number, with no import:
`x.abs()`, `x.sqrt()`, `x.round()`, and `x.floor()`.

## Trigonometry

| Function | Returns |
| --- | --- |
| `math::sin(x)` | The sine of `x`, in radians |
| `math::cos(x)` | The cosine of `x`, in radians |
| `math::tan(x)` | The tangent of `x`, in radians |
| `math::asin(x)` | The arc sine of `x`, in radians |
| `math::acos(x)` | The arc cosine of `x`, in radians |
| `math::atan(x)` | The arc tangent of `x`, in radians |
| `math::atan2(x, y)` | The arc tangent of `x / y`, using the signs of both to pick the quadrant |
| `math::sinh(x)` | The hyperbolic sine of `x` |
| `math::cosh(x)` | The hyperbolic cosine of `x` |
| `math::tanh(x)` | The hyperbolic tangent of `x` |
| `math::asinh(x)` | The inverse hyperbolic sine of `x` |
| `math::acosh(x)` | The inverse hyperbolic cosine of `x` |
| `math::atanh(x)` | The inverse hyperbolic tangent of `x` |

## Exponentials and logarithms

| Function | Returns |
| --- | --- |
| `math::exp(x)` | e raised to the power `x` |
| `math::expm1(x)` | `exp(x) - 1`, accurate for small `x` |
| `math::log(x)` | The natural logarithm of `x` |
| `math::log2(x)` | The base-2 logarithm of `x` |
| `math::log10(x)` | The base-10 logarithm of `x` |
| `math::log1p(x)` | `log(1 + x)`, accurate for small `x` |
| `math::logb(x)` | The exponent of `x` in the floating-point radix, as a float |
| `math::ilogb(x)` | The same exponent, as an int |
| `math::ldexp(x, n)` | `x` multiplied by 2 raised to the int `n` |
| `math::scalbn(x, n)` | `x` scaled by the radix raised to the int `n` |
| `math::pow(x, y)` | `x` raised to the power `y` |

## Roots and distances

| Function | Returns |
| --- | --- |
| `math::sqrt(x)` | The square root of `x` |
| `math::cbrt(x)` | The cube root of `x` |
| `math::hypot(x, y)` | The length of the hypotenuse with sides `x` and `y`, without intermediate overflow |
| `math::erf(x)` | The error function of `x` |
| `math::erfc(x)` | The complementary error function, `1 - erf(x)` |

## Rounding and sign

| Function | Returns |
| --- | --- |
| `math::floor(x)` | The largest whole number not greater than `x` |
| `math::ceil(x)` | The smallest whole number not less than `x` |
| `math::round(x)` | `x` rounded to the nearest whole number, halves away from zero |
| `math::trunc(x)` | `x` with its fractional part discarded, towards zero |
| `math::fmod(x, y)` | The remainder of `x / y`, with the sign of `x` |
| `math::copysign(x, y)` | The magnitude of `x` with the sign of `y` |

All of these return a float, including `floor`, `ceil`, `round`, and `trunc`.
Convert with `int(...)` when you want a whole-number type.

## Constants

Each constant is a function call, so it costs a call and returns a float.

| Function | Returns |
| --- | --- |
| `math::pi()` | The ratio of a circle's circumference to its diameter |
| `math::e()` | The base of the natural logarithm |
| `math::tau()` | The ratio of a circle's circumference to its radius, two pi |

```rust
import "std/math" as math;

fn main() {
    print(math::sqrt(2.0));
    print(math::round(math::sin(math::pi() / 2.0)));
    print(int(math::floor(7.0 / 2.0)));
}
```
