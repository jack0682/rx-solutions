"""Published execution-v2 CLI. P owns selections, admission, outcomes and custody."""
import hashlib
import json
import os
from pathlib import Path
import time
import urllib.parse
import uuid
from runtime_client import RuntimeClient, Terminal, encoded, read_file, save_new
from definitions_client import reference

BASE = '/api/v1/workflow-executions/'
MUTATIONS = {'preview': 'preview', 'publish': 'publish', 'initialize-slots': 'slot-pools',
             'create-run': 'runs', 'bind-object': 'objects'}


class ExecutionClient(RuntimeClient):
    def command(self, action, command, request_id):
        route = BASE + MUTATIONS[action]
        with self.journal(request_id) as root:
            save_new(root/'execution-request.json', {'connection': self.terminal.fingerprint,
                     'action': action, 'command': command})
            # Replays consult P; a cached receipt is never authority.
            result = self.terminal.request(route, {'request_key': request_id, 'command': command})
            if action == 'preview':
                if result['reference']['id'] != command['id'] or result['workflow'] != command['candidates'][0]['request']['workflow']:
                    raise ValueError('Preview receipt differs')
            elif action == 'publish':
                if result['reference']['id'] != command['id'] or result['preview'] != command['preview'] or result['cell'] != command['cell']:
                    raise ValueError('Publication receipt differs')
            elif action == 'create-run':
                bound = result['binding']
                if bound['cell'] != command['cell'] or bound['publication'] != command['publication'] or len(bound['slots']) != int(command['count']):
                    raise ValueError('Run receipt differs')
            elif action == 'bind-object':
                if any(result[k] != command[k] for k in ('run','ordinal','object')):
                    raise ValueError('Object receipt differs')
            elif result['cell'] != command['cell'] or result['layout']['resource'] != command['resource'] or result['layout']['rule'] != command['rule']:
                raise ValueError('Inventory receipt differs')
            save_new(root/'execution-reply.json', result)
            return result

    def export(self, preview, output):
        ref = reference(preview['reference'])
        output.mkdir(mode=0o700)  # Reserve the destination before any request.
        chunks = {}
        for kind, limit in [('policy', 128*1024), ('inputs', 16*1024*1024), ('index', 2*1024*1024)]:
            pin = preview[kind]
            size = int(pin['size_bytes'])
            if not 0 < size <= limit: raise ValueError('artifact size outside execution-v2 bound')
            url = BASE+'material?'+urllib.parse.urlencode(dict(ref, artifact=kind))
            # Keep the exact bytes, including canonical number spellings.
            with self.terminal.opener.open(self.terminal.origin+url, timeout=60) as reply:
                raw = reply.read(size+1)
            if len(raw) != size or hashlib.sha256(raw).hexdigest() != pin['sha256']:
                raise ValueError('saved execution material digest/size differs: '+kind)
            parts = []
            for start in range(0, len(raw), 1024*1024):
                data = raw[start:start+1024*1024]
                path = output/(kind+'-'+str(len(parts))+'.json')
                with path.open('xb') as stream:
                    stream.write(data); stream.flush(); os.fsync(stream.fileno())
                parts.append({'path':str(path), 'sha256':hashlib.sha256(data).hexdigest()})
            chunks[kind] = {'reference':pin, 'parts':parts}
        save_new(output/'host-material.json', chunks)
        return {'preview':ref, 'material':str(output/'host-material.json'), 'qualified':False}

    def start(self, run, request_id, wait_seconds, on_status=None, until_unknown=False):
        run = str(uuid.UUID(run))
        bound = self.terminal.get(BASE+'runs', run=run)
        if bound['run'] != run: raise ValueError('Run binding differs')
        with self.journal(request_id) as root:
            save_new(root/'execution-start.json', {'connection':self.terminal.fingerprint, 'run':run,
                                                  'binding':bound})
            request_file = root/'start.request.json'
            if not request_file.exists():
                context = self.terminal.get(BASE+'start-context', cell=bound['cell'], run=run,
                                           purpose='PRODUCTION', budget_limit=str(len(bound['slots'])))
                if (context['run']['id'] != run or context['cell'] != bound['cell']
                        or context['recipe']['schema_id'] != 'rx.execution-plan.v2'
                        or context['can_request'] is not True):
                    raise ValueError('v2 Start unavailable: '+str(context.get('blocking_reason')))
                command = context['request']
                if command['run'] != run or command['budget_limit'] != str(len(bound['slots'])):
                    raise ValueError('Start request differs from reserved Run')
                save_new(request_file, {'request_key':request_id, 'command':command})
            request = json.loads(request_file.read_bytes())
            attempt = self.terminal.request(BASE+'start', request)
            if attempt['run'] != run or attempt['cell'] != bound['cell']:
                raise ValueError('Start receipt differs')
            receipt = root/'start.reply.json'
            if receipt.exists():
                if json.loads(receipt.read_bytes())['id'] != attempt['id']:
                    raise ValueError('original Start attempt changed')
            else: save_new(receipt, attempt)
        deadline = time.monotonic()+wait_seconds
        while True:
            result = self.inspect(run)
            if on_status is not None and result['run']['value']['state'] == 'EXECUTING':
                on_status(result)
            unknown = any(w['operation']['execution_knowledge'] == 'UNKNOWN' for w in result['work'])
            if (result['run']['value']['state'] not in ('PREPARED','EXECUTING')
                    or time.monotonic() >= deadline or (until_unknown and unknown)):
                return {'schema':'rx.execution-cli-receipt.v2', 'environment':result['binding']['environment'],
                        'workflow':result['binding']['name'], 'request_id':request_id,
                        'binding':bound, 'start_attempt':attempt['id'], 'result':result}
            time.sleep(.2)

    def run_parts(self, publication, objects, request_id, wait_seconds, until_unknown=False):
        reference(publication['reference'])
        if not 1 <= len(objects) <= 2400:
            raise ValueError('One to 2400 ordered objects required')
        for obj in objects: reference(obj)
        if len({(obj['catalog'], obj['id']) for obj in objects}) != len(objects):
            raise ValueError('Each Part requires a distinct actual object')
        # Preserve CP1's one-Part journal and derived request identities.
        selection = {'object':objects[0]} if len(objects) == 1 else {'objects':objects}
        with self.journal(request_id) as root:
            save_new(root/'run-one.json', {'connection':self.terminal.fingerprint,
                     'publication':publication, **selection})
            create_path = root/'create.json'
            if not create_path.exists():
                cell = self.terminal.get('/api/v1/cell', id=publication['cell'])
                cfg = cell['value']['configuration']
                if cfg.get('execution', {}).get('publication') != publication['reference']:
                    raise ValueError('selected publication is not the installed cell version')
                save_new(create_path, {'cell':publication['cell'], 'publication':publication['reference'],
                         'expected_cell':cell['revision'], 'count':str(len(objects))})
            create = json.loads(create_path.read_bytes())
        child = lambda stage: str(uuid.uuid5(uuid.UUID(request_id), 'rx.execution.'+stage))
        created = self.command('create-run', create, child('create'))
        run = created['binding']['run']
        def bind(ordinal):
            identity = 'object' if len(objects) == 1 else 'object/'+str(ordinal)
            self.command('bind-object', {'run':run, 'ordinal':str(ordinal),
                         'object':objects[ordinal-1]}, child(identity))
        bind(1)
        def next_object(result):
            parts = sorted((p['value'] for p in result['parts']), key=lambda p:int(p['ordinal']))
            if parts and parts[-1]['disposition'] == 'CONFIRMED_COMPLETED':
                ordinal = int(parts[-1]['ordinal'])+1
                if ordinal <= len(objects): bind(ordinal)
        receipt = self.start(run, child('start'), wait_seconds, next_object, until_unknown=until_unknown)
        receipt['request_id'] = request_id
        receipt['approved_reports'] = self.approved_reports(receipt['binding'], receipt['result'])
        receipt['slot_pools'] = self.slot_pools(receipt['binding'])
        return receipt

    def approved_reports(self, bound, result):
        reports = {}
        for work in result['work']:
            binding = work.get('execution')
            if binding is None: raise ValueError('v2 work binding missing')
            if binding['publication'] != bound['publication'] or binding['policy'] != bound['policy']:
                raise ValueError('work belongs to a different publication')
            pin = binding['report']; key = pin['sha256']
            if key in reports: continue
            size = int(pin['size_bytes'])
            if pin['schema_id'] != 'rx.execution-report.v2' or not 0 < size <= 900000:
                raise ValueError('approved report reference differs')
            query = {'run':bound['run'], 'report':key}
            with self.terminal.opener.open(self.terminal.origin+BASE+'report?'+urllib.parse.urlencode(query), timeout=60) as reply:
                raw = reply.read(size+1)
            if len(raw) != size or hashlib.sha256(raw).hexdigest() != key:
                raise ValueError('report differs from the actual operation binding')
            reports[key] = json.loads(raw)
        return reports

    def slot_pools(self, bound):
        result = []
        for pin in bound['pools']:
            pool = self.terminal.get(BASE+'slot-pools', **pin['resource'])
            if pool['cell'] != bound['cell']: raise ValueError('Slot pool cell differs')
            result.append({'resource':pin['resource'], 'generation':pool['generation'],
                           'layout_digest':pool['layout']['layout_digest'],
                           'holds':{k:v for k,v in pool['holds'].items() if v['run'] == bound['run']}})
        return result

    def inspect_execution(self, run, reports=False):
        result = self.inspect(run)
        if result['binding']['recipe']['schema_id'] != 'rx.execution-plan.v2':
            raise ValueError('execution-v2 Run required')
        bound = self.terminal.get(BASE+'runs', run=run)
        receipt = {'schema':'rx.execution-cli-receipt.v2', 'environment':result['binding']['environment'],
                   'workflow':result['binding']['name'], 'binding':bound, 'result':result}
        if reports: receipt['approved_reports'] = self.approved_reports(bound, result)
        receipt['slot_pools'] = self.slot_pools(bound)
        return receipt


def arguments(sub):
    p = sub.add_parser('execution', help='Publish and operate explicit v2 workflows on a registered installation')
    p.add_argument('--connection', type=Path, required=True)
    p.add_argument('--state-dir', type=Path, default=Path.home()/'.local/share/rx-execution-client')
    commands = p.add_subparsers(dest='action', required=True)
    for action in MUTATIONS:
        a = commands.add_parser(action); a.add_argument('input', type=Path)
        a.add_argument('--request-id'); a.add_argument('--output', type=Path)
    a = commands.add_parser('export'); a.add_argument('preview', type=Path); a.add_argument('directory', type=Path)
    a = commands.add_parser('start'); a.add_argument('run'); a.add_argument('--request-id')
    a.add_argument('--wait-seconds', type=int, default=60); a.add_argument('--output', type=Path)
    a = commands.add_parser('run', help='Run ordered Parts of an installed published workflow')
    a.add_argument('publication', type=Path); a.add_argument('--object', type=Path, action='append', required=True)
    a.add_argument('--until-unknown', action='store_true', help='Return the product receipt as soon as P reports UNKNOWN')
    a.add_argument('--count', type=int, help='Must match the number of ordered --object arguments')
    a.add_argument('--request-id'); a.add_argument('--wait-seconds', type=int, default=60); a.add_argument('--output', type=Path)
    a = commands.add_parser('inspect'); a.add_argument('run'); a.add_argument('--output', type=Path); a.add_argument('--reports', action='store_true')


def run(args):
    output = getattr(args, 'output', None)
    if output is not None and (output.exists() or output.is_symlink()):
        raise ValueError('Output already exists; no server request was sent: '+str(output))
    if args.action == 'export' and (args.directory.exists() or args.directory.is_symlink()):
        raise ValueError('Export directory already exists; no server request was sent')
    if args.action == 'run' and args.count is not None and args.count != len(args.object):
        raise ValueError('--count must match the number of ordered --object arguments')
    client = ExecutionClient(Terminal(args.connection.absolute()), args.state_dir.absolute())
    if args.action in MUTATIONS:
        request_id = str(uuid.UUID(args.request_id)) if args.request_id else str(uuid.uuid4())
        print('Execution request: '+request_id, file=__import__('sys').stderr, flush=True)
        result = client.command(args.action, json.loads(read_file(args.input.absolute())), request_id)
    elif args.action == 'export':
        result = client.export(json.loads(read_file(args.preview.absolute())), args.directory.absolute())
    elif args.action == 'run':
        if args.wait_seconds < 0: raise ValueError('nonnegative wait required')
        request_id = str(uuid.UUID(args.request_id)) if args.request_id else str(uuid.uuid4())
        print('Execution request: '+request_id, file=__import__('sys').stderr, flush=True)
        objects = [json.loads(read_file(path.absolute())) for path in args.object]
        result = client.run_parts(json.loads(read_file(args.publication.absolute())),
                                  objects, request_id, args.wait_seconds, until_unknown=args.until_unknown)
    elif args.action == 'start':
        if args.wait_seconds < 0: raise ValueError('nonnegative wait required')
        request_id = str(uuid.UUID(args.request_id)) if args.request_id else str(uuid.uuid4())
        print('Execution request: '+request_id, file=__import__('sys').stderr, flush=True)
        result = client.start(args.run, request_id, args.wait_seconds)
    else: result = client.inspect_execution(str(uuid.UUID(args.run)), args.reports)
    if output is not None: save_new(output.absolute(), result)
    return result
