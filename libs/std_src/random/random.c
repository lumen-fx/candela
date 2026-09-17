#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#include "pcg_basic.h"
#include <math.h>
#include <stdint.h>
#include <time.h>

static pcg32_random_t rng;
static int8_t seeded = 0;

inline void seed(void) {
  pcg32_srandom_r(&rng, time(NULL), clock());
  seeded = 1;
}

EXPORT void candela_seed(int64_t seed) {
  pcg32_srandom_r(&rng, (uint64_t)seed, 54u);
  seeded = 1;
}

// The generator draws 32 bits at a time, so a full-width draw is two of them.
static uint64_t draw64(void) {
  if (seeded == 0) {
    seed();
  }
  uint64_t high = pcg32_random_r(&rng);
  return (high << 32) | pcg32_random_r(&rng);
}

EXPORT int64_t candela_random_int(void) { return (int64_t)draw64(); }

EXPORT int64_t candela_random_int_range(int64_t min, int64_t max) {
  uint64_t span = (uint64_t)max - (uint64_t)min + 1u;
  if (span == 0u) {
    // The whole range: every draw is in it, and the modulus below is not.
    return (int64_t)draw64();
  }
  // Rejection sampling, as pcg32_boundedrand_r does at 32 bits: drop the draws
  // in the short tail so every value in the span comes up equally often.
  uint64_t threshold = ((uint64_t)0 - span) % span;
  for (;;) {
    uint64_t r = draw64();
    if (r >= threshold) {
      return min + (int64_t)(r % span);
    }
  }
}

EXPORT double candela_random(void) {
  if (seeded == 0) {
    seed();
  }
  return ldexp(pcg32_random_r(&rng), -32);
}

EXPORT double candela_random_float_range(double min, double max) {
  if (seeded == 0) {
    seed();
  }
  return min + (candela_random() * (max - min));
}