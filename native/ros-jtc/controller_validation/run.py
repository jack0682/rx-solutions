#!/usr/bin/env python3
"""Exercise the unmodified RX bridge against official JTC + GenericSystem mock hardware.

No ActionServer is implemented here. The publisher supplies only a nonphysical URDF;
the subscriptions observe controller messages. No Rust Host Authority is supplied.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import threading
import time
import uuid
from datetime import datetime, timezone

import rclpy
from rclpy.executors import SingleThreadedExecutor
from rclpy.qos import DurabilityPolicy, QoSProfile
from control_msgs.action import FollowJointTrajectory
from control_msgs.msg import JointTrajectoryControllerState
from controller_manager_msgs.srv import ListControllers
from rosidl_runtime_py.convert import message_to_ordereddict
from std_msgs.msg import String

ROOT = Path(__file__).resolve().parent
WORK = Path('/tmp/n5-jtc')
CATALOG = Path('/opt/rx/catalogs/device-support.v1.json')
BINARY = Path('/opt/rx/bin/rx-ros-jtc-bridge')
MANAGER = '/rx_test/controller_manager'
ACTION = '/rx_test/arm_controller/follow_joint_trajectory'
JOINTS = [f'joint{i}' for i in range(1, 7)]
os.environ.update(RMW_IMPLEMENTATION='rmw_fastrtps_cpp',
                  ROS_AUTOMATIC_DISCOVERY_RANGE='LOCALHOST', ROS_DOMAIN_ID='171',
                  ROS_LOG_DIR='/tmp/n5-ros-log', RCUTILS_LOGGING_USE_STDOUT='0')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(args):
    result = subprocess.run(args, capture_output=True, text=True, timeout=40)
    record = {'argv': args, 'returncode': result.returncode,
              'stdout': result.stdout, 'stderr': result.stderr}
    commands.append(record)
    assert result.returncode == 0, record
    return result.stdout


class Bridge:
    def __init__(self, label):
        self.label = label
        self.log = (WORK / f'{label}.stderr').open('w+')
        self.process = subprocess.Popen([str(BINARY), str(WORK / 'bridge.json')],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.log, text=True)
        self.sequence = 0
        self.hello = self.read()
        assert self.hello['state'] == 'READY', self.hello
        self.instance = self.hello['bridge_instance']
        exchanges.append({'bridge': label, 'hello': self.hello})

    def read(self):
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            assert selector.select(8), 'bridge response timeout'
            line = self.process.stdout.readline()
        assert line, (self.label, self.process.poll(), (WORK / f'{self.label}.stderr').read_text())
        return json.loads(line)

    def call(self, operation, body):
        self.sequence += 1
        request = {'schema': 'rx.ros-jtc-request.v1', 'bridge_instance': self.instance,
                   'sequence': str(self.sequence), 'expires_at_ns': str(
                       time.clock_gettime_ns(time.CLOCK_BOOTTIME) + 900_000_000),
                   'command': operation, 'body': body}
        self.process.stdin.write(json.dumps(request) + '\n')
        self.process.stdin.flush()
        response = self.read()
        exchanges.append({'bridge': self.label, 'request': request, 'response': response})
        return response

    def ready(self):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            response = self.call('inspect', {})
            if response['state'] == 'OBSERVED' and all(response['value'][key] for key in
                    ('send_service_ready', 'result_service_ready', 'cancel_service_ready')):
                assert response['value']['controller_generation_known'] is False
                assert response['value']['physical_readiness_proven'] is False
                return response
            time.sleep(.1)
        raise AssertionError(response)

    def result(self, token):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            response = self.call('result', {'invocation': token})
            if response['state'] == 'RESULT_CAPTURED':
                assert response['value']['goal_id'] == token
                return response
            assert response['state'] in ('RPC_UNKNOWN', 'RESULT_UNKNOWN'), response
            time.sleep(.05)
        raise AssertionError(response)

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()  # Not a goal-cancel or physical stop command.
            self.process.wait(timeout=5)
        assert self.process.returncode == 0
        self.log.close()


commands, exchanges, feedback, controller_states = [], [], [], []
manager = None
bridges = []
executor = None
node = None
thread = None
WORK.mkdir()
try:
    catalog = json.loads(CATALOG.read_text())
    selected = next(p for p in catalog['profiles'] if p['support_id'] == 'SIM-JTC-6DOF')
    assert selected['evidence_level'] == 'SIMULATION_FIXTURE'
    controller = next(c for c in selected['controllers'] if c['name'] == 'arm_controller')
    assert controller['joint_order'] == JOINTS
    config = {'schema': 'rx.ros-jtc-bridge.v1', 'catalog_sha256': digest(CATALOG),
              'support_id': selected['support_id'], 'controller': controller['name'],
              'namespace': '/rx_test', 'controller_manager': MANAGER, 'domain_id': 171,
              'timeout_ms': 150, 'capacity': 32}
    (WORK / 'bridge.json').write_text(json.dumps(config))
    description = (ROOT / 'mock.urdf').read_text()
    rclpy.init(domain_id=171)
    node = rclpy.create_node('rx_n5_description_and_observer')
    qos = QoSProfile(depth=1, durability=DurabilityPolicy.TRANSIENT_LOCAL)
    publisher = node.create_publisher(String, '/rx_test/robot_description', qos)
    # Transient-local history supplies this one immutable description to late subscribers.
    publisher.publish(String(data=description))

    def observe_feedback(message):
        feedback.append({'received_boottime_ns': str(time.clock_gettime_ns(time.CLOCK_BOOTTIME)),
                         'invocation': str(uuid.UUID(bytes=bytes(message.goal_id.uuid))),
                         'message': message_to_ordereddict(message.feedback)})
        if len(feedback) > 100:
            del feedback[0]

    def observe_state(message):
        controller_states.append({'received_boottime_ns': str(time.clock_gettime_ns(time.CLOCK_BOOTTIME)),
                                  'message': message_to_ordereddict(message)})
        if len(controller_states) > 20:
            del controller_states[0]

    node.create_subscription(FollowJointTrajectory.Impl.FeedbackMessage,
                             ACTION + '/_action/feedback', observe_feedback, 10)
    node.create_subscription(JointTrajectoryControllerState,
                             '/rx_test/arm_controller/controller_state', observe_state, 10)
    executor = SingleThreadedExecutor()
    executor.add_node(node)
    thread = threading.Thread(target=executor.spin, daemon=True)
    thread.start()
    manager_argv = ['/opt/ros/jazzy/lib/controller_manager/ros2_control_node', '--ros-args',
                    '-r', '__ns:=/rx_test', '--params-file', str(ROOT / 'controllers.yaml')]
    manager_log = (WORK / 'controller-manager.log').open('w+')
    manager = subprocess.Popen(manager_argv, stdout=manager_log, stderr=subprocess.STDOUT)
    client = node.create_client(ListControllers, MANAGER + '/list_controllers')
    assert client.wait_for_service(timeout_sec=20), 'controller_manager service did not appear'
    command(['ros2', 'run', 'controller_manager', 'spawner', 'arm_controller', '--inactive',
             '--controller-manager', MANAGER, '--param-file', str(ROOT / 'controllers.yaml')])
    inactive = command(['ros2', 'control', 'list_controllers', '-c', MANAGER, '--verbose'])
    assert 'inactive' in inactive
    b = Bridge('first')
    bridges.append(b)
    inactive_view = b.ready()
    assert inactive_view['value']['selected_matches'] is False
    goal = json.loads((ROOT / 'goal.json').read_text())
    blocked = b.call('send', {'operation': str(uuid.uuid4()), 'invocation': str(uuid.uuid4()),
                              'goal': goal})
    assert blocked['state'] == 'REJECTED', blocked
    command(['ros2', 'control', 'set_controller_state', 'arm_controller', 'active', '-c', MANAGER])
    active = command(['ros2', 'control', 'list_controllers', '-c', MANAGER, '--verbose'])
    hardware = command(['ros2', 'control', 'list_hardware_components', '-c', MANAGER])
    interfaces = command(['ros2', 'control', 'list_hardware_interfaces', '-c', MANAGER])
    assert b.ready()['value']['selected_matches'] is True
    maps = Path(f'/proc/{manager.pid}/maps').read_text()
    libraries = sorted({line.split()[-1] for line in maps.splitlines()
                        if 'libjoint_trajectory_controller.so' in line or 'libmock_components.so' in line})
    assert any('libjoint_trajectory_controller.so' in path for path in libraries), libraries
    assert any('libmock_components.so' in path for path in libraries), libraries

    malformed = copy.deepcopy(goal)
    malformed['joints'].reverse()
    rejected = b.call('send', {'operation': str(uuid.uuid4()), 'invocation': str(uuid.uuid4()),
                              'goal': malformed})
    assert rejected['state'] == 'REJECTED', rejected  # Bridge validation, not ROS rejection.
    token = str(uuid.uuid4())
    body = {'operation': str(uuid.uuid4()), 'invocation': token, 'goal': goal}
    accepted = b.call('send', body)
    assert accepted['state'] == 'SEND_RECORDED' and accepted['value']['accepted'], accepted
    repeated = b.call('send', body)
    assert repeated['value'] == accepted['value']
    succeeded = b.result(token)
    assert succeeded['value']['ros_goal_status'] == 4, succeeded
    assert succeeded['value']['controller_error_code'] == 0, succeeded

    cancel_goal = copy.deepcopy(goal)
    cancel_goal['points'][0]['positions'] = [.3] * 6
    cancel_goal['points'][0]['time_ns'] = '4000000000'
    cancel_token = str(uuid.uuid4())
    sent_cancel = b.call('send', {'operation': str(uuid.uuid4()), 'invocation': cancel_token,
                                 'goal': cancel_goal})
    assert sent_cancel['state'] == 'SEND_RECORDED' and sent_cancel['value']['accepted'], sent_cancel
    deadline = time.monotonic() + 2
    while not any(f['invocation'] == cancel_token for f in feedback) and time.monotonic() < deadline:
        time.sleep(.02)
    assert any(f['invocation'] == cancel_token for f in feedback), 'no actual JTC action feedback'
    canceled = b.call('cancel', {'invocation': cancel_token})
    assert canceled['state'] == 'CANCEL_RESPONSE', canceled
    assert canceled['value']['goals_canceling'] == [cancel_token], canceled
    assert canceled['value']['terminal_stop_proven'] is False
    terminal_cancel = b.result(cancel_token)
    assert terminal_cancel['value']['ros_goal_status'] == 5, terminal_cancel
    b.close()

    restored = Bridge('restarted')
    bridges.append(restored)
    restored.ready()
    assert restored.instance != b.instance
    lookup = restored.result(cancel_token)
    assert lookup['value']['known_to_bridge'] is False
    assert lookup['value']['ros_goal_status'] == 5
    assert lookup['value']['controller_generation_known'] is False
    no_cancel_authority = restored.call('cancel', {'invocation': cancel_token})
    assert no_cancel_authority['state'] == 'REJECTED'
    command(['ros2', 'control', 'set_controller_state', 'arm_controller', 'inactive', '-c', MANAGER])
    assert restored.call('inspect', {})['value']['selected_matches'] is False
    refused = restored.call('send', {'operation': str(uuid.uuid4()), 'invocation': str(uuid.uuid4()),
                                     'goal': goal})
    assert refused['state'] == 'REJECTED', refused
    restored.close()
    assert controller_states, 'no controller state samples'
    manager.send_signal(signal.SIGINT)
    manager.wait(timeout=10)
    assert manager.returncode == 0, manager.returncode
    manager_log.close()
    packages = Path('/opt/rx/manifests/n5-packages.tsv').read_text()
    result = {'schema': 'rx.jtc-controller-validation.v1', 'status': 'PASS',
              'observed_at': datetime.now(timezone.utc).isoformat(),
              'controller': 'joint_trajectory_controller/JointTrajectoryController',
              'hardware': 'mock_components/GenericSystem', 'physical_qualification': 'NOT_PERFORMED',
              'catalog_level': 'SIMULATION_FIXTURE', 'commissioning': 'NOT_COMMISSIONED',
              'authority_provider': 'NONE_IN_BRIDGE_TEST; HOST_UnavailableAuthority_UNCHANGED',
              'n1_operations_executed': [], 'p_admission_or_host_journal_exercised': False,
              'bridge_config': config, 'bridge_sha256': digest(BINARY),
              'input_sha256': {name: digest(ROOT / name) for name in
                               ('run.py', 'controllers.yaml', 'mock.urdf', 'goal.json')},
              'loaded_plugin_sha256': {p: digest(Path(p)) for p in libraries},
              'manager_argv': manager_argv, 'manager_exit': manager.returncode,
              'commands': commands, 'exchanges': exchanges, 'action_feedback': feedback,
              'controller_state_samples': controller_states,
              'controller_manager_log': (WORK / 'controller-manager.log').read_text(),
              'bridge_logs': {p.name: p.read_text() for p in WORK.glob('*.stderr')},
              'packages_tsv': packages,
              'limits': ['real ROS controller software with idealized mock hardware only',
                         'no controller generation provider or external-client exclusion proof',
                         'no physical support, stop, human receipt or service-completion evidence',
                         'restart queries same running controller cache, not persistent controller history',
                         'no Rust Host, P admission, grant, permit, or N1 operation execution']}
    print(json.dumps(result), flush=True)
except BaseException:
    for path in sorted(WORK.glob('*.log')) + sorted(WORK.glob('*.stderr')):
        print(f'{path.name}:\n{path.read_text()}', file=sys.stderr)
    raise
finally:
    for bridge in bridges:
        if bridge.process.poll() is None:
            bridge.process.terminate()
            bridge.process.wait(timeout=5)
    if manager and manager.poll() is None:
        manager.send_signal(signal.SIGINT)
        try:
            manager.wait(timeout=10)
        except subprocess.TimeoutExpired:
            manager.kill()  # Only this container's GenericSystem test process.
            manager.wait(timeout=5)
    if executor:
        executor.shutdown(timeout_sec=5)
    if node:
        node.destroy_node()
    if rclpy.ok():
        rclpy.shutdown()
    if thread:
        thread.join(timeout=5)
