#!/usr/bin/env python3
"""Exercise hostile direct callers of the real helper; no real device is opened."""
import argparse,json,socket,subprocess,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--helper',required=True,type=Path);p.add_argument('--caller',type=Path);p.add_argument('--require-installed-host',action='store_true');a=p.parse_args()
if a.require_installed_host:assert Path('/opt/rx/bin/rx-hostd').is_file()
rows=[]
request=(json.dumps({'operation':'11111111-1111-4111-8111-111111111111','invocation':'22222222-2222-4222-8222-222222222222','instance':'33333333-3333-4333-8333-333333333333','deadline_ns':'18446744073709551615'})+'\n').encode()
with tempfile.TemporaryDirectory(prefix='dxl-caller-') as tmp:
 caller=a.caller or Path(tmp)/'caller'
 if not a.caller:subprocess.run(['c++','-std=c++20',str(Path(__file__).with_name('dynamixel_caller_probe.cpp')),'-o',str(caller)],check=True)
 for language in ['python','cpp']:
  for mode in ['direct','socket','spoof']:
   errors=Path(tmp)/(language+'-'+mode+'.log')
   error_file=errors.open('w')
   if language=='cpp':r=subprocess.run([str(caller),str(a.helper),mode],stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=error_file,text=True)
   elif mode=='direct':r=subprocess.run([str(a.helper),'--endpoint','simulation/dynamixel/id-1'],stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=error_file,text=True)
   else:
    peer,child=socket.socketpair();peer.sendall(request)
    r=subprocess.run(['/opt/rx/bin/rx-hostd' if mode=='spoof' else str(a.helper),'--endpoint','simulation/dynamixel/id-1'],executable=str(a.helper),stdin=child,stdout=subprocess.PIPE,stderr=error_file,text=True)
    peer.close();child.close()
   error_file.close();r.stderr=errors.read_text()
   expected='DXL_HOST_CHANNEL_REQUIRED' if mode=='direct' else 'DXL_HOST_EXECUTABLE_REQUIRED'
   rows.append({'language':language,'mode':mode,'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr})
   assert r.returncode==21 and expected in r.stderr and 'helper_entered' not in r.stderr,rows[-1]
 for endpoint in ['/dev/ttyUSB0','/dev/ttyACM0','tcp://127.0.0.1:1234','simulation/dynamixel/id-2']:
  r=subprocess.run([str(a.helper),'--endpoint',endpoint],capture_output=True,text=True)
  rows.append({'endpoint':endpoint,'exit':r.returncode,'stderr':r.stderr})
  assert r.returncode==21 and 'DXL_REAL_ENDPOINT_UNSUPPORTED' in r.stderr,rows[-1]
print(json.dumps({'status':'HELPER_BOUNDARY_PASS','physical_device_opened':False,'rows':rows},indent=2))
