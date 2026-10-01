"""Versioned workflow model/resolve client. All calculations and constraints run in P."""
import hashlib
import json
from pathlib import Path
import uuid
from definitions_client import Definitions, LocalAuthoring, reference
from runtime_client import Terminal, encoded, read_file, save_new


class OutputExistsError(ValueError):
    """The requested output already exists; no server request was sent."""


class Workflows(Definitions):
    def command(self, request_id, route, command, validate):
        with self.journal(request_id) as root:
            intent = {'connection': self.terminal.fingerprint, 'route': route, 'command': command}
            save_new(root / 'workflow-request.json', intent)
            # Replay consults P even when a local receipt exists, so current access is always checked.
            result = self.terminal.request(route, {'request_key':request_id, 'command':command})
            validate(result)
            save_new(root / 'workflow-reply.json', result)
            return result

    def apply(self, package, references, request_id):
        if (set(package) != {'schema','catalog','id','expected','label','spec'}
                or package['schema'] != 'rx.workflow-model-package.v1'):
            raise ValueError('workflow model package schema differs')
        checked = set()
        def bind(value):
            if isinstance(value,dict):
                if set(value) == {'$ref'}:
                    key = value['$ref']
                    if key not in references: raise ValueError('unknown definition alias: '+str(key))
                    ref = reference(references[key])
                    if ref['catalog'] != package['catalog']: raise ValueError('cross-catalog definition reference')
                    if key not in checked: self.show(ref); checked.add(key)
                    return ref
                return {k:bind(v) for k,v in value.items()}
            if isinstance(value,list): return [bind(v) for v in value]
            return value
        command = {k:package[k] for k in ('catalog','id','expected','label')}
        command['spec'] = bind(package['spec'])
        result = self.command(request_id,'/api/v1/workflow-models',command,lambda v:self.check_model(v,command))
        ref = result['reference']
        return {'schema':'rx.workflow-model-receipt.v1','workflow':ref,'label':result['label'],
                'definitions':references,'request_id':request_id}

    def check_model(self,result,command):
        ref = reference(result['reference'])
        if (ref['catalog'] != command['catalog'] or ref['id'] != command['id']
                or ref['revision'] != str(int(command['expected'] or '0')+1)
                or result['spec'] != command['spec'] or result['label'] != command['label']
                or result.get('updated_by') != self.terminal.principal):
            raise ValueError('workflow save receipt differs')

    def model(self, ref):
        reference(ref)
        result = self.terminal.get('/api/v1/workflow-model',catalog=ref['catalog'],id=ref['id'],revision=ref['revision'])
        if result.get('reference') != ref: raise ValueError('workflow model reference differs')
        return result

    def resolve(self, request, request_id):
        result = self.command(request_id,'/api/v1/workflow-resolutions',request,lambda v:self.check_resolution(v,request))
        return result

    def check_resolution(self,result,request=None):
        ref = reference(result['reference'])
        report = result['report']
        if (report.get('schema') != 'rx.workflow-resolution.v1' or ref['revision'] != '1'
                or ref['catalog'] != report['request']['workflow']['catalog']
                or (request is not None and report['request'] != request)
                or not isinstance(report.get('valid'),bool) or not isinstance(report.get('concrete'),bool)
                or not isinstance(report.get('violations'),list)
                or report['valid'] != (not report['violations'])
                or report['concrete'] and not report['valid']
                or report.get('status') != ('BLOCKED' if not report['valid'] else 'RESOLVED_NOT_QUALIFIED' if report['concrete'] else 'BOUNDED_INPUT_NOT_EXECUTABLE')
                or request is not None and result.get('created_by') != self.terminal.principal):
            raise ValueError('workflow resolution receipt differs')

    def report(self, ref):
        result = self.terminal.get('/api/v1/workflow-resolution',catalog=ref['catalog'],id=ref['id'])
        self.check_resolution(result)
        if result['reference'] != ref: raise ValueError('stored resolution differs')
        return result

    def recover(self, request_id):
        with self.journal(request_id) as root:
            intent = json.loads(read_file((root/'workflow-request.json').absolute()))
            if intent['connection'] != self.terminal.fingerprint: raise ValueError('original connection differs')
            route = intent['route']
            if route not in ('/api/v1/workflow-models','/api/v1/workflow-resolutions'):
                raise ValueError('original workflow route differs')
            result = self.terminal.request(route,{'request_key':request_id,'command':intent['command']})
            if route == '/api/v1/workflow-resolutions': self.check_resolution(result,intent['command'])
            else:
                self.check_model(result,intent['command'])
            save_new(root/'workflow-reply.json',result)
            return result


def quantity(text):
    if text.startswith('@'): return json.loads(read_file(Path(text[1:]).absolute()))
    value, separator, unit = text.rpartition(':')
    if not separator or not unit: raise ValueError('a value and explicit unit are required: 60:N or 20..60:N')
    if '..' in value:
        parts = value.split('..')
        if len(parts) != 2: raise ValueError('invalid interval')
        data = {'kind':'NUMBER','range':{'min':float(parts[0]),'max':float(parts[1])}}
    else:
        raw = json.loads(value)
        if isinstance(raw,bool): data = {'kind':'BOOLEAN','value':raw}
        elif isinstance(raw,(int,float)): data = {'kind':'NUMBER','range':{'min':raw,'max':raw}}
        elif isinstance(raw,str): data = {'kind':'TEXT','value':raw}
        elif isinstance(raw,list) and all(type(v) in (int,float) for v in raw):
            data = {'kind':'VECTOR','ranges':[{'min':v,'max':v} for v in raw]}
        else: raise ValueError('unsupported input value shape')
    result = {'unit':unit,'data':data}
    encoded(result)  # Refuse nonfinite JSON before sending.
    return result


def arguments(sub):
    p = sub.add_parser('workflow',help='Apply and resolve versioned workflow contracts through P')
    modes = p.add_mutually_exclusive_group(required=True)
    modes.add_argument('--connection',type=Path)
    modes.add_argument('--local-url')
    p.add_argument('--public-origin'); p.add_argument('--principal',default='admin')
    p.add_argument('--password-file',type=Path)
    p.add_argument('--state-dir',type=Path,default=Path.home()/'.local/share/rx-workflow-client')
    p.add_argument('--references',type=Path)
    p.add_argument('--format',choices=('json','text'),default='json')
    commands = p.add_subparsers(dest='action',required=True)
    a=commands.add_parser('apply'); a.add_argument('package',type=Path);a.add_argument('--request-id');a.add_argument('--output',type=Path)
    a=commands.add_parser('show');a.add_argument('model',type=Path)
    a=commands.add_parser('resolve');a.add_argument('model',type=Path)
    a.add_argument('--context',action='append',default=[]);a.add_argument('--property-set',action='append',default=[])
    a.add_argument('--override',action='append',default=[]);a.add_argument('--input',action='append',default=[])
    a.add_argument('--slot-index',type=int,default=0);a.add_argument('--latest',action='store_true')
    a.add_argument('--request-id');a.add_argument('--output',type=Path)
    a=commands.add_parser('report');a.add_argument('receipt',type=Path)
    a=commands.add_parser('recover');a.add_argument('request_id');a.add_argument('--output',type=Path)


def run(args):
    output = getattr(args, 'output', None)
    if args.action == 'resolve' and output is not None:
        output = output.absolute()
        if output.exists() or output.is_symlink():
            raise OutputExistsError(f'Output path already exists: {output}. No server request was sent. '
                                    'Use a new --output path, omit --output, or read the existing receipt with workflow report')
    if args.connection: terminal=Terminal(args.connection.absolute())
    else:
        if not args.password_file: raise ValueError('--password-file is required for local authoring')
        terminal=LocalAuthoring(args.local_url,args.public_origin or args.local_url,args.principal,args.password_file)
    client=Workflows(terminal,args.state_dir.absolute())
    refs={}
    if args.references:
        refs=json.loads(read_file(args.references.absolute()))['references']
    if args.action=='apply':
        package=json.loads(read_file(args.package.absolute()))
        request_id=args.request_id or str(uuid.uuid5(uuid.NAMESPACE_URL,terminal.fingerprint+hashlib.sha256(encoded([package,refs])).hexdigest()))
        result=client.apply(package,refs,request_id)
    elif args.action in ('show','resolve'):
        receipt=json.loads(read_file(args.model.absolute()))
        if receipt.get('schema')!='rx.workflow-model-receipt.v1': raise ValueError('workflow model receipt required')
        model=client.model(receipt['workflow'])
        refs={**receipt['definitions'],**refs}
        if args.action=='show': result=model
        else:
            contexts={}
            for item in args.context:
                slot,separator,aliases=item.partition('=')
                if not separator: raise ValueError('context must be SLOT=ALIAS')
                if slot in contexts: raise ValueError('duplicate context argument')
                try: contexts[slot]=[refs[k] for k in aliases.split(',') if k]
                except KeyError as e: raise ValueError('unknown definition alias: '+str(e)) from None
            if args.latest:
                contexts={**model['spec']['defaults'],**contexts}
                contexts={k:[client.show(ref,True)['version']['definition']['reference'] for ref in values] for k,values in contexts.items()}
            overrides={}; inputs={}
            for item in args.override:
                target,sep,value=item.partition('='); node,dot,prop=target.partition('.')
                if not sep or not dot or prop in overrides.get(node,{}): raise ValueError('override must be a unique NODE.PROPERTY=VALUE:UNIT')
                overrides.setdefault(node,{})[prop]=quantity(value)
            for item in args.input:
                key,sep,value=item.partition('=')
                if not sep or key in inputs: raise ValueError('input must be a unique KEY=VALUE:UNIT')
                inputs[key]=quantity(value)
            try: sets=[refs[k] for k in args.property_set]
            except KeyError as e: raise ValueError('unknown property-set alias: '+str(e)) from None
            request={'workflow':receipt['workflow'],'contexts':contexts,'property_sets':sets,'overrides':overrides,'inputs':inputs,'slot_index':str(args.slot_index)}
            request_id=args.request_id or str(uuid.uuid4())
            print('Resolution request: '+request_id,file=__import__('sys').stderr,flush=True)
            result=client.resolve(request,request_id)
    elif args.action=='report':
        saved=json.loads(read_file(args.receipt.absolute())); result=client.report(saved['reference'])
    else: result=client.recover(str(uuid.UUID(args.request_id)))
    if getattr(args,'output',None): save_new(args.output.absolute(),result)
    return result, format_report(result) if args.format=='text' else None


def quantity_text(q):
    def number(n):
        return str(int(n)) if isinstance(n, float) and n.is_integer() else str(n)
    def span(r):
        return number(r['min']) if r['min'] == r['max'] else number(r['min']) + '..' + number(r['max'])
    data = q['data']
    if data['kind'] == 'NUMBER': return span(data['range'])
    if data['kind'] == 'VECTOR': return '[' + ', '.join(span(r) for r in data['ranges']) + ']'
    return json.dumps(data['value'], ensure_ascii=False)


def format_report(result):
    if 'report' not in result: return json.dumps(result,indent=2,ensure_ascii=False)
    report=result['report']; names={encoded(d['reference']):d['label'] for d in report['definitions']}
    workflow=report['request']['workflow']
    def source(o):
        ref=o['reference']
        label='request input' if ref is None else ('Workflow' if ref==workflow else names.get(encoded(ref),'Definition'))+' r'+ref['revision']
        return o['kind']+': '+label+' / '+o['path']
    lines=[report['status'],'Resolution: '+result['reference']['id'],'Digest: '+result['reference']['digest'],'Slot index: '+report['request']['slot_index']]
    for step in report['steps']:
        lines.append('\n'+step['node']+' — '+step['label'])
        for key,value in step['properties'].items():
            q=value['value'];lines.append(f"  {key}: {quantity_text(q)} {q['unit']}"+(f" [{value['frame']}]" if value['frame'] else ''))
            seen = set()
            for origin in value['origins']:
                identity = encoded(origin)
                if identity in seen: continue
                seen.add(identity)
                lines.append('    ' + source(origin) + ' = ' + quantity_text(origin['value']) + ' ' + origin['value']['unit'])
    for issue in report['violations']: lines.append(issue['location']+': '+issue['code']+' — '+issue['message'])
    return '\n'.join(lines)
