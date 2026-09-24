"""Private, embedded exec gate. It neither applies policy nor issues receipts.

The parent owns the connected socket and observes/applies policy before EXEC.
The final process replaces this same PID. There is no listener or installed
helper service. The selected release interpreter is already hash-verified.
"""
import json
import os
import sys

control = os.dup(1)
os.set_inheritable(control, False)
try:
    request = json.loads(sys.argv[1])
    output = os.open(request["stdout"], os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    null = os.open("/dev/null", os.O_RDONLY)
    os.write(control, b"GATED\n")
    if sys.stdin.buffer.readline(6) != b"EXEC\n":
        os._exit(125)
    os.dup2(null, 0)
    os.dup2(output, 1)
    os.close(null)
    os.close(output)
    try:
        os.execv(request["executable"], [request["executable"], *request["arguments"]])
    except OSError as error:
        os.write(control, ("EXEC_ERROR:" + str(error.errno) + "\n").encode("ascii"))
        os._exit(126)
except BaseException:
    # No success can be inferred from EOF: the parent independently checks the
    # final argv, limits and owned child after exec before issuing a receipt.
    os._exit(125)
