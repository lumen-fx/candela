#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

#include <stdint.h>
#include <time.h>

EXPORT int64_t now(void) { return (int64_t)time(NULL); }
EXPORT const char *format(int64_t timestamp, const char *fmt) {
  static char buffer[128];
  time_t t = (time_t)timestamp;
  struct tm *info = localtime(&t);
  strftime(buffer, 128, fmt, info);
  return buffer;
}