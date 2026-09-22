/* This file is derived from keel (https://github.com/horacehoff/keel),
 * Copyright 2026 Horace Hoff, licensed under the Apache License, Version 2.0.
 * It has been modified by the candela authors. See the NOTICE file.
 */
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#include <limits.h>
#include <math.h>
#include <stdint.h>

// An exponent wider than a C int says the same thing as the largest one that
// fits: the result is zero or an infinity either way.
static int clamp_exponent(int64_t y) {
  if (y > INT_MAX) {
    return INT_MAX;
  }
  if (y < INT_MIN) {
    return INT_MIN;
  }
  return (int)y;
}

EXPORT double candela_acos(double x) { return acos(x); }
EXPORT double candela_asin(double x) { return asin(x); }
EXPORT double candela_atan(double x) { return atan(x); }
EXPORT double candela_atan2(double x, double y) { return atan2(x, y); }
EXPORT double candela_cos(double x) { return cos(x); }
EXPORT double candela_sin(double x) { return sin(x); }
EXPORT double candela_tan(double x) { return tan(x); }
EXPORT double candela_acosh(double x) { return acosh(x); }
EXPORT double candela_asinh(double x) { return asinh(x); }
EXPORT double candela_atanh(double x) { return atanh(x); }
EXPORT double candela_cosh(double x) { return cosh(x); }
EXPORT double candela_sinh(double x) { return sinh(x); }
EXPORT double candela_tanh(double x) { return tanh(x); }
EXPORT double candela_exp(double x) { return exp(x); }
EXPORT double candela_expm1(double x) { return expm1(x); }
EXPORT double candela_log(double x) { return log(x); }
EXPORT double candela_log10(double x) { return log10(x); }
EXPORT double candela_log2(double x) { return log2(x); }
EXPORT double candela_log1p(double x) { return log1p(x); }
EXPORT double candela_logb(double x) { return logb(x); }
EXPORT double candela_ldexp(double x, int64_t y) {
  return ldexp(x, clamp_exponent(y));
}
EXPORT int64_t candela_ilogb(double x) { return (int64_t)ilogb(x); }
EXPORT double candela_scalbn(double x, int64_t y) {
  return scalbn(x, clamp_exponent(y));
}
EXPORT double candela_cbrt(double x) { return cbrt(x); }
EXPORT double candela_hypot(double x, double y) { return hypot(x, y); }
EXPORT double candela_erf(double x) { return erf(x); }
EXPORT double candela_erfc(double x) { return erfc(x); }
EXPORT double candela_sqrt(double x) { return sqrt(x); }
EXPORT double candela_pow(double x, double y) { return pow(x, y); }
EXPORT double candela_floor(double x) { return floor(x); }
EXPORT double candela_ceil(double x) { return ceil(x); }
EXPORT double candela_round(double x) { return round(x); }
EXPORT double candela_trunc(double x) { return trunc(x); }
EXPORT double candela_fmod(double x, double y) { return fmod(x, y); }
EXPORT double candela_copysign(double x, double y) { return copysign(x, y); }