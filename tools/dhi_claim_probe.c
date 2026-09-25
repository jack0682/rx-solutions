/* Private input probe injected only into mutation fixtures, never installed. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static int seen;
int open(const char *path, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list args;
    va_start(args, flags);
    mode = va_arg(args, int);
    va_end(args);
  }
  int (*original)(const char *, int, ...) = dlsym(RTLD_NEXT, "open");
  if (!original)
    _exit(82);
  int fd = original(path, flags, mode);
  const char *endpoint = getenv("DHI_TEST_ENDPOINT");
  if (endpoint && !strcmp(path, endpoint) && ++seen == 2) {
    int extra = original(path, O_RDWR | O_NOCTTY | O_NONBLOCK);
    int error = errno;
    fprintf(
        stderr,
        "{\"probe\":\"excess_grant\",\"pid\":%d,\"opened\":%s,\"errno\":%d}\n",
        getpid(), extra >= 0 ? "true" : "false", error);
    if (extra >= 0) {
      close(extra);
      _exit(42);
    }
  }
  return fd;
}
