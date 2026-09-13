"""Lifecycle regression checks for the local browser-test server runner."""
import shlex
import socket
import subprocess
import sys
import unittest
from pathlib import Path

RUNNER = Path(__file__).with_name("with_servers.py")


def unused_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def connected(port):
    with socket.socket() as connection:
        connection.settimeout(0.2)
        return connection.connect_ex(("127.0.0.1", port)) == 0


class ServerLifecycleTests(unittest.TestCase):
    def invoke(self, port, check, server=None):
        server = server or [sys.executable, "-c",
                            "import socket, threading; s=socket.socket(); "
                            f"s.bind(('127.0.0.1', {port})); s.listen(); threading.Event().wait()"]
        return subprocess.run(
            [sys.executable, str(RUNNER), "--server", shlex.join(server), "--port", str(port),
             "--timeout", "3", "--", sys.executable, "-c", check],
            capture_output=True, text=True, timeout=15,
        )

    def test_success_waits_for_readiness_and_cleans_up(self):
        port = unused_port()
        result = self.invoke(port, f"import socket; socket.create_connection(('127.0.0.1', {port})).close()")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(connected(port))

    def test_failed_check_preserves_exit_status_and_cleans_up(self):
        port = unused_port()
        result = self.invoke(port, "raise SystemExit(7)")
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertFalse(connected(port))

    def test_occupied_port_preserves_existing_listener(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            result = self.invoke(port, "raise SystemExit(0)")
            self.assertEqual(result.returncode, 2)
            self.assertIn("existing processes were not touched", result.stderr)
            self.assertTrue(connected(port))

    def test_failed_start_does_not_run_the_check(self):
        result = self.invoke(unused_port(), "print('CHECK_EXECUTED')",
                             [sys.executable, "-c", "print('startup failure'); raise SystemExit(3)"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("startup failure", result.stderr)
        self.assertNotIn("CHECK_EXECUTED", result.stdout)


if __name__ == "__main__":
    unittest.main()
