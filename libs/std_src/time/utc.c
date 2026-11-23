/* UTC formatting for std/time, beside time.c's local-time `format`. */
#ifndef _WIN32
/* gmtime_r is POSIX, which a strict C11 build leaves out unless asked. */
#define _POSIX_C_SOURCE 200809L
#endif
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#include <stdint.h>
#include <time.h>

EXPORT const char *candela_format_utc(int64_t timestamp, const char *fmt) {
  static char buffer[128];
  time_t t = (time_t)timestamp;
  struct tm info;
#ifdef _WIN32
  if (gmtime_s(&info, &t) != 0) {
    buffer[0] = '\0';
    return buffer;
  }
#else
  if (gmtime_r(&t, &info) == NULL) {
    buffer[0] = '\0';
    return buffer;
  }
#endif
  if (strftime(buffer, sizeof buffer, fmt, &info) == 0) {
    buffer[0] = '\0';
  }
  return buffer;
}
