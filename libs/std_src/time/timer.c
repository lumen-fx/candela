/* Sleeping and timers for std/time. A timer runs on a thread of its own and
 * posts an empty event to its source through the table the candela runtime
 * hands candela_dylib_init, so its function runs on the program's thread. */
#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#include <windows.h>
#else
#define _POSIX_C_SOURCE 200809L
#define EXPORT __attribute__((visibility("default")))
#include <pthread.h>
#include <time.h>
#endif

#include <stdint.h>
#include <stdlib.h>

typedef struct candela_runtime {
  uint32_t abi;
  int64_t (*new_source)(void);
  void (*post)(int64_t source, const char *payload);
  void (*hold)(int64_t source);
  void (*release)(int64_t source);
  const char *(*base_dir)(void);
} candela_runtime;

static const candela_runtime *runtime;

EXPORT void candela_dylib_init(const candela_runtime *table) {
  if (table != NULL && table->abi == 1) {
    runtime = table;
  }
}

/* One running timer. The list is guarded by `lock`, and `changed` wakes a
 * waiting timer when it is cancelled. */
typedef struct timer {
  int64_t source;
  int64_t ms;
  int repeat;
  int cancelled;
  struct timer *next;
} timer;

static timer *live;

#ifdef _WIN32
static SRWLOCK lock = SRWLOCK_INIT;
static CONDITION_VARIABLE changed = CONDITION_VARIABLE_INIT;
#define LOCK() AcquireSRWLockExclusive(&lock)
#define UNLOCK() ReleaseSRWLockExclusive(&lock)
#else
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed = PTHREAD_COND_INITIALIZER;
#define LOCK() pthread_mutex_lock(&lock)
#define UNLOCK() pthread_mutex_unlock(&lock)
#endif

static int64_t now_ms(void) {
#ifdef _WIN32
  return (int64_t)GetTickCount64();
#else
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (int64_t)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
#endif
}

/* Waits under `lock` until `deadline` or a cancel; answers whether the timer
 * was cancelled. */
static int wait_until(timer *t, int64_t deadline) {
  while (!t->cancelled) {
    int64_t left = deadline - now_ms();
    if (left <= 0) {
      return 0;
    }
#ifdef _WIN32
    SleepConditionVariableSRW(&changed, &lock, (DWORD)(left > 0x7ffffffe ? 0x7ffffffe : left), 0);
#else
    struct timespec until;
    clock_gettime(CLOCK_REALTIME, &until);
    until.tv_sec += (time_t)(left / 1000);
    until.tv_nsec += (long)(left % 1000) * 1000000L;
    if (until.tv_nsec >= 1000000000L) {
      until.tv_sec += 1;
      until.tv_nsec -= 1000000000L;
    }
    pthread_cond_timedwait(&changed, &lock, &until);
#endif
  }
  return 1;
}

static void unlink_timer(timer *t) {
  for (timer **at = &live; *at != NULL; at = &(*at)->next) {
    if (*at == t) {
      *at = t->next;
      return;
    }
  }
}

#ifdef _WIN32
static DWORD WINAPI run(LPVOID arg) {
#else
static void *run(void *arg) {
#endif
  timer *t = (timer *)arg;
  int64_t deadline = now_ms() + t->ms;
  LOCK();
  for (;;) {
    if (wait_until(t, deadline)) {
      break;
    }
    UNLOCK();
    runtime->post(t->source, "");
    LOCK();
    if (!t->repeat) {
      break;
    }
    deadline += t->ms;
  }
  unlink_timer(t);
  UNLOCK();
  runtime->release(t->source);
  free(t);
  return 0;
}

/* Starts a timer posting to `source` after `ms`, once, or every `ms` when
 * `repeat` is not 0. */
EXPORT void candela_timer_start(int64_t source, int64_t ms, int64_t repeat) {
  if (runtime == NULL) {
    return;
  }
  timer *t = (timer *)calloc(1, sizeof(timer));
  if (t == NULL) {
    return;
  }
  t->source = source;
  t->ms = ms < 1 ? 1 : ms;
  t->repeat = repeat != 0;
  runtime->hold(source);
  LOCK();
  t->next = live;
  live = t;
  UNLOCK();
#ifdef _WIN32
  HANDLE thread = CreateThread(NULL, 0, run, t, 0, NULL);
  int started = thread != NULL;
  if (started) {
    CloseHandle(thread);
  }
#else
  pthread_t thread;
  int started = pthread_create(&thread, NULL, run, t) == 0;
  if (started) {
    pthread_detach(thread);
  }
#endif
  if (!started) {
    LOCK();
    unlink_timer(t);
    UNLOCK();
    runtime->release(source);
    free(t);
  }
}

/* Cancels the timer under `source`: 1 when it was still running, 0 when not. */
EXPORT int64_t candela_timer_cancel(int64_t source) {
  int64_t found = 0;
  LOCK();
  for (timer *t = live; t != NULL; t = t->next) {
    if (t->source == source && !t->cancelled) {
      t->cancelled = 1;
      found = 1;
    }
  }
  UNLOCK();
#ifdef _WIN32
  WakeAllConditionVariable(&changed);
#else
  pthread_cond_broadcast(&changed);
#endif
  return found;
}

/* Pauses the calling thread for `ms` milliseconds. */
EXPORT void candela_sleep_ms(int64_t ms) {
  if (ms <= 0) {
    return;
  }
#ifdef _WIN32
  Sleep((DWORD)ms);
#else
  struct timespec ts = {(time_t)(ms / 1000), (long)(ms % 1000) * 1000000L};
  while (nanosleep(&ts, &ts) != 0) {
  }
#endif
}
