#!/usr/bin/env python3
"""Exercise the persistent C++ process with bounded private-pipe traffic; simulation only."""
from pathlib import Path
import argparse, copy, json, selectors, subprocess, tempfile, uuid

parser=argparse.ArgumentParser()
parser.add_argument('--fixture-dir',type=Path,required=True)
parser.add_argument('--evidence-dir',type=Path,required=True)
args=parser.parse_args();out=args.evidence_dir.absolute();out.mkdir(parents=True,exist_ok=False)
fixture=args.fixture_dir.absolute()
process=json.loads((fixture/'resolved.json').read_text())
xml=(fixture/'process.bt.xml').read_text()
frame=json.loads((fixture/'frame-0.json').read_text())
image=subprocess.check_output(['docker','image','inspect','rx-solutions:executor-validation','--format','{{.Id}}'],text=True).strip()
results={}
class Engine:
    def __init__(self, clock, production=False, extra=()):
        command=['docker','run','--rm','--pull=never','-i','--network','none','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','-v',f'{clock}:/clock.json:ro','--entrypoint','/build/executor/rx-bt-engine' + ('' if production else '-fixture'),image]
        command += list(extra) if production else ['/clock.json']
        self.process=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        self.sequence=0;self.replies=[]
    def read(self):
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout,selectors.EVENT_READ)
            assert selector.select(5), 'engine response timeout'
        packet=self.process.stdout.readline(1000001)
        assert packet.endswith(b'\n') and len(packet)<=1000000, 'missing or oversized reply'
        reply=json.loads(packet);self.replies.append(reply);return reply
    def raw(self, value):
        try:self.process.stdin.write(value);self.process.stdin.flush()
        except BrokenPipeError:pass
        return self.read()
    def send(self, command, **fields):
        self.sequence+=1
        return self.raw(json.dumps(dict(schema='rx.bt-command.v1',sequence=str(self.sequence),command=command,**fields),separators=(',',':')).encode()+b'\n')
    def init(self):
        reply=self.send('INITIALIZE',identity=frame['identity'],resolved=process,xml=xml)
        assert reply['state']=='READY' and reply['requests']==[]
    def wait(self, code):
        assert self.process.wait(timeout=5)==code
    def cleanup(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:self.process.kill();self.process.wait()

with tempfile.TemporaryDirectory(prefix='rx-engine-clock-') as temporary:
    clock=Path(temporary)/'clock.json'
    def case(name, run, production=False, extra=(), ticks="1000"):
        case_clock=Path(temporary)/(name+".json")
        case_clock.write_text(json.dumps(ticks))
        engine=Engine(case_clock,production,extra)
        try:run(engine)
        finally:
            engine.cleanup(); results[name]=engine.replies
            (out/"replies.json").write_text(json.dumps(results,indent=2)+"\n")
    def normal(engine):
        engine.init();first=engine.send('STEP',frame=frame)
        assert first['state']=='RUNNING' and len(first['requests'])==1
        assert engine.send('STEP',frame=frame)['requests']==[]
        paused=engine.send('HALT');assert paused['state']=='HALTED' and [r['kind'] for r in paused['requests']]==['PAUSE_EXECUTOR']
        failed=engine.send('STEP',frame=frame);assert failed['state']=='FAULT' and failed['requests']==[];engine.wait(2)
    case('one_request_across_ticks_and_latched_halt',normal)
    def stale(engine):
        engine.init()
        old=engine.send('STEP',frame=frame);assert old['state']=='STALE' and old['requests']==[]
        fresh=copy.deepcopy(frame);now=int(frame['source']['valid_until_ns'])
        fresh['source']['checked_at_ns']=str(now);fresh['source']['valid_until_ns']=str(now+100000000)
        fresh['remaining_validity_ns']='100000000';fresh['revision']=str(int(frame['revision'])+1)
        reply=engine.send('STEP',frame=fresh);assert reply['state']=='RUNNING' and len(reply['requests'])==1
        engine.send('CLOSE');engine.wait(0)
    case('expired_frame_waits_for_fresh_data',stale,ticks=frame['source']['valid_until_ns'])
    def foreign(engine):
        engine.init();changed=copy.deepcopy(frame);changed['identity']['epoch']=str(int(frame['identity']['epoch'])+1)
        reply=engine.send('STEP',frame=changed);assert reply['state']=='FAULT' and [r['kind'] for r in reply['requests']]==['PAUSE_EXECUTOR'];engine.wait(2)
    case('foreign_context_is_not_adopted',foreign)
    def duplicate(engine):
        engine.init();reply=engine.raw(b'{"schema":"rx.bt-command.v1","sequence":"2","command":"STEP","command":"HALT"}\n')
        assert reply['state']=='FAULT';engine.wait(2)
    case('duplicate_json_key_is_rejected',duplicate)
    def wait_changed(engine):
        engine.init();engine.send('STEP',frame=frame)
        wait=frame['eligible'][0]
        changed=copy.deepcopy(frame);changed['revision']=str(int(frame['revision'])+1)
        changed['nodes'][wait]=dict(operation_id=None,outcome='NONE',unknown=False,integrity_valid=True,released=False,branch=None,decision_id=str(uuid.uuid4()),wait='SATISFIED',clearance=None)
        engine.send('STEP',frame=changed)
        changed['revision']=str(int(changed['revision'])+1);changed['nodes'][wait]['wait']='TIMED_OUT'
        reply=engine.send('STEP',frame=changed);assert reply['state']=='FAULT';engine.wait(2)
    case('committed_wait_result_is_immutable',wait_changed)
    def truncated(engine):
        engine.init();engine.process.stdin.write(b'{"schema":');engine.process.stdin.close()
        assert engine.read()['state']=='FAULT';engine.wait(2)
    case('truncated_packet_fails_closed',truncated)
    def oversized(engine):
        engine.init();assert engine.raw(b' '*(4194304+1)+b'\n')['state']=='FAULT';engine.wait(2)
    case('packet_size_is_bounded',oversized)
    def real_clock(engine):
        engine.init();assert engine.send('STEP',frame=frame)['state']=='FAULT';engine.wait(2)
    case('production_clock_rejects_synthetic_frame',real_clock,True)
    def override(engine):
        assert engine.read()['state']=='FAULT';engine.wait(2)
    case('production_has_no_clock_override',override,True,('--clock','test-clock'))
(out/'replies.json').write_text(json.dumps(results,indent=2)+'\n')
(out/'result.json').write_text(json.dumps({'schema':'rx.bt-engine-check.v1','status':'PASS','cases':list(results),'image':image,'scope':'Actual persistent C++ process/private pipes with recorded simulation P inputs; no P network, ROS, native device or physical qualification.'},indent=2)+'\n')
print(f'PASS: {len(results)} persistent BT IPC cases; image {image}')
