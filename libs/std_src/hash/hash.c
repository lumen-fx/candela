/* MD5 (RFC 1321), SHA-1 and SHA-256 (FIPS 180-4) over a NUL-terminated
 * string, each answered as lowercase hex. The bytes hashed are the string's
 * UTF-8 bytes, up to the terminating NUL.
 *
 * The answer lives in a per-thread buffer that the next call on the same
 * thread overwrites; the runtime copies it out before that can happen.
 */
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#include <stddef.h>
#include <stdint.h>
#include <string.h>

/* Large enough for the widest answer, SHA-256's 64 hex digits, and the NUL. */
static _Thread_local char out[65];

static const char HEX[] = "0123456789abcdef";

static const char *to_hex(const uint8_t *digest, size_t len) {
  for (size_t i = 0; i < len; i++) {
    out[2 * i] = HEX[digest[i] >> 4];
    out[2 * i + 1] = HEX[digest[i] & 15];
  }
  out[2 * len] = '\0';
  return out;
}

static uint32_t rotl(uint32_t x, int n) { return (x << n) | (x >> (32 - n)); }
static uint32_t rotr(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }

/* Feeds `msg` through `block` in 64-byte blocks with the Merkle-Damgard
 * padding all three digests share: a 0x80 byte, zeros, then the message
 * length in bits as eight bytes, little-endian for MD5 and big-endian for the
 * SHA family. */
static void pad_and_run(const uint8_t *msg, size_t len, int big_endian,
                        void (*block)(void *, const uint8_t *), void *state) {
  size_t full = len / 64;
  for (size_t i = 0; i < full; i++) {
    block(state, msg + 64 * i);
  }
  uint8_t tail[128];
  size_t rest = len - 64 * full;
  memcpy(tail, msg + 64 * full, rest);
  tail[rest] = 0x80;
  size_t tail_len = rest < 56 ? 64 : 128;
  memset(tail + rest + 1, 0, tail_len - rest - 1);
  uint64_t bits = (uint64_t)len * 8;
  for (int i = 0; i < 8; i++) {
    int shift = big_endian ? 56 - 8 * i : 8 * i;
    tail[tail_len - 8 + i] = (uint8_t)(bits >> shift);
  }
  block(state, tail);
  if (tail_len == 128) {
    block(state, tail + 64);
  }
}

/* ---- MD5 ---------------------------------------------------------------- */

static const uint32_t MD5_K[64] = {
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a,
    0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
    0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8,
    0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
    0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
    0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391};

static const int MD5_S[64] = {7,  12, 17, 22, 7,  12, 17, 22, 7,  12, 17, 22, 7,
                              12, 17, 22, 5,  9,  14, 20, 5,  9,  14, 20, 5,  9,
                              14, 20, 5,  9,  14, 20, 4,  11, 16, 23, 4,  11, 16,
                              23, 4,  11, 16, 23, 4,  11, 16, 23, 6,  10, 15, 21,
                              6,  10, 15, 21, 6,  10, 15, 21, 6,  10, 15, 21};

static void md5_block(void *state, const uint8_t *p) {
  uint32_t *h = state;
  uint32_t m[16];
  for (int i = 0; i < 16; i++) {
    m[i] = (uint32_t)p[4 * i] | (uint32_t)p[4 * i + 1] << 8 |
           (uint32_t)p[4 * i + 2] << 16 | (uint32_t)p[4 * i + 3] << 24;
  }
  uint32_t a = h[0], b = h[1], c = h[2], d = h[3];
  for (int i = 0; i < 64; i++) {
    uint32_t f;
    int g;
    if (i < 16) {
      f = (b & c) | (~b & d);
      g = i;
    } else if (i < 32) {
      f = (d & b) | (~d & c);
      g = (5 * i + 1) & 15;
    } else if (i < 48) {
      f = b ^ c ^ d;
      g = (3 * i + 5) & 15;
    } else {
      f = c ^ (b | ~d);
      g = (7 * i) & 15;
    }
    uint32_t next = d;
    d = c;
    c = b;
    b = b + rotl(a + f + MD5_K[i] + m[g], MD5_S[i]);
    a = next;
  }
  h[0] += a;
  h[1] += b;
  h[2] += c;
  h[3] += d;
}

EXPORT const char *candela_md5(const char *s) {
  uint32_t h[4] = {0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476};
  pad_and_run((const uint8_t *)s, strlen(s), 0, md5_block, h);
  uint8_t digest[16];
  for (int i = 0; i < 16; i++) {
    digest[i] = (uint8_t)(h[i / 4] >> (8 * (i % 4)));
  }
  return to_hex(digest, 16);
}

/* ---- SHA-1 -------------------------------------------------------------- */

static uint32_t load_be(const uint8_t *p) {
  return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 |
         (uint32_t)p[3];
}

static void sha1_block(void *state, const uint8_t *p) {
  uint32_t *h = state;
  uint32_t w[80];
  for (int i = 0; i < 16; i++) {
    w[i] = load_be(p + 4 * i);
  }
  for (int i = 16; i < 80; i++) {
    w[i] = rotl(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
  }
  uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];
  for (int i = 0; i < 80; i++) {
    uint32_t f, k;
    if (i < 20) {
      f = (b & c) | (~b & d);
      k = 0x5a827999;
    } else if (i < 40) {
      f = b ^ c ^ d;
      k = 0x6ed9eba1;
    } else if (i < 60) {
      f = (b & c) | (b & d) | (c & d);
      k = 0x8f1bbcdc;
    } else {
      f = b ^ c ^ d;
      k = 0xca62c1d6;
    }
    uint32_t t = rotl(a, 5) + f + e + k + w[i];
    e = d;
    d = c;
    c = rotl(b, 30);
    b = a;
    a = t;
  }
  h[0] += a;
  h[1] += b;
  h[2] += c;
  h[3] += d;
  h[4] += e;
}

static void store_be(const uint32_t *h, int words, uint8_t *digest) {
  for (int i = 0; i < 4 * words; i++) {
    digest[i] = (uint8_t)(h[i / 4] >> (24 - 8 * (i % 4)));
  }
}

EXPORT const char *candela_sha1(const char *s) {
  uint32_t h[5] = {0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476,
                   0xc3d2e1f0};
  pad_and_run((const uint8_t *)s, strlen(s), 1, sha1_block, h);
  uint8_t digest[20];
  store_be(h, 5, digest);
  return to_hex(digest, 20);
}

/* ---- SHA-256 ------------------------------------------------------------ */

static const uint32_t SHA256_K[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2};

static void sha256_block(void *state, const uint8_t *p) {
  uint32_t *h = state;
  uint32_t w[64];
  for (int i = 0; i < 16; i++) {
    w[i] = load_be(p + 4 * i);
  }
  for (int i = 16; i < 64; i++) {
    uint32_t s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
    uint32_t s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
    w[i] = w[i - 16] + s0 + w[i - 7] + s1;
  }
  uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5],
           g = h[6], hh = h[7];
  for (int i = 0; i < 64; i++) {
    uint32_t s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
    uint32_t ch = (e & f) ^ (~e & g);
    uint32_t t1 = hh + s1 + ch + SHA256_K[i] + w[i];
    uint32_t s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
    uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
    uint32_t t2 = s0 + maj;
    hh = g;
    g = f;
    f = e;
    e = d + t1;
    d = c;
    c = b;
    b = a;
    a = t1 + t2;
  }
  h[0] += a;
  h[1] += b;
  h[2] += c;
  h[3] += d;
  h[4] += e;
  h[5] += f;
  h[6] += g;
  h[7] += hh;
}

EXPORT const char *candela_sha256(const char *s) {
  uint32_t h[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                   0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
  pad_and_run((const uint8_t *)s, strlen(s), 1, sha256_block, h);
  uint8_t digest[32];
  store_be(h, 8, digest);
  return to_hex(digest, 32);
}
