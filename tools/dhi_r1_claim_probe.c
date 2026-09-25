/* Private R1 test instrumentation. Never installed in the product image. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/openat2.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static void result(const char *name, int value, int error) {
  fprintf(stderr, "{\"r1_probe\":\"%s\",\"result\":%d,\"errno\":%d}\n", name,
          value, error);
}

#ifdef TEST_SURFACES
static void denied(const char *name, int value) {
  int error = errno;
  result(name, value, error);
  if (value >= 0) {
    close(value);
    _exit(42);
  }
  if (error != EPERM)
    _exit(43);
}

__attribute__((constructor)) static void surfaces(void) {
  const char *endpoint = getenv("DHI_TEST_ENDPOINT");
  if (!endpoint)
    _exit(80);
  denied("open_absolute", open(endpoint, O_RDWR | O_NOCTTY | O_NONBLOCK));
  unlink("/tmp/dhi-r1-outside-link");
  if (symlink(endpoint, "/tmp/dhi-r1-outside-link"))
    _exit(81);
  denied("open_symlink",
         open("/tmp/dhi-r1-outside-link", O_RDWR | O_NOCTTY | O_NONBLOCK));
  denied("open_proc_self_fd", open("/proc/self/fd/0", O_RDWR | O_NONBLOCK));
  int directory = open("/dev/pts", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
  if (directory < 0)
    _exit(82);
  const char *leaf = strrchr(endpoint, '/');
  denied("openat_dirfd", openat(directory, leaf ? leaf + 1 : endpoint,
                                O_RDWR | O_NOCTTY | O_NONBLOCK));
  close(directory);
#ifdef SYS_openat2
  struct open_how how = {.flags = O_RDWR | O_NOCTTY | O_NONBLOCK};
  denied("openat2_absolute",
         syscall(SYS_openat2, AT_FDCWD, endpoint, &how, sizeof(how)));
  how.flags = O_RDONLY;
  how.resolve = RESOLVE_NO_SYMLINKS;
  denied("openat2_resolve",
         syscall(SYS_openat2, AT_FDCWD, "/tmp", &how, sizeof(how)));
#endif
#ifdef SYS_open_by_handle_at
  denied("open_by_handle_at",
         syscall(SYS_open_by_handle_at, -1, NULL, O_RDONLY));
#endif
#ifdef SYS_io_uring_setup
  denied("io_uring_setup", syscall(SYS_io_uring_setup, 1, NULL));
#endif
#ifdef SYS_pidfd_getfd
  denied("pidfd_getfd", syscall(SYS_pidfd_getfd, -1, 0, 0));
#endif
}
#endif

#ifdef TEST_LATE_FORK
static int opens;
int open(const char *path, int flags, ...) {
  mode_t mode = 0;
  if (flags & O_CREAT) {
    va_list arguments;
    va_start(arguments, flags);
    mode = va_arg(arguments, int);
    va_end(arguments);
  }
  int (*original)(const char *, int, ...) = dlsym(RTLD_NEXT, "open");
  if (!original)
    _exit(83);
  int descriptor = original(path, flags, mode);
  const char *custody = getenv("DHI_TEST_CUSTODY");
  if (descriptor >= 0 && custody && !strcmp(path, custody) && ++opens == 2) {
    int descriptor_flags = fcntl(descriptor, F_GETFD);
    result("grant_fd_cloexec", descriptor_flags & FD_CLOEXEC, errno);
    if (descriptor_flags < 0 || !(descriptor_flags & FD_CLOEXEC))
      _exit(87);
    pid_t child = fork();
    int error = errno;
    result("late_fork", child, error);
    if (child < 0) {
      if (error != EPERM)
        _exit(84);
      return descriptor;
    }
    if (!child) {
      ssize_t written = write(descriptor, "X", 1);
      result("inherited_fd_write", (int)written, errno);
      _exit(written == 1 ? 42 : 85);
    }
    int status = 0;
    if (waitpid(child, &status, 0) != child)
      _exit(86);
    result("late_fork_child_status", status, 0);
    _exit(81);
  }
  return descriptor;
}
#endif
