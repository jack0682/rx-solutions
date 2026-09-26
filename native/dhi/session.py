#!/usr/bin/env python3
"""Release-owned DHI PTY simulation; never a physical work permission provider."""
import argparse
import array
import errno
import hashlib
import json
import os
from pathlib import Path
import platform
import select
import signal
import socket
import subprocess
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT = Path("/opt/rx")
OVERLAY = ROOT / "dhi"
GUARDIAN = ROOT / "bin/rx-dhi-custody"
MANAGER = ROOT / "bin/rx-dhi-controller-manager"
PROFILE = "rx.dhi-fresh-pty-simulation.v1"
EXPECTED_MODEL = 1060
EXPECTED_FIRMWARE = 44
OUTPUT_LOCK = threading.Lock()


def emit(row):
    row = dict(
        row,
        schema="rx.dhi-observation.v1",
        profile=PROFILE,
        supervisor_instance=os.environ.get("RX_PROCESS_INSTANCE_ID"),
        recorded_at_ns=time.monotonic_ns(),
        physical_qualification="NOT_PERFORMED",
    )
    with OUTPUT_LOCK:
        print(json.dumps(row, sort_keys=True), flush=True)


class Refusal(Exception):
    pass


def environment():
    if sys.platform != "linux" or platform.machine() not in ("aarch64", "x86_64"):
        raise Refusal("DHI_PLATFORM_UNSUPPORTED")
    try:
        instance = str(uuid.UUID(os.environ["RX_PROCESS_INSTANCE_ID"]))
        if instance != os.environ["RX_PROCESS_INSTANCE_ID"]:
            raise ValueError()
    except (KeyError, ValueError):
        raise Refusal("DHI_INSTANCE_REQUIRED") from None
    fixed = {
        "PATH": "/usr/bin:/bin",
        "LANG": "C.UTF-8",
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONPATH": "/opt/rx/dhi/lib/python3.12/site-packages:/opt/ros/jazzy/lib/python3.12/site-packages:/opt/ros/jazzy/local/lib/python3.12/dist-packages",
        "LD_LIBRARY_PATH": "/opt/rx/dhi/lib:/opt/ros/jazzy/lib:/opt/ros/jazzy/lib/aarch64-linux-gnu:/opt/ros/jazzy/lib/x86_64-linux-gnu:/opt/ros/jazzy/opt/sdformat_vendor/lib:/opt/ros/jazzy/opt/gz_math_vendor/lib:/opt/ros/jazzy/opt/gz_utils_vendor/lib:/opt/ros/jazzy/opt/gz_tools_vendor/lib:/opt/ros/jazzy/opt/gz_cmake_vendor/lib",
        "AMENT_PREFIX_PATH": "/opt/rx/dhi:/opt/ros/jazzy",
        "RMW_IMPLEMENTATION": "rmw_fastrtps_cpp",
        "ROS_DISTRO": "jazzy",
        "ROS_LOCALHOST_ONLY": "1",
        "ROS_DOMAIN_ID": "106",
        "ROS_LOG_DIR": "/run/rx-solutions/dhi-" + instance + "/ros-log",
        "RX_PROCESS_INSTANCE_ID": instance,
    }
    # Re-exec is required: the ELF loader reads LD_LIBRARY_PATH at process start.
    # No caller-supplied library path, preload, ROS endpoint, or model path survives.
    if dict(os.environ) != fixed:
        os.execve(
            "/usr/bin/python3",
            ["/usr/bin/python3", str(Path(__file__).resolve()), *sys.argv[1:]],
            fixed,
        )
    return instance


def installed_content():
    try:
        inventory = json.loads((ROOT / "manifests/runtime-files.json").read_text())
        pins = json.loads((ROOT / "tools/dhi/dependencies.json").read_text())
    except (OSError, ValueError):
        raise Refusal("DHI_RELEASE_METADATA_UNAVAILABLE") from None
    for relative in (
        "bin/rx-dhi-custody",
        "tools/dhi/session.py",
        "tools/dhi/model.py",
        "tools/dhi/dependencies.json",
        "tools/dhi/endpoint-channels.json",
    ):
        try:
            actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        except OSError:
            raise Refusal("release/content-mismatch: " + relative) from None
        if inventory["files"].get(relative) != actual:
            raise Refusal("release/content-mismatch: " + relative)
    try:
        manager_hash = hashlib.sha256(MANAGER.read_bytes()).hexdigest()
        if inventory["files"].get("bin/rx-dhi-controller-manager") != manager_hash:
            raise Refusal("release/content-mismatch: controller_manager")
        source_lock = json.loads((ROOT / "manifests/dhi-source-lock.json").read_text())
        if source_lock.get("sources") != pins["sources"]:
            raise Refusal("DHI_DEPENDENCY_PIN_MISMATCH")
        version = subprocess.check_output(
            [
                "/usr/bin/dpkg-query",
                "-W",
                "-f=${Version}",
                "ros-jazzy-controller-manager",
            ],
            text=True,
        ).strip()
        if version != pins["controller_manager"]["debian_version"]:
            raise Refusal("DHI_MANAGER_VERSION_UNSUPPORTED")
    except (OSError, ValueError, subprocess.CalledProcessError):
        raise Refusal("DHI_DEPENDENCY_PROVENANCE_UNAVAILABLE") from None
    expected = pins["model_assets"]
    for relative, digest in expected.items():
        path = OVERLAY / relative
        try:
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError:
            raise Refusal("DHI_FIRMWARE_MODEL_FILE_UNAVAILABLE") from None
        if actual != digest:
            raise Refusal("DHI_FIRMWARE_MODEL_FILE_MISMATCH")
    if set(pins["sources"]) != {
        "dynamixel_hardware_interface",
        "dynamixel_sdk",
        "dynamixel_interfaces",
    }:
        raise Refusal("DHI_DEPENDENCY_PIN_MISMATCH")
    return pins


def receive_fd(channel):
    data, ancillary, flags, _ = channel.recvmsg(128, socket.CMSG_SPACE(4))
    if (
        flags
        or len(ancillary) != 1
        or ancillary[0][:2] != (socket.SOL_SOCKET, socket.SCM_RIGHTS)
    ):
        raise Refusal("DHI_CUSTODY_CHANNEL_INVALID")
    fds = array.array("i")
    fds.frombytes(ancillary[0][2])
    if len(fds) != 1:
        for fd in fds:
            os.close(fd)
        raise Refusal("DHI_CUSTODY_CHANNEL_INVALID")
    os.set_inheritable(fds[0], False)
    return data.rstrip(b"\0").decode("ascii"), fds[0]


class Session:
    def __init__(self, instance):
        from model import Model
        import rclpy
        from rclpy.qos import QoSProfile, DurabilityPolicy, ReliabilityPolicy
        from std_msgs.msg import String
        from controller_manager_msgs.srv import (
            ListHardwareComponents,
            SetHardwareComponentState,
        )
        from std_srvs.srv import SetBool

        self.instance = instance
        self.guardian = None
        self.model = None
        self.manager_fd = None
        self.node = None
        self.phase = "STARTING"
        self.loss = None
        self.loss_emitted = False
        self.grants = []
        self.events = []
        self.event_lock = threading.Lock()
        self.rclpy = rclpy
        self.String = String
        self.List = ListHardwareComponents
        self.SetState = SetHardwareComponentState
        self.SetBool = SetBool
        self.namespace = "/rx_dhi_" + instance.replace("-", "_")
        self.model_files = installed_content()["model_assets"]
        parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        try:
            parent.settimeout(5)
            self.guardian = subprocess.Popen(
                [str(GUARDIAN), str(child.fileno())],
                pass_fds=(child.fileno(),),
                stdout=subprocess.PIPE,
                stderr=None,
                text=True,
            )
            child.close()
            self.path, master = receive_fd(parent)
            if (
                not self.path.startswith("/dev/pts/")
                or not self.path.removeprefix("/dev/pts/").isdigit()
            ):
                os.close(master)
                raise Refusal("DHI_ENDPOINT_PROVENANCE_UNVERIFIABLE")
            self.model = Model(master, emit)
            label, self.manager_fd = receive_fd(parent)
            if not label.startswith("manager:"):
                raise Refusal("DHI_MANAGER_HANDLE_INVALID")
            self.manager_pid = int(label.split(":")[1])
        finally:
            parent.close()
            child.close()
        self.reader = threading.Thread(target=self.read_guardian, daemon=True)
        self.reader.start()
        from rclpy.signals import SignalHandlerOptions

        rclpy.init(signal_handler_options=SignalHandlerOptions.NO)
        self.node = rclpy.create_node("rx_dhi_session", namespace=self.namespace)
        qos = QoSProfile(
            depth=1,
            durability=DurabilityPolicy.TRANSIENT_LOCAL,
            reliability=ReliabilityPolicy.RELIABLE,
        )
        self.publisher = self.node.create_publisher(String, "robot_description", qos)
        self.listing = self.node.create_client(
            ListHardwareComponents,
            self.namespace + "/controller_manager/list_hardware_components",
        )
        self.lifecycle = self.node.create_client(
            SetHardwareComponentState,
            self.namespace + "/controller_manager/set_hardware_component_state",
        )
        self.torque_client = self.node.create_client(
            SetBool, self.namespace + "/dynamixel_hardware_interface/set_dxl_torque"
        )
        self.urdf = self.description()
        self.publisher.publish(String(data=self.urdf))
        end = time.monotonic() + 20
        while time.monotonic() < end:
            self.check_live()
            if self.listing.wait_for_service(timeout_sec=0.2):
                reply = self.call(self.listing, ListHardwareComponents.Request())
                if reply.component and reply.component[0].state.id == 2:
                    break
            rclpy.spin_once(self.node, timeout_sec=0.02)
        else:
            raise Refusal("DHI_MANAGER_INITIALIZATION_UNCONFIRMED")
        self.check_model()
        if len(self.grants) != 2:
            raise Refusal("DHI_EXPECTED_GRANTS_NOT_OBSERVED")
        self.phase = "INACTIVE"
        emit({"event": "session_ready", "status": self.status()})

    def description(self):
        return f"""<robot name="rx_dhi_model"><link name="base"/><link name="tip"/><joint name="joint1" type="revolute"><parent link="base"/><child link="tip"/><axis xyz="0 0 1"/><limit lower="-3.14" upper="3.14" effort="1" velocity="1"/></joint><ros2_control name="DxlHardware" type="system"><hardware><plugin>dynamixel_hardware_interface/DynamixelHardware</plugin><param name="port_name">{self.path}</param><param name="baud_rate">57600</param><param name="number_of_joints">1</param><param name="number_of_transmissions">1</param><param name="dynamixel_model_folder">/param/dxl_model</param></hardware><joint name="joint1"><command_interface name="position"/><state_interface name="position"/></joint><gpio name="dxl1"><param name="type">dxl</param><param name="ID">1</param><command_interface name="Goal Position"/><state_interface name="Present Position"/><state_interface name="Hardware Error Status"/></gpio></ros2_control></robot>"""

    def read_guardian(self):
        try:
            for line in self.guardian.stdout:
                if len(line) > 8192:
                    raise Refusal("DHI_CUSTODY_EVENT_TOO_LARGE")
                row = json.loads(line)
                with self.event_lock:
                    if row.get("event") == "grant":
                        self.grants.append(row)
                    self.events.append(row)
                    self.events = self.events[-64:]
                emit({"event": "custody_observation", "observation": row})
        except Exception as error:
            self.loss = "DHI_CUSTODY_OBSERVATION_LOST: " + str(error)

    def check_model(self):
        snapshot = self.model.snapshot()
        identity = snapshot["identity"]
        if identity and identity["model"] != EXPECTED_MODEL:
            raise Refusal("DHI_MODEL_MISMATCH")
        if identity and identity["firmware"] != EXPECTED_FIRMWARE:
            raise Refusal("DHI_FIRMWARE_UNSUPPORTED")
        if snapshot["error"]:
            raise Refusal("DHI_MODEL_PROTOCOL_FAILED: " + snapshot["error"])
        if identity is None:
            raise Refusal("DHI_MODEL_IDENTITY_UNOBSERVED")

    def check_live(self):
        if (
            self.loss
            or self.guardian.poll() is not None
            or select.select([self.manager_fd], [], [], 0)[0]
        ):
            self.loss = self.loss or "DHI_GUARDIAN_OR_MANAGER_LOST"
            self.phase = "ATTENTION"
            raise Refusal(self.loss)
        snapshot = self.model.snapshot()
        if snapshot["identity"] is not None:
            self.check_model()

    def call(self, client, request):
        if not client.wait_for_service(timeout_sec=2):
            raise Refusal("DHI_ROS_REQUEST_UNAVAILABLE")
        future = client.call_async(request)
        self.rclpy.spin_until_future_complete(self.node, future, timeout_sec=3)
        if not future.done() or future.exception():
            raise Refusal("DHI_ROS_REQUEST_RESULT_UNKNOWN")
        return future.result()

    def command(self, request):
        action = request.get("action")
        if action in ("adopt", "claim", "replace-owner"):
            raise Refusal("DHI_OWNER_CLAIM_UNATTESTED")
        if action == "restart":
            raise Refusal("DHI_PREVIOUS_ENDPOINT_RETAINED")
        if action not in ("torque", "activate", "deactivate", "stop"):
            raise Refusal("DHI_DIAGNOSTIC_ACTION_UNSUPPORTED")
        expected = {"action", "enable"} if action == "torque" else {"action"}
        if set(request) != expected or (
            action == "torque" and type(request["enable"]) is not bool
        ):
            raise Refusal("DHI_DIAGNOSTIC_ARGUMENTS_INVALID")
        self.check_live()
        emit(
            {
                "event": "diagnostic_request",
                "request": request,
                "work_permission": "NOT_GRANTED",
            }
        )
        if action == "torque":
            reply = self.call(
                self.torque_client, self.SetBool.Request(data=request["enable"])
            )
            accepted, message = reply.success, reply.message
        else:
            req = self.SetState.Request()
            req.name = "DxlHardware"
            req.target_state.id = 3 if action == "activate" else 2
            reply = self.call(self.lifecycle, req)
            accepted, message = reply.ok, reply.state.label
            if reply.ok:
                self.phase = "ACTIVE" if action == "activate" else "INACTIVE"
        result = {
            "request_accepted": accepted,
            "component_message": message,
            "observed_model": self.model.snapshot(),
            "stop_effect": "UNCONFIRMED",
            "physical_qualification": "NOT_PERFORMED",
            "work_permission": "NOT_GRANTED",
        }
        emit({"event": "diagnostic_response", "request": request, "result": result})
        return result

    def status(self):
        if self.guardian and self.manager_fd is not None:
            try:
                self.check_live()
            except Refusal as error:
                self.loss = str(error)
        return {
            "schema": "rx.solutions-status.v1",
            "phase": (
                "SOFTWARE_READY_UNCOMMISSIONED"
                if self.phase in ("INACTIVE", "ACTIVE") and not self.loss
                else "ATTENTION"
            ),
            "component_phase": self.phase,
            "supervisor_instance": self.instance,
            "profile": PROFILE,
            "endpoint": self.path,
            "guardian_pid": self.guardian.pid,
            "manager_pid": self.manager_pid,
            "grants": list(self.grants),
            "refusal": self.loss,
            "model": self.model.snapshot(),
            "stop_effect": "UNCONFIRMED",
            "owned_process_exit_is_not_model_stop": True,
            "same_uid_tampering": "OUTSIDE_TRUST_BOUNDARY_CHMOD_PTRACE",
            "effect_class_scope": "NONACTUATING_FRESH_PTY_MODEL_ONLY",
            "physical_qualification": "NOT_PERFORMED",
            "work_permission": "NOT_GRANTED",
            "sdk_baseline_complete": False,
            "robotis_bundle_complete": False,
            "compose_s6_reuse": "NOT_ESTABLISHED",
            "ros_admin_authority": "NOT_ESTABLISHED",
        }

    def close(self):
        # The native guardian requests cooperative CM shutdown. No exit or
        # unchecked upstream stop response erases the separately logged residual.
        if self.guardian and self.guardian.poll() is None:
            self.guardian.send_signal(signal.SIGTERM)
            try:
                self.guardian.wait(timeout=8)
            except subprocess.TimeoutExpired:
                emit(
                    {
                        "event": "stop_unconfirmed",
                        "reason": "DHI_MANAGER_STOP_UNCONFIRMED",
                        "residual": self.model.snapshot() if self.model else None,
                    }
                )
                self.guardian.kill()
                self.guardian.wait()
        if self.model:
            emit(
                {
                    "event": "session_exit",
                    "guardian_returncode": self.guardian.returncode,
                    "residual": self.model.snapshot(),
                    "stop_effect": "UNCONFIRMED",
                }
            )
            self.model.close()
        if self.manager_fd is not None:
            os.close(self.manager_fd)
        if self.node:
            self.node.destroy_node()
            self.rclpy.shutdown()


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise Refusal("DHI_ARGUMENT_UNSUPPORTED: " + message)


def main():
    instance = environment()
    parser = Parser()
    parser.add_argument("mode", choices=["serve"])
    parser.add_argument("--port", required=True, type=int)
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535:
        raise Refusal("DHI_PORT_INVALID")
    session = None
    stopping = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stopping.set())
    signal.signal(signal.SIGINT, lambda *_: stopping.set())
    try:
        session = Session.__new__(Session)
        session.__init__(instance)

        class Handler(BaseHTTPRequestHandler):
            def setup(self):
                super().setup()
                self.connection.settimeout(3)

            def log_message(self, *_):
                pass

            def send_json(self, code, value):
                data = json.dumps(value).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                if self.path not in ("/health", "/status"):
                    return self.send_json(404, {"refusal": "DHI_ROUTE_UNSUPPORTED"})
                return self.send_json(200, session.status())

            def do_POST(self):
                try:
                    if self.path != "/diagnostic":
                        raise Refusal("DHI_ROUTE_UNSUPPORTED")
                    size = int(self.headers.get("Content-Length", "0"))
                    if size < 1 or size > 2048:
                        raise Refusal("DHI_REQUEST_SIZE_INVALID")

                    def strict(pairs):
                        result = {}
                        for key, value in pairs:
                            if key in result:
                                raise Refusal("DHI_DUPLICATE_FIELD")
                            result[key] = value
                        return result

                    request = json.loads(
                        self.rfile.read(size), object_pairs_hook=strict
                    )
                    if not isinstance(request, dict):
                        raise Refusal("DHI_REQUEST_INVALID")
                    result = session.command(request)
                    return self.send_json(200, result)
                except (Refusal, ValueError, OSError) as error:
                    emit({"event": "diagnostic_refused", "reason": str(error)})
                    return self.send_json(
                        409, {"refusal": str(error), "stop_effect": "UNCONFIRMED"}
                    )

        with HTTPServer(("127.0.0.1", args.port), Handler) as server:
            server.timeout = 0.1
            while not stopping.is_set():
                server.handle_request()
                if session.guardian.poll() is not None and not session.loss_emitted:
                    emit({"event": "custody_lost", "status": session.status()})
                    session.loss_emitted = True
    finally:
        if session is not None and hasattr(session, "guardian"):
            session.close()


if __name__ == "__main__":
    try:
        main()
    except (Refusal, OSError) as error:
        emit(
            {
                "event": "startup_refused",
                "reason": str(error),
                "stop_effect": "UNCONFIRMED",
            }
        )
        raise SystemExit(1)
