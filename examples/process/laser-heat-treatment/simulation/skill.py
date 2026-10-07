"""File-only tending device model. All poses, forces and observations are SIMULATION.

Executed by the existing Host Python runner after admission, not a workflow runner.
One call implements one declared primitive. Host/P retain request and resource authority.
"""
import copy
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import time
import uuid


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def read(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1_048_576:
        raise ValueError('regular bounded simulation record required')
    return json.loads(path.read_bytes())


def save(root, state):
    target = root / 'state.json'
    if target.is_symlink(): raise ValueError('simulation state symlink refused')
    temporary = root / ('state-' + uuid.uuid4().hex + '.tmp')
    try:
        with temporary.open('xb') as stream:
            stream.write(encoded(state)); stream.flush(); os.fsync(stream.fileno())
        os.replace(temporary, target)
        descriptor = os.open(root, os.O_RDONLY)
        try: os.fsync(descriptor)
        finally: os.close(descriptor)
    finally:
        temporary.unlink(missing_ok=True)


def value(parameters, key, kind, unit):
    quantity = parameters[key]['value']
    data = quantity['data']
    if quantity['unit'] != unit or data['kind'] != kind:
        raise ValueError('simulation parameter type/unit differs: ' + key)
    if kind == 'NUMBER':
        span = data['range']; result = span['min']
        if type(result) not in (int, float) or not math.isfinite(result) or result != span['max']:
            raise ValueError('concrete finite number required')
        return result
    if kind == 'VECTOR':
        result = [s['min'] for s in data['ranges']]
        if any(type(n) not in (int, float) or not math.isfinite(n) or n != span['max'] for n, span in zip(result, data['ranges'])):
            raise ValueError('concrete finite vector required')
        return result
    result = data['value']
    if (kind == 'BOOLEAN' and type(result) is not bool) or (kind == 'TEXT' and not isinstance(result, str)):
        raise ValueError('simulation scalar shape differs')
    return result


def motion(parameters):
    frame = value(parameters, 'frame', 'TEXT', 'unitless')
    target = value(parameters, 'target_position', 'VECTOR', 'mm')
    approach = value(parameters, 'approach_position', 'VECTOR', 'mm')
    orientation = value(parameters, 'orientation', 'VECTOR', 'unitless')
    width = value(parameters, 'width', 'NUMBER', 'mm')
    force = value(parameters, 'force', 'NUMBER', 'N')
    if len(target) != 3 or len(approach) != 3 or len(orientation) != 4 or width <= 0 or force <= 0:
        raise ValueError('simulation pose/opening/force invalid')
    if abs(sum(n*n for n in orientation) - 1) > 1e-9:
        raise ValueError('simulation orientation must be normalized')
    if any(parameters[k]['frame'] != frame for k in ['target_position', 'approach_position', 'orientation']):
        raise ValueError('simulation pose frames differ')
    return {'target': target, 'approach': approach, 'orientation': orientation, 'frame': frame, 'width': width, 'force': force}


def main(inputs):
    if set(inputs) != {'schema','resolution','node','task','slot_index','primitive','values','done','on_failure','on_unknown'}:
        raise ValueError('exact workflow parameter shape required')
    if inputs.get('schema') != 'rx.workflow-parameters.v1' or inputs['on_failure'] != 'STOP' or inputs['on_unknown'] != 'HOLD_AND_RECONCILE':
        raise ValueError('supported pinned workflow parameters required')
    specification = read(Path(__file__).with_name('skill.json'))['simulation']
    if specification['environment'] != 'FILE_SIMULATION': raise ValueError('simulation only')
    root = Path(specification['state_directory'])
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise ValueError('owned simulation directory must be prepared explicitly')
    if root.stat().st_uid != os.getuid(): raise ValueError('simulation owner differs')
    descriptor = os.open(root/'state.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        current = root/'state.json'
        state = read(current) if current.exists() else {
            'schema': 'rx.tending-simulation-state.v1', 'environment': 'FILE_SIMULATION',
            'phase': 'SUPPLY', 'available_slots': specification['initial_slots'],
            'completed_slots': [], 'holding': False, 'clamped': False, 'door_closed': False,
            'process_done': False, 'robot_clear': True, 'effects': 0,
        }
        if state.get('schema') != 'rx.tending-simulation-state.v1' or state.get('environment') != 'FILE_SIMULATION':
            raise ValueError('simulation state identity differs')
        before = copy.deepcopy(state)
        primitive = inputs['primitive']; params = inputs['values']; slot = int(inputs['slot_index'])
        if not isinstance(inputs['slot_index'], str) or str(slot) != inputs['slot_index'] or slot < 0:
            raise ValueError('canonical slot index required')
        timeout = value(params, 'timeout_s', 'NUMBER', 's')
        if timeout <= 0: raise ValueError('positive timeout required')
        observations = {}
        def require(condition):
            if not condition: raise ValueError('simulation interlock/state does not allow ' + primitive)
        if primitive == 'pick':
            require(state['phase'] == 'SUPPLY' and slot in state['available_slots'] and not state['holding'])
            state['motion'] = motion(params)
            state.update(phase='HOLDING', holding=True, robot_clear=True,
                         active_report=inputs['resolution'], active_slot=slot)
            observations['gripper.part_held'] = True
        else:
            require(state.get('active_report') == inputs['resolution'] and state.get('active_slot') == slot)
            if primitive == 'load':
                require(state['phase'] == 'HOLDING' and state['holding'] and not state['door_closed'] and not state['clamped'])
                state['motion'] = motion(params)
                state.update(phase='SEATED', robot_clear=False)
                observations['jig.part_seated'] = True
            elif primitive == 'clamp':
                require(state['phase'] == 'SEATED' and state['holding'])
                force = value(params, 'force', 'NUMBER', 'N')
                require(force > 0 and value(params, 'state', 'BOOLEAN', 'unitless') and value(params, 'release_gripper', 'BOOLEAN', 'unitless'))
                # Simulated handover: clamp supports the part before gripper release/retreat.
                state.update(phase='CLAMPED', clamped=True, clamp_force=force, holding=False, robot_clear=True)
                state['motion']['target'] = state['motion']['approach']
                observations['chuck.clamped'] = True
            elif primitive == 'door':
                closed = value(params, 'state', 'BOOLEAN', 'unitless')
                require(state['phase'] == ('CLAMPED' if closed else 'PROCESSED') and state['robot_clear'] and state['clamped'])
                state.update(phase='CLOSED' if closed else 'OPEN', door_closed=closed)
                observations['machine.door_closed'] = closed
            elif primitive == 'process':
                require(state['phase'] == 'CLOSED' and state['door_closed'] and state['clamped'])
                duration = value(params, 'duration_s', 'NUMBER', 's')
                require(0 < duration <= timeout)
                state.update(phase='PROCESSING', process_done=False, requested_duration_s=duration)
                save(root, state)  # A killed worker leaves an explicit incomplete device state.
                started = time.monotonic()
                time.sleep(duration)
                state.update(phase='PROCESSED', process_done=True, observed_duration_s=time.monotonic()-started)
                observations['machine.process_done'] = True
            elif primitive == 'unload':
                require(state['phase'] == 'OPEN' and not state['door_closed'] and state['clamped'] and state['process_done'])
                state['motion'] = motion(params)
                # Simulated gripper support is established before unclamping.
                state.update(holding=True, clamped=False, phase='UNLOADED', robot_clear=True)
                observations['jig.part_unloaded'] = True
            elif primitive == 'place':
                require(state['phase'] == 'UNLOADED' and state['holding'])
                state['motion'] = motion(params)
                state['available_slots'].remove(slot); state['completed_slots'].append(slot)
                state.update(phase='SUPPLY', holding=False, process_done=False, robot_clear=True)
                observations['tray.part_placed'] = True
            else:
                raise ValueError('unsupported simulation primitive')
        done = inputs['done']
        expected = value({'done': {'value': done['equals']}}, 'done', 'BOOLEAN', 'unitless')
        require(observations.get(done['observation']) is expected)
        state['effects'] += 1
        save(root, state)
        effect = {'environment':'FILE_SIMULATION', 'resolution':inputs['resolution'], 'node':inputs['node'],
                  'slot_index':inputs['slot_index'], 'primitive':primitive, 'parameters':params,
                  'before_digest':hashlib.sha256(encoded(before)).hexdigest(),
                  'after_digest':hashlib.sha256(encoded(state)).hexdigest(), 'observations':observations,
                  'device':{key:state.get(key) for key in ['phase','motion','holding','clamped','door_closed','process_done']}}
        log = os.open(root/'effects.jsonl', os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NOFOLLOW, 0o600)
        with os.fdopen(log, 'ab') as stream:
            stream.write(encoded(effect)+b'\n'); stream.flush(); os.fsync(stream.fileno())
        return {'done':expected, 'observations':observations, 'resolution':inputs['resolution'],
                'slot_index':inputs['slot_index'], 'state_digest':effect['after_digest']}
    finally:
        os.close(descriptor)
