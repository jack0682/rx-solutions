#!/usr/bin/env python3
"""AI Sapiens asset admission with observed mock-controller pipe output."""
import argparse, hashlib, json, platform, subprocess, sys
from pathlib import Path
PINS=Path('/opt/rx/tools/ai-sapiens/dependencies.json')
class Refusal(Exception):pass
def emit(v):print(json.dumps(v,sort_keys=True),flush=True)
def audit(root):
 v=json.loads(PINS.read_text());missing=[];failed=[]
 for name,expected in v['policy_assets'].items():
  p=root/name/'exported/policy.onnx'
  if not p.is_file():missing.append(name)
  elif hashlib.sha256(p.read_bytes()).hexdigest()!=expected:failed.append(name)
 problem=missing or failed
 if problem:raise Refusal('AI_SAPIENS_POLICY_ASSETS_MISSING' if missing else 'AI_SAPIENS_POLICY_LOAD_FAILED')
 return v
def measure():
 code="import os,sys;os.write(sys.stdout.fileno(),sys.argv[1].encode())"
 children=[subprocess.Popen([sys.executable,'-I','-B','-c',code,label],stdout=subprocess.PIPE) for label in ('I','R')]
 observed=[child.communicate(timeout=5)[0] for child in children]
 return {'controller_processes_started':len(children),'impedance_observed_pipe_bytes':len(observed[0]),'rc_observed_pipe_bytes':len(observed[1]),'observation_basis':'SIMULATED_CONTROLLER_OS_PIPE'}
def probe(root):
 try:v=audit(root)
 except Refusal as e:
  emit({'schema':'rx.ai-sapiens-refusal.v1','condition':str(e),'current_permission':'NOT_GRANTED','controller_processes_started':0,'impedance_observed_pipe_bytes':0,'rc_observed_pipe_bytes':0,'observation_basis':'SIMULATED_CONTROLLER_OS_PIPE','physical_qualification':'NOT_PERFORMED'});raise SystemExit(76)
 emit({'schema':'rx.ai-sapiens-residual-output.v1','onnx_runtime':v['onnx_runtime'],'policy_assets':'VERIFIED',**measure(),'physical_qualification':'NOT_PERFORMED'})
def main():
 p=argparse.ArgumentParser();s=p.add_subparsers(dest='cmd',required=True);q=s.add_parser('probe');q.add_argument('--asset-root',required=True,type=Path);u=s.add_parser('support');u.add_argument('kind',choices=('usb','imu','architecture'));a=p.parse_args()
 if a.cmd=='probe':probe(a.asset_root)
 elif a.kind=='usb':raise Refusal('AI_SAPIENS_RADIOMASTER_USB_UNAVAILABLE')
 elif a.kind=='imu':raise Refusal('AI_SAPIENS_IMU_SOURCE_UNAVAILABLE')
 else:emit({'schema':'rx.ai-sapiens-architecture.v1','architecture':platform.machine(),'amd64':'SOURCE_SUPPORTED_UNQUALIFIED','arm64':'SOURCE_SUPPORTED_UNQUALIFIED','physical_qualification':'NOT_PERFORMED'})
if __name__=='__main__':
 try:main()
 except Refusal as e:emit({'schema':'rx.ai-sapiens-refusal.v1','condition':str(e),'current_permission':'NOT_GRANTED','physical_qualification':'NOT_PERFORMED'});raise SystemExit(76)
