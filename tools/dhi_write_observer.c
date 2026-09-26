/* Independent Linux ptrace observation of the installed guardian only.
 * Other processes retain their real parent/child relationships. */
#define _GNU_SOURCE
#include <errno.h>
#include <linux/ptrace.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static long trace(long op, pid_t pid, void *address, void *data) {
  return syscall(SYS_ptrace, op, pid, address, data);
}
struct task {
  pid_t pid;
  int guardian;
} tasks[512];
static int count;
static struct task *task(pid_t pid) {
  for (int i = 0; i < count; i++)
    if (tasks[i].pid == pid)
      return &tasks[i];
  if (count == 512)
    exit(91);
  tasks[count].pid = pid;
  return &tasks[count++];
}
static int is_guardian(pid_t pid) {
  char name[80], target[512];
  snprintf(name, sizeof(name), "/proc/%d/exe", pid);
  ssize_t n = readlink(name, target, sizeof(target) - 1);
  if (n < 0)
    return 0;
  target[n] = 0;
  return !strcmp(target, "/opt/rx/bin/rx-dhi-custody");
}
int main(int argc, char **argv) {
  if (argc < 2)
    return 90;
  pid_t root = fork();
  if (root < 0)
    return 91;
  if (!root) {
    if (trace(PTRACE_TRACEME, 0, 0, 0))
      _exit(92);
    raise(SIGSTOP);
    execv(argv[1], argv + 1);
    _exit(93);
  }
  int status;
  if (waitpid(root, &status, 0) != root || !WIFSTOPPED(status))
    return 94;
  long options = PTRACE_O_TRACESYSGOOD | PTRACE_O_EXITKILL |
                 PTRACE_O_TRACEEXEC | PTRACE_O_TRACEFORK | PTRACE_O_TRACEVFORK |
                 PTRACE_O_TRACECLONE;
  if (trace(PTRACE_SETOPTIONS, root, 0, (void *)options) < 0)
    return 95;
  task(root);
  if (trace(PTRACE_CONT, root, 0, 0) < 0)
    return 96;
  unsigned long long entries = 0, writes = 0;
  int root_status = -1, guardians = 0;
  for (;;) {
    pid_t pid = waitpid(-1, &status, __WALL);
    if (pid < 0) {
      if (errno == EINTR)
        continue;
      if (errno == ECHILD)
        break;
      return 97;
    }
    struct task *current = task(pid);
    if (WIFEXITED(status) || WIFSIGNALED(status)) {
      if (pid == root)
        root_status = status;
      continue;
    }
    int sig = WSTOPSIG(status);
    unsigned event = (unsigned)status >> 16;
    if (event == PTRACE_EVENT_EXEC) {
      current->guardian = is_guardian(pid);
      if (current->guardian)
        guardians++;
      sig = 0;
    } else if (event == PTRACE_EVENT_FORK || event == PTRACE_EVENT_VFORK ||
               event == PTRACE_EVENT_CLONE) {
      unsigned long child = 0;
      if (trace(PTRACE_GETEVENTMSG, pid, 0, &child) < 0)
        return 98;
      task((pid_t)child);
      sig = 0;
    } else if (sig == (SIGTRAP | 0x80)) {
      struct ptrace_syscall_info info = {0};
      if (trace(PTRACE_GET_SYSCALL_INFO, pid, (void *)sizeof(info), &info) < 0)
        return 99;
      if (current->guardian && info.op == PTRACE_SYSCALL_INFO_ENTRY) {
        entries++;
        long nr = info.entry.nr;
        int fd = -1;
        if (nr == SYS_write || nr == SYS_writev || nr == SYS_pwrite64 ||
            nr == SYS_pwritev || nr == SYS_sendfile)
          fd = info.entry.args[0];
        if (nr == SYS_splice || nr == SYS_copy_file_range)
          fd = info.entry.args[2];
        if (fd >= 0) {
          char path[80], target[256];
          snprintf(path, sizeof(path), "/proc/%d/fd/%d", pid, fd);
          ssize_t n = readlink(path, target, sizeof(target) - 1);
          if (n >= 0) {
            target[n] = 0;
            if (!strncmp(target, "/dev/pts/", 9)) {
              writes++;
              fprintf(stderr,
                      "{\"observer\":\"ptrace\",\"event\":\"GUARDIAN_TERMINAL_"
                      "WRITE\",\"pid\":%d,\"syscall\":%ld,\"fd\":%d,\"target\":"
                      "\"%s\"}\n",
                      pid, nr, fd, target);
            }
          }
        }
      }
      sig = 0;
    } else if (sig == SIGSTOP || sig == SIGTRAP)
      sig = 0;
    if (trace(current->guardian ? PTRACE_SYSCALL : PTRACE_CONT, pid, 0,
              (void *)(long)sig) < 0 &&
        errno != ESRCH)
      return 100;
  }
  fprintf(
      stderr,
      "{\"observer\":\"ptrace\",\"guardians\":%d,\"guardian_syscall_entries\":%"
      "llu,\"terminal_write_attempts\":%llu,\"root_wait_status\":%d}\n",
      guardians, entries, writes, root_status);
  if (writes || guardians != 1 || !entries)
    return 42;
  return root_status >= 0 && WIFEXITED(root_status) ? WEXITSTATUS(root_status)
                                                    : 101;
}
