/* RX fresh-PTY descriptor custody. The guardian never authors device command
 * bytes. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <linux/audit.h>
#include <linux/capability.h>
#include <linux/filter.h>
#include <linux/memfd.h>
#include <linux/openat2.h>
#include <linux/seccomp.h>
#include <poll.h>
#include <pty.h>
#include <sched.h>
#include <signal.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/sysmacros.h>
#include <sys/uio.h>
#include <sys/wait.h>
#include <unistd.h>
#if defined(__aarch64__)
#define RX_ARCH AUDIT_ARCH_AARCH64
#elif defined(__x86_64__)
#define RX_ARCH AUDIT_ARCH_X86_64
#else
#error DHI_ARCH_UNSUPPORTED
#endif
#define MANAGER "/opt/rx/bin/rx-dhi-controller-manager"
static volatile sig_atomic_t stopping;
static void stop(int sig) {
  (void)sig;
  stopping = 1;
}
static void fail(const char *name) {
  fprintf(stderr, "{\"refusal\":\"%s\",\"errno\":%d}\n", name, errno);
  exit(90);
}
static void pass(int s, int fd, const char *b) {
  char control[CMSG_SPACE(sizeof(int))] = {0};
  struct iovec io = {(void *)b, strlen(b) + 1};
  struct msghdr m = {.msg_iov = &io,
                     .msg_iovlen = 1,
                     .msg_control = control,
                     .msg_controllen = sizeof(control)};
  struct cmsghdr *c = CMSG_FIRSTHDR(&m);
  c->cmsg_level = SOL_SOCKET;
  c->cmsg_type = SCM_RIGHTS;
  c->cmsg_len = CMSG_LEN(sizeof(int));
  memcpy(CMSG_DATA(c), &fd, sizeof(fd));
  if (sendmsg(s, &m, MSG_NOSIGNAL) != (ssize_t)io.iov_len)
    fail("DHI_HANDSHAKE_FAILED");
}
static int take(int s) {
  char b[128], control[CMSG_SPACE(sizeof(int))] = {0};
  struct iovec io = {b, sizeof(b)};
  struct msghdr m = {.msg_iov = &io,
                     .msg_iovlen = 1,
                     .msg_control = control,
                     .msg_controllen = sizeof(control)};
  if (recvmsg(s, &m, MSG_CMSG_CLOEXEC) < 1)
    fail("DHI_LISTENER_ABSENT");
  struct cmsghdr *c = CMSG_FIRSTHDR(&m);
  if (!c || c->cmsg_level != SOL_SOCKET || c->cmsg_type != SCM_RIGHTS ||
      c->cmsg_len != CMSG_LEN(sizeof(int)) || (m.msg_flags & MSG_CTRUNC))
    fail("DHI_LISTENER_INVALID");
  int fd;
  memcpy(&fd, CMSG_DATA(c), sizeof(fd));
  return fd;
}
static int tgid(pid_t tid) {
  char p[80], line[256];
  snprintf(p, sizeof(p), "/proc/%d/status", tid);
  FILE *f = fopen(p, "r");
  if (!f)
    return -1;
  int id = -1;
  while (fgets(line, sizeof(line), f))
    if (sscanf(line, "Tgid: %d", &id) == 1)
      break;
  fclose(f);
  return id;
}
static int same_exe(pid_t tid, struct stat *expected) {
  char p[80];
  struct stat actual;
  snprintf(p, sizeof(p), "/proc/%d/exe", tid);
  return !stat(p, &actual) && actual.st_dev == expected->st_dev &&
         actual.st_ino == expected->st_ino;
}
static ssize_t read_remote(pid_t tid, unsigned long address, void *bytes,
                           size_t size) {
  struct iovec local = {bytes, size}, remote = {(void *)address, size};
  return process_vm_readv(tid, &local, 1, &remote, 1, 0);
}
static int read_remote_string(pid_t tid, unsigned long address, char *text,
                              size_t size) {
  ssize_t read = read_remote(tid, address, text, size);
  return read > 0 && memchr(text, 0, (size_t)read) != NULL;
}
struct open_request {
  int dirfd;
  unsigned long flags;
  unsigned long path_address;
};
static const char *decode_open(const struct seccomp_notif *request,
                               struct open_request *open) {
  memset(open, 0, sizeof(*open));
#ifdef __NR_open
  if (request->data.nr == __NR_open) {
    open->dirfd = AT_FDCWD;
    open->path_address = request->data.args[0];
    open->flags = request->data.args[1];
    return NULL;
  }
#endif
  if (request->data.nr == __NR_openat) {
    open->dirfd = (int)request->data.args[0];
    open->path_address = request->data.args[1];
    open->flags = request->data.args[2];
    return NULL;
  }
#ifdef __NR_openat2
  if (request->data.nr == __NR_openat2) {
    struct open_how how = {0};
    if (request->data.args[3] < sizeof(how) ||
        read_remote(request->pid, request->data.args[2], &how, sizeof(how)) !=
            (ssize_t)sizeof(how))
      return "DHI_OPENAT2_LAYOUT_UNSUPPORTED";
    open->dirfd = (int)request->data.args[0];
    open->path_address = request->data.args[1];
    open->flags = how.flags;
    if (how.resolve)
      return "DHI_OPENAT2_RESOLVE_UNSUPPORTED";
    return NULL;
  }
#endif
  return "DHI_OPEN_SYSCALL_UNSUPPORTED";
}
static const char *resource_path(pid_t tid, int dirfd, const char *requested,
                                 char *resolved, size_t size) {
  int written;
  if (requested[0] == '/') {
    if (!strncmp(requested, "/proc/self/", 11))
      written = snprintf(resolved, size, "/proc/%d/%s", tid, requested + 11);
    else if (!strncmp(requested, "/proc/thread-self/", 18))
      written = snprintf(resolved, size, "/proc/%d/%s", tid, requested + 18);
    else if (!strncmp(requested, "/dev/fd/", 8))
      written = snprintf(resolved, size, "/proc/%d/fd/%s", tid, requested + 8);
    else if (!strcmp(requested, "/dev/stdin"))
      written = snprintf(resolved, size, "/proc/%d/fd/0", tid);
    else if (!strcmp(requested, "/dev/stdout"))
      written = snprintf(resolved, size, "/proc/%d/fd/1", tid);
    else if (!strcmp(requested, "/dev/stderr"))
      written = snprintf(resolved, size, "/proc/%d/fd/2", tid);
    else
      written = snprintf(resolved, size, "/proc/%d/root%s", tid, requested);
  } else if (dirfd == AT_FDCWD) {
    written = snprintf(resolved, size, "/proc/%d/cwd/%s", tid, requested);
  } else {
    written =
        snprintf(resolved, size, "/proc/%d/fd/%d/%s", tid, dirfd, requested);
  }
  if (written < 0 || (size_t)written >= size)
    return "DHI_RESOURCE_PATH_TOO_LONG";
  return NULL;
}
static int could_name_device(const char *requested, unsigned long flags) {
  int proc_fd = !strncmp(requested, "/proc/self/fd/", 14) ||
                !strncmp(requested, "/proc/thread-self/fd/", 21) ||
                !strncmp(requested, "/dev/fd/", 8);
  int direct_dev = !strncmp(requested, "/dev/", 5) &&
                   strncmp(requested, "/dev/shm/", 9) &&
                   strncmp(requested, "/dev/mqueue/", 12) &&
                   strncmp(requested, "/dev/hugepages/", 15);
  return proc_fd || (direct_dev && !(flags & O_CREAT));
}
static const char *clone_disposition(const struct seccomp_notif *request,
                                     int *allow_thread) {
  unsigned long long flags = 0;
  *allow_thread = 0;
#ifdef __NR_clone
  if (request->data.nr == __NR_clone)
    flags = request->data.args[0];
  else
#endif
#ifdef __NR_clone3
      if (request->data.nr == __NR_clone3) {
    unsigned long long clone_flags = 0;
    if (request->data.args[1] < sizeof(clone_flags) ||
        read_remote(request->pid, request->data.args[0], &clone_flags,
                    sizeof(clone_flags)) != (ssize_t)sizeof(clone_flags))
      return "DHI_CLONE3_LAYOUT_UNSUPPORTED";
    flags = clone_flags;
  } else
#endif
    return "DHI_PROCESS_FORK_UNSUPPORTED";
  if (flags & CLONE_THREAD) {
    *allow_thread = 1;
    return NULL;
  }
  return "DHI_PROCESS_FORK_UNSUPPORTED";
}
static void print_json_string(const char *text) {
  putchar('"');
  for (const unsigned char *byte = (const unsigned char *)text; *byte; byte++) {
    if (*byte == '"' || *byte == '\\')
      printf("\\%c", *byte);
    else if (*byte >= 0x20 && *byte < 0x7f)
      putchar(*byte);
    else
      printf("\\u%04x", *byte);
  }
  putchar('"');
}
static const char *syscall_name(int number) {
#ifdef __NR_open
  if (number == __NR_open)
    return "open";
#endif
  if (number == __NR_openat)
    return "openat";
#ifdef __NR_openat2
  if (number == __NR_openat2)
    return "openat2";
#endif
#ifdef __NR_clone
  if (number == __NR_clone)
    return "clone";
#endif
#ifdef __NR_clone3
  if (number == __NR_clone3)
    return "clone3";
#endif
#ifdef __NR_fork
  if (number == __NR_fork)
    return "fork";
#endif
#ifdef __NR_vfork
  if (number == __NR_vfork)
    return "vfork";
#endif
#ifdef __NR_open_by_handle_at
  if (number == __NR_open_by_handle_at)
    return "open_by_handle_at";
#endif
#ifdef __NR_io_uring_setup
  if (number == __NR_io_uring_setup)
    return "io_uring_setup";
#endif
#ifdef __NR_pidfd_getfd
  if (number == __NR_pidfd_getfd)
    return "pidfd_getfd";
#endif
  return "unsupported";
}
static void refuse(struct seccomp_notif_resp *response,
                   const struct seccomp_notif *request, const char *reason,
                   const char *path, const struct stat *resource) {
  response->error = -EPERM;
  printf("{\"refusal\":\"%s\",\"syscall\":\"%s\",\"tid\":%u,"
         "\"notification\":%llu",
         reason, syscall_name(request->data.nr), request->pid,
         (unsigned long long)request->id);
  if (path) {
    printf(",\"requested_path\":");
    print_json_string(path);
  }
  if (resource)
    printf(",\"resource_kind\":\"CHARACTER_DEVICE\",\"resource_major\":%u,"
           "\"resource_minor\":%u",
           major(resource->st_rdev), minor(resource->st_rdev));
  printf("}\n");
  fflush(stdout);
}
int main(int argc, char **argv) {
  if (argc != 2)
    fail("DHI_ARGUMENTS_INVALID");
  char *argument_end = NULL;
  errno = 0;
  long descriptor = strtol(argv[1], &argument_end, 10);
  if (errno || !argument_end || *argument_end || descriptor < 0 ||
      descriptor > INT_MAX)
    fail("DHI_SESSION_CHANNEL_ARGUMENT_INVALID");
  int observer = (int)descriptor;
  struct ucred peer = {0};
  socklen_t peer_size = sizeof(peer);
  pid_t session = getppid();
  if (getsockopt(observer, SOL_SOCKET, SO_PEERCRED, &peer, &peer_size) ||
      peer_size != sizeof(peer) || peer.pid != session)
    fail("DHI_SESSION_CHANNEL_UNATTESTED");
  if (prctl(PR_SET_PDEATHSIG, SIGKILL) || getppid() != session)
    fail("DHI_SESSION_LOST");
  struct __user_cap_header_struct cap_header = {_LINUX_CAPABILITY_VERSION_3, 0};
  struct __user_cap_data_struct capabilities[2] = {{0}, {0}};
  if (syscall(SYS_capget, &cap_header, capabilities))
    fail("DHI_CAPABILITY_POSTURE_UNVERIFIABLE");
  for (int i = 0; i < 2; i++)
    if (capabilities[i].effective || capabilities[i].permitted ||
        capabilities[i].inheritable)
      fail("DHI_CAPABILITY_POSTURE_UNSUPPORTED");
  if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))
    fail("DHI_NO_NEW_PRIVILEGES_UNAVAILABLE");
  const char *instance = getenv("RX_PROCESS_INSTANCE_ID");
  if (!instance || strlen(instance) != 36)
    fail("DHI_INSTANCE_REQUIRED");
  for (int i = 0; i < 36; i++)
    if (!((instance[i] >= '0' && instance[i] <= '9') ||
          (instance[i] >= 'a' && instance[i] <= 'f') || instance[i] == '-'))
      fail("DHI_INSTANCE_INVALID");
  char namespace_arg[100];
  snprintf(namespace_arg, sizeof(namespace_arg), "__ns:=/rx_dhi_%s", instance);
  for (char *c = namespace_arg; *c; c++)
    if (*c == '-')
      *c = '_';
  int config = syscall(SYS_memfd_create, "rx-dhi-fixed-config",
                       MFD_CLOEXEC | MFD_ALLOW_SEALING);
  const char yaml[] =
      "/**:\n  ros__parameters:\n    update_rate: 20\n    "
      "hardware_components_initial_state:\n      inactive: [DxlHardware]\n";
  if (config < 0 ||
      write(config, yaml, sizeof(yaml) - 1) != (ssize_t)sizeof(yaml) - 1 ||
      fcntl(config, F_ADD_SEALS,
            F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL))
    fail("DHI_FIXED_CONFIGURATION_UNAVAILABLE");
  char config_path[80];
  snprintf(config_path, sizeof(config_path), "/proc/self/fd/%d", config);
  int master, keeper, s[2];
  char path[128];
  master = open("/dev/ptmx", O_RDWR | O_NOCTTY | O_CLOEXEC);
  if (master < 0)
    fail("DHI_FRESH_PTY_UNAVAILABLE");
  unsigned index = 0;
  int locked = 0;
  if (ioctl(master, TIOCGPTN, &index) || ioctl(master, TIOCGPTLCK, &locked) ||
      locked != 1)
    fail("DHI_PTY_PROVENANCE_UNVERIFIABLE");
  snprintf(path, sizeof(path), "/dev/pts/%u", index);
  if (chmod(path, 0))
    fail("DHI_PTY_CREATION_FENCE_FAILED");
  locked = 0;
  if (ioctl(master, TIOCSPTLCK, &locked))
    fail("DHI_PTY_UNLOCK_FAILED");
  keeper =
      ioctl(master, TIOCGPTPEER, O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC);
  if (keeper < 0)
    fail("DHI_PEER_FD_UNSUPPORTED");
  if (ioctl(keeper, TIOCEXCL) || fcntl(keeper, F_SETFL, O_NONBLOCK) ||
      socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, s))
    fail("DHI_FENCE_UNAVAILABLE");
  if (fchmod(keeper, 0600))
    fail("DHI_PTY_MODE_FAILED");
  if (fcntl(master, F_SETFD, FD_CLOEXEC) || fcntl(keeper, F_SETFD, FD_CLOEXEC))
    fail("DHI_CLOEXEC_FAILED");
  struct stat expected;
  if (stat(MANAGER, &expected))
    fail("DHI_MANAGER_UNAVAILABLE");
  struct seccomp_notif_sizes sizes = {0};
  if (syscall(SYS_seccomp, SECCOMP_GET_NOTIF_SIZES, 0, &sizes))
    fail("DHI_SECCOMP_UNSUPPORTED");
  pid_t parent = getpid(), child = fork();
  if (child < 0)
    fail("DHI_FORK_FAILED");
  if (!child) {
    close(s[0]);
    close(keeper);
    close(master);
    close(observer);
    if (prctl(PR_SET_PDEATHSIG, SIGKILL) || getppid() != parent)
      fail("DHI_GUARDIAN_PARENT_LOST");
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))
      fail("DHI_NO_NEW_PRIVILEGES_UNAVAILABLE");
    struct sock_filter code[] = {
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, arch)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, RX_ARCH, 1, 0),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_openat, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#ifdef __NR_open
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_open, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_openat2
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_openat2, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_clone
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_clone, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_clone3
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_clone3, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_fork
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_fork, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_vfork
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_vfork, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_open_by_handle_at
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_open_by_handle_at, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_io_uring_setup
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_io_uring_setup, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
#ifdef __NR_pidfd_getfd
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_pidfd_getfd, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
#endif
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, __NR_ioctl, 0, 3),
        BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
                 offsetof(struct seccomp_data, args[1])),
        BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, TIOCNXCL, 0, 1),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EPERM),
        BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)};
    struct sock_fprog p = {.len = sizeof(code) / sizeof(code[0]),
                           .filter = code};
    int listener = syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
                           SECCOMP_FILTER_FLAG_NEW_LISTENER, &p);
    if (listener < 0)
      fail("DHI_SECCOMP_LISTENER_UNSUPPORTED");
    pass(s[1], listener, "listener");
    close(listener);
    close(s[1]);
    if (fcntl(config, F_SETFD, 0) || dup2(STDERR_FILENO, STDOUT_FILENO) < 0)
      fail("DHI_EXEC_DESCRIPTOR_SETUP_FAILED");
    execl(MANAGER, MANAGER, "--ros-args", "--params-file", config_path, "-r",
          namespace_arg, NULL);
    fail("DHI_MANAGER_EXEC_FAILED");
  }
  close(s[1]);
  close(config);
  int pidfd = syscall(SYS_pidfd_open, child, 0);
  if (pidfd < 0) {
    kill(child, SIGKILL);
    fail("DHI_PIDFD_UNSUPPORTED");
  }
  int listener = take(s[0]);
  close(s[0]);
  pass(observer, master, path);
  close(master);
  char owner_label[80];
  snprintf(owner_label, sizeof(owner_label), "manager:%d", child);
  pass(observer, pidfd, owner_label);
  close(observer);
  struct sigaction sa = {.sa_handler = stop};
  sigaction(SIGINT, &sa, NULL);
  sigaction(SIGTERM, &sa, NULL);
  printf("{\"event\":\"custody\",\"guardian_pid\":%d,\"manager_pid\":%d,"
         "\"endpoint\":\"%s\",\"grant_budget\":2}\n",
         getpid(), child, path);
  fflush(stdout);
  struct seccomp_notif *q = calloc(1, sizes.seccomp_notif);
  struct seccomp_notif_resp *r = calloc(1, sizes.seccomp_notif_resp);
  if (!q || !r)
    fail("DHI_ALLOCATION_FAILED");
  struct stat custody;
  if (fstat(keeper, &custody) || !S_ISCHR(custody.st_mode))
    fail("DHI_CUSTODY_RESOURCE_UNVERIFIABLE");
  int grants = 0, refusals = 0, resource_refusals = 0, fork_refusals = 0,
      thread_clones = 0, status = 0, finished = 0, signaled = 0;
  while (!finished) {
    if (stopping && !signaled) {
      kill(child, SIGINT);
      signaled = 1;
    }
    struct pollfd polls[2] = {{listener, POLLIN, 0}, {pidfd, POLLIN, 0}};
    int available = poll(polls, 2, 100);
    if (available < 0 && errno != EINTR)
      fail("DHI_POLL_FAILED");
    if (available > 0 && (polls[0].revents & POLLIN)) {
      memset(q, 0, sizes.seccomp_notif);
      memset(r, 0, sizes.seccomp_notif_resp);
      if (ioctl(listener, SECCOMP_IOCTL_NOTIF_RECV, q)) {
        if (errno == ENOENT || errno == EINTR)
          continue;
        fail("DHI_NOTIFICATION_FAILED");
      }
      r->id = q->id;
      if (q->data.arch != RX_ARCH) {
        refuse(r, q, "DHI_SYSCALL_ARCH_UNSUPPORTED", NULL, NULL);
      }
#ifdef __NR_open_by_handle_at
      else if (q->data.nr == __NR_open_by_handle_at) {
        refuse(r, q, "DHI_OPEN_BY_HANDLE_UNSUPPORTED", NULL, NULL);
      }
#endif
#ifdef __NR_io_uring_setup
      else if (q->data.nr == __NR_io_uring_setup) {
        refuse(r, q, "DHI_IO_URING_UNSUPPORTED", NULL, NULL);
      }
#endif
#ifdef __NR_pidfd_getfd
      else if (q->data.nr == __NR_pidfd_getfd) {
        refuse(r, q, "DHI_PIDFD_IMPORT_UNSUPPORTED", NULL, NULL);
      }
#endif
      else if (
#ifdef __NR_clone
          q->data.nr == __NR_clone ||
#endif
#ifdef __NR_clone3
          q->data.nr == __NR_clone3 ||
#endif
#ifdef __NR_fork
          q->data.nr == __NR_fork ||
#endif
#ifdef __NR_vfork
          q->data.nr == __NR_vfork ||
#endif
          0) {
        const char *reason = NULL;
        int allow_thread = 0;
        if (tgid(q->pid) != child || !same_exe(q->pid, &expected))
          reason = "DHI_OWNER_INCARNATION_MISMATCH";
        else
          reason = clone_disposition(q, &allow_thread);
        if (reason) {
          refuse(r, q, reason, NULL, NULL);
          fork_refusals++;
        } else if (allow_thread) {
          r->flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE;
          thread_clones++;
        }
      } else {
        struct open_request requested_open;
        const char *reason = decode_open(q, &requested_open);
        char requested[4096] = {0}, resolved[8192] = {0};
        struct stat resource;
        if (reason) {
          refuse(r, q, reason, NULL, NULL);
        } else if (!read_remote_string(q->pid, requested_open.path_address,
                                       requested, sizeof(requested))) {
          refuse(r, q, "DHI_OPEN_PATH_UNREADABLE", NULL, NULL);
        } else if ((reason =
                        resource_path(q->pid, requested_open.dirfd, requested,
                                      resolved, sizeof(resolved)))) {
          refuse(r, q, reason, requested, NULL);
        } else if (stat(resolved, &resource)) {
          int classification_errno = errno;
          if (could_name_device(requested, requested_open.flags) ||
              (classification_errno != ENOENT &&
               classification_errno != ENOTDIR)) {
            errno = classification_errno;
            refuse(r, q, "DHI_RESOURCE_CLASSIFICATION_UNAVAILABLE", requested,
                   NULL);
          } else {
            r->flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE;
          }
        } else if (!S_ISCHR(resource.st_mode)) {
          r->flags = SECCOMP_USER_NOTIF_FLAG_CONTINUE;
        } else if (resource.st_rdev != custody.st_rdev) {
          refuse(r, q, "DHI_UNCUSTODIED_CHARACTER_RESOURCE", requested,
                 &resource);
          resource_refusals++;
        } else {
          if (tgid(q->pid) != child || !same_exe(q->pid, &expected))
            reason = "DHI_OWNER_INCARNATION_MISMATCH";
          else if (grants >= 2)
            reason = "DHI_REOPEN_BUDGET_EXCEEDED";
          else if ((requested_open.flags &
                    ~(unsigned long)(O_LARGEFILE | O_CLOEXEC)) !=
                   (O_RDWR | O_NOCTTY | O_NONBLOCK))
            reason = "DHI_OPEN_FLAGS_UNSUPPORTED";
          if (reason) {
            refuse(r, q, reason, requested, &resource);
          } else {
            if (ioctl(listener, SECCOMP_IOCTL_NOTIF_ID_VALID, &q->id)) {
              if (errno == ENOENT)
                continue;
              fail("DHI_NOTIFICATION_STALE");
            }
            int other = open(path, O_RDWR | O_NOCTTY | O_NONBLOCK);
            if (other >= 0) {
              close(other);
              kill(child, SIGKILL);
              fail("DHI_FENCE_BROKEN");
            }
            if (errno != EBUSY)
              fail("DHI_FENCE_UNVERIFIABLE");
            refusals++;
            struct seccomp_notif_addfd add = {
                .id = q->id,
                .flags = SECCOMP_ADDFD_FLAG_SEND,
                .srcfd = keeper,
                // The SDK remains in-process. Always closing the injected copy
                // on a later exec prevents an unrelated image inheriting it.
                .newfd_flags = O_CLOEXEC};
            int fd = ioctl(listener, SECCOMP_IOCTL_NOTIF_ADDFD, &add);
            if (fd < 0) {
              if (errno == ENOENT)
                continue;
              kill(child, SIGKILL);
              fail("DHI_FD_GRANT_UNSUPPORTED");
            }
            grants++;
            printf("{\"event\":\"grant\",\"syscall\":\"%s\","
                   "\"notification\":%llu,\"tid\":%u,\"tgid\":%d,"
                   "\"requested_path\":",
                   syscall_name(q->data.nr), (unsigned long long)q->id, q->pid,
                   child);
            print_json_string(requested);
            printf(",\"resource_major\":%u,\"resource_minor\":%u,"
                   "\"ordinal\":%d,\"receiver_fd\":%d,"
                   "\"outsider\":\"DHI_ENDPOINT_BUSY\"}\n",
                   major(resource.st_rdev), minor(resource.st_rdev), grants,
                   fd);
            fflush(stdout);
            continue;
          }
        }
      }
      if (ioctl(listener, SECCOMP_IOCTL_NOTIF_SEND, r) && errno != ENOENT)
        fail("DHI_RESPONSE_FAILED");
    }
    if (waitpid(child, &status, WNOHANG) == child)
      finished = 1;
  }
  printf("{\"event\":\"manager_exit\",\"grants\":%d,\"outsider_refusals\":%d,"
         "\"resource_refusals\":%d,\"fork_refusals\":%d,"
         "\"thread_clones_allowed\":%d,\"wait_status\":%d,"
         "\"stop_effect\":\"UNCONFIRMED\"}\n",
         grants, refusals, resource_refusals, fork_refusals, thread_clones,
         status);
  fflush(stdout);
  close(listener);
  close(pidfd);
  close(keeper);
  free(q);
  free(r);
  return !WIFEXITED(status) || WEXITSTATUS(status) != 0 || grants != 2 ||
         refusals != 2;
}
