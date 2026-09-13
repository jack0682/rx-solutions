#!/usr/bin/env python3
"""Read-only startup diagnostics. No ROS/BT/driver process is launched by this entry point."""
import argparse,hashlib,json,signal,threading,os
from http.server import BaseHTTPRequestHandler,HTTPServer
from pathlib import Path
ROOT=Path('/opt/rx')
def inspect():
    inventory=json.loads((ROOT/'manifests/runtime-files.json').read_text())
    if inventory['schema']!='rx.solutions-runtime-files.v1':raise RuntimeError('runtime inventory schema')
    for path,digest in inventory.get('external_files',{}).items():
        if hashlib.sha256(Path(path).read_bytes()).hexdigest()!=digest:raise RuntimeError('external runtime file integrity differs')
    for path,digest in inventory['files'].items():
        if hashlib.sha256((ROOT/path).read_bytes()).hexdigest()!=digest:raise RuntimeError('runtime file integrity differs')
    audit=json.loads((ROOT/'manifests/native-install-audit.json').read_text())
    catalog_path=ROOT/'catalogs/device-support.v1.json'
    catalog=json.loads(catalog_path.read_text())
    if audit['status']!='PASS' or audit['catalog_sha256']!=hashlib.sha256(catalog_path.read_bytes()).hexdigest():raise RuntimeError('native audit identity mismatch')
    for binary in audit['elf']:
        if hashlib.sha256((ROOT/binary['path']).read_bytes()).hexdigest()!=binary['sha256']:raise RuntimeError('native binary integrity differs')
    for name in ['rx-bt-engine','rx-executor-service','rx-process-compile','rx-hostd','rx-ros-jtc-bridge']:
        if not (ROOT/'bin'/name).is_file():raise RuntimeError('required RX executable missing')
    if not (ROOT/'operator/index.html').is_file():raise RuntimeError('operator UI build missing')
    return {'schema':'rx.solutions-status.v1','supervisor_instance':os.environ.get('RX_PROCESS_INSTANCE_ID'),'phase':'SOFTWARE_READY_UNCOMMISSIONED','native_packages':audit['native_packages'],'external_device_repositories':len(catalog['repositories']),'support_profiles':len(catalog['profiles']),'simulation_profiles':sum(p['evidence_level']=='SIMULATION_FIXTURE' for p in catalog['profiles']),'physical_qualification':'NOT_PERFORMED','hardware_processes_started_by_entrypoint':0,'operator_api_delegation':'NOT_CONNECTED'}
def main():
    p=argparse.ArgumentParser();p.add_argument('mode',choices=['inspect','serve'],nargs='?',default='serve');p.add_argument('--bind',default='0.0.0.0');p.add_argument('--port',type=int,default=8081);a=p.parse_args();report=inspect()
    if a.mode=='inspect':print(json.dumps(report));return
    payload=json.dumps(report).encode()
    class Handler(BaseHTTPRequestHandler):
        def setup(self):super().setup();self.connection.settimeout(2)
        def log_message(self,*args):pass
        def do_GET(self):
            allowed=self.path in ('/','/health','/api/v1/solution/support');body=payload if allowed else b'{"code":"NOT_FOUND"}'
            self.send_response(200 if allowed else 404);self.send_header('Content-Type','application/json');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        def do_POST(self):
            body=b'{"code":"CONTROL_NOT_EXPOSED","outcome_unknown":false}'
            self.send_response(503);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    server=HTTPServer((a.bind,a.port),Handler)
    def stop(*_):threading.Thread(target=server.shutdown,daemon=True).start()
    signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
    try:server.serve_forever(poll_interval=.2)
    finally:server.server_close()
if __name__=='__main__':main()
