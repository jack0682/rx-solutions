"""Run browser checks with owned local servers and clean up their process groups."""
import argparse
import os
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import time


def port_open(port):
    with socket.socket() as connection:
        connection.settimeout(0.2)
        return connection.connect_ex(("127.0.0.1", port)) == 0


def stop(process):
    if os.name == "posix":
        # Descendants may still be alive after the server launcher exits.
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            process.poll()
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.05)
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    elif process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
    process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", action="append", required=True)
    parser.add_argument("--port", action="append", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=30,
                        help="Readiness timeout in seconds for each server")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if len(args.server) != len(args.port) or not command:
        parser.error("provide one --port per --server and a command after --")
    if args.timeout <= 0 or any(not 0 < port < 65536 for port in args.port):
        parser.error("timeout must be positive and ports must be within 1..65535")
    if len(set(args.port)) != len(args.port):
        parser.error("server ports must be distinct")
    for port in args.port:
        if port_open(port):
            parser.error(f"port {port} is occupied; existing processes were not touched")

    servers = []
    check = None
    status = 1
    try:
        for invocation, port in zip(args.server, args.port):
            arguments = shlex.split(invocation)
            if not arguments:
                raise ValueError("server command is empty")
            log = tempfile.TemporaryFile(mode="w+b")
            try:
                process = subprocess.Popen(arguments, stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
            except BaseException:
                log.close()
                raise
            servers.append((process, log, port))
            deadline = time.monotonic() + args.timeout
            while True:
                if process.poll() is not None:
                    raise RuntimeError(f"server for port {port} exited with {process.returncode}")
                if port_open(port):
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"server for port {port} did not become ready")
                time.sleep(0.05)
        check = subprocess.Popen(command, start_new_session=True)
        status = check.wait()
    except KeyboardInterrupt:
        status = 130
    except (OSError, ValueError, RuntimeError) as error:
        print(str(error), file=sys.stderr)
    finally:
        if check is not None:
            stop(check)
        for process, log, port in reversed(servers):
            stop(process)
            if status:
                log.seek(0, os.SEEK_END)
                log.seek(max(0, log.tell() - 8000))
                print(f"Server log for port {port}:\n{log.read().decode(errors='replace')}",
                      file=sys.stderr)
            log.close()
    return status if status >= 0 else 128 - status


if __name__ == "__main__":
    raise SystemExit(main())
