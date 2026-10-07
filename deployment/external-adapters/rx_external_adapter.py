"""Native facts for the Host-owned external-process channel. No P authority or Run loop.

An installed adapter supplies execute(envelope, correlation), observe(sources), and
custody(). Observe/custody must be passive. The Host validates declarations, freshness,
original identity and current process custody; this helper never settles/releases work.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import uuid


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False, ensure_ascii=False).encode()


def digest(value):
    return hashlib.sha256(encoded(value)).hexdigest()


def read(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1_048_576:
        raise ValueError('regular bounded native record required')
    return json.loads(path.read_bytes())


def publish(path, value):
    """Create once; complete immutable native facts before exposing them on the channel."""
    raw = encoded(value)
    if path.exists():
        if path.read_bytes() != raw:
            raise ValueError('original native fact cannot be replaced')
        return
    temporary = path.with_name('.' + uuid.uuid4().hex + '.tmp')
    try:
        with temporary.open('xb') as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def now():
    boot = str(uuid.UUID(Path('/proc/sys/kernel/random/boot_id').read_text().strip()))
    return {'clock_id': 'linux-boottime/' + boot,
            'ticks_ns': str(time.clock_gettime_ns(time.CLOCK_BOOTTIME))}


def sample(value, acquired_at=None, uncertainty_ns=0, quality_good=True, origin_age_bounded=True):
    """Call after the actual read, or pass the sensor's original timestamp/quality."""
    return {'value': value, 'acquired_at': acquired_at if acquired_at is not None else now(),
            'uncertainty_ns': str(uncertainty_ns), 'quality_good': quality_good,
            'origin_age_bounded': origin_age_bounded}


def snapshot(adapter, request, root, completed=None):
    values = adapter.observe(request['sources'])
    if set(values) != set(request['sources']):
        raise ValueError('declared source coverage differs')
    acquired = now()
    custody = adapter.custody()
    required = {'no_pending_commands', 'control_available', 'support_stable', 'safe_to_drop'}
    if set(custody) != required or any(type(v) is not bool for v in custody.values()):
        raise ValueError('native custody shape differs')
    # A live command owns its lock. Historical request files alone do not mean live work.
    for directory in root.iterdir():
        if not directory.is_dir() or directory.name == completed:
            continue
        lock = directory / 'owner.lock'
        if not lock.is_file() or lock.is_symlink():
            raise ValueError('native command ownership record differs')
        with lock.open('rb') as stream:
            try:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                custody['no_pending_commands'] = False
    return {'schema': 'rx.external-native-snapshot.v1', 'challenge': request['challenge'],
            'profile_digest': request['profile_digest'], 'device_session': request['device_session'],
            'observed_at': acquired, 'uncertainty_ns': '0', **custody, 'samples': values}


def original(request):
    dispatch = request['dispatch']
    if not isinstance(dispatch, dict):
        raise ValueError('original finite dispatch required')
    for key in ('operation', 'invocation', 'device_session'):
        if str(uuid.UUID(dispatch[key])) != dispatch[key]:
            raise ValueError('canonical native identity required')
    if dispatch['device_session'] != request['device_session']:
        raise ValueError('device generation differs')
    return {'schema': 'rx.external-native-request.v1', 'dispatch': dispatch,
            'profile_digest': request['profile_digest'], 'dispatch_digest': digest(dispatch)}


def completion(adapter, request, root, record, ignore=None):
    dispatch = request['dispatch']
    capture = None
    file = root / dispatch['operation'] / 'completion.json'
    if file.exists():
        saved = read(file)
        if set(saved) != {'schema', 'original', 'capture'} or saved['schema'] != 'rx.external-native-fact.v1' or saved['original'] != record:
            raise ValueError('completion belongs to another native request')
        capture = saved['capture']
    return {'schema': 'rx.external-native-completion.v1', 'challenge': request['challenge'],
            'dispatch_digest': record['dispatch_digest'], 'operation': dispatch['operation'],
            'invocation': dispatch['invocation'], 'intent_digest': dispatch['input']['binding']['selection']['intent_digest'],
            'profile_digest': request['profile_digest'], 'device_session': request['device_session'],
            'capture': capture, 'current': snapshot(adapter, request, root, ignore)}


def serve(adapter):
    os.umask(0o077)
    if len(sys.argv) < 3 or sys.argv[-2] not in ('execute', 'lookup', 'observe'):
        raise ValueError('Host-owned protocol mode required')
    mode, directory = sys.argv[-2:]
    root = Path(directory)
    if not root.is_absolute() or root.is_symlink() or not root.is_dir() or root.stat().st_uid != os.getuid():
        raise ValueError('owned native directory required')
    raw = sys.stdin.buffer.read(1_048_577)
    if len(raw) > 1_048_576:
        raise ValueError('native request exceeds bound')
    request = json.loads(raw)
    if set(request) != {'schema', 'challenge', 'profile_digest', 'device_session', 'now', 'dispatch', 'sources'} or request['schema'] != 'rx.external-process-channel.v1':
        raise ValueError('native protocol shape differs')
    if str(uuid.UUID(request['challenge'])) != request['challenge'] or request['device_session'] != (root / 'device-session').read_text():
        raise ValueError('native session/challenge differs')
    if now()['clock_id'] != request['now']['clock_id']:
        raise ValueError('native clock differs')
    if mode == 'observe':
        if request['dispatch'] is not None:
            raise ValueError('observation cannot carry a command')
        result = snapshot(adapter, request, root)
    else:
        record = original(request)
        dispatch = request['dispatch']
        operation = root / dispatch['operation']
        if mode == 'lookup':
            if not operation.is_dir() or operation.is_symlink() or read(operation / 'request.json') != record:
                raise ValueError('original native request absent/different')
            result = completion(adapter, request, root, record)
        else:
            # Existing originals are queried, never re-executed. A second Execute gets no new entry.
            operation.mkdir()
            parent = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(parent)
            finally:
                os.close(parent)
            lock = os.open(operation / 'owner.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                current = now()
                if dispatch['expires_at']['clock_id'] != current['clock_id'] or int(dispatch['expires_at']['ticks_ns']) <= int(current['ticks_ns']):
                    raise ValueError('native admission expired')
                publish(operation / 'request.json', record)
                entry = {'schema': 'rx.external-native-entry.v1', 'challenge': request['challenge'],
                         'request_sha256': hashlib.sha256(raw).hexdigest(), 'operation': dispatch['operation'],
                         'invocation': dispatch['invocation'], 'intent_digest': dispatch['input']['binding']['selection']['intent_digest'],
                         'profile_digest': request['profile_digest'], 'device_session': request['device_session']}
                sys.stdout.buffer.write(encoded(entry) + b'\n')
                sys.stdout.buffer.flush()
                envelope = json.loads(bytes(dispatch['input']['parameters']))
                correlation = {k: dispatch[k] for k in ('operation', 'invocation')}
                correlation['selection'] = dispatch['input']['binding']['selection']
                # Optional adapter hooks can expose its native boundary in isolated SIM fault tests.
                if hasattr(adapter, 'after_entry'):
                    adapter.after_entry(correlation)
                status = adapter.execute(envelope, correlation)
                if set(status) != {'status_schema', 'status'} or type(status['status']) is not int:
                    raise ValueError('native status shape differs')
                capture = {**status, 'native_id': dispatch['invocation'],
                           'captured_at': now(), 'device_session': request['device_session']}
                publish(operation / 'completion.json', {'schema': 'rx.external-native-fact.v1',
                                                       'original': record, 'capture': capture})
                if hasattr(adapter, 'after_completion'):
                    adapter.after_completion(correlation)
                result = completion(adapter, request, root, record, dispatch['operation'])
            finally:
                os.close(lock)
    sys.stdout.buffer.write(encoded(result))
    sys.stdout.buffer.flush()
