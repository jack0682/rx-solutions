"""Run the browser contract checks against owned, disposable local-development processes.

The --server-runner argument accepts the webapp-testing skill's with_server.py helper.
Without it, start the servers separately and invoke browser_smoke.py directly.
"""
import argparse
import os
import shlex
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--platform-executable",type=Path,required=True)
parser.add_argument("--server-runner",type=Path,required=True)
parser.add_argument("--evidence-dir",type=Path,required=True)
args=parser.parse_args()
project=Path(__file__).resolve().parents[1]
args.evidence_dir.mkdir(parents=True,exist_ok=True)
for port in (8080,5173):
    with socket.socket() as check:
        if check.connect_ex(("127.0.0.1",port))==0:
            raise SystemExit(f"port {port} is occupied; existing process was not touched")
with tempfile.TemporaryDirectory(prefix="rx-browser-") as temporary:
    state=Path(temporary)/"installation"
    diagnostics=args.evidence_dir.resolve()/"diagnostic-fixtures"
    platform=project.parents[2]/"rx-platform"
    fixture_env=dict(os.environ,RX_DIAGNOSTIC_FIXTURES_DIR=str(diagnostics))
    subprocess.run([str(platform/"tools/cargo"),"test","-p","rx-application","--test","transactions","operator_diagnostic_browser_fixtures_are_real_unqualified_read_models","--locked","--","--exact"],cwd=platform,env=fixture_env,check=True,capture_output=True)
    service_health=args.evidence_dir.resolve()/"service-health-fixtures"
    health_env=dict(os.environ,RX_SERVICE_HEALTH_FIXTURES_DIR=str(service_health))
    subprocess.run([str(platform/"tools/cargo"),"test","-p","rx-api","--test","http","service_diagnostics_expire_without_rejuvenating_worker_activity_and_reject_old_owners","--locked","--","--exact"],cwd=platform,env=health_env,check=True,capture_output=True)
    binary=args.platform_executable.resolve()
    subprocess.run([str(binary),"init",str(state)],input=b"browser-fixture-password\n",check=True,capture_output=True)
    command=[sys.executable,str(args.server_runner),
        "--server",shlex.join([str(binary),"serve",str(state),"127.0.0.1:8080","http://127.0.0.1:5173"]),"--port","8080",
        "--server",shlex.join(["npm","--prefix",str(project),"run","dev"]),"--port","5173",
        "--",sys.executable,str(project/"tests/browser_smoke.py")]
    env=dict(os.environ,RX_BROWSER_EVIDENCE=str(args.evidence_dir.resolve()),RX_DIAGNOSTICS_FIXTURES=str(diagnostics),RX_SERVICE_HEALTH_FIXTURES=str(service_health))
    result=subprocess.run(command,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    (args.evidence_dir/"browser-run.log").write_text(result.stdout)
    print(result.stdout)
    raise SystemExit(result.returncode)
