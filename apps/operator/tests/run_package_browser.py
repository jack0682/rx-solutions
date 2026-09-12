"""Real package/review API browser harness. Only generated test identities and signed fixtures."""
import argparse,os,shlex,socket,subprocess,sys,tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--evidence-dir',type=Path,required=True);args=p.parse_args()
project=Path(__file__).resolve().parents[1]
# apps/operator -> rx-solutions/apps -> rx-solutions -> rx_ws
ws=project.parents[2];platform=ws/'rx-platform';solutions=ws/'rx-solutions'
for port in (8080,5173):
 with socket.socket() as s:
  if s.connect_ex(('127.0.0.1',port))==0:raise SystemExit(f'port {port} occupied; existing process not touched')
args.evidence_dir.mkdir(parents=True,exist_ok=False)
subprocess.run([str(solutions/'tools/cargo'),'build','-p','rx-process-package','--bin','rx-process-package','--locked'],cwd=solutions,check=True,capture_output=True)
binary=platform/'target/debug/rx-platform-local';tool=solutions/'target/debug/rx-process-package'
with tempfile.TemporaryDirectory(prefix='rx-package-browser-') as temporary:
 fixture=Path(temporary)/'fixture';env=dict(os.environ,RX_PACKAGE_BROWSER_FIXTURE=str(fixture),RX_PROCESS_PACKAGE_BIN=str(tool))
 subprocess.run([str(platform/'tools/cargo'),'test','-p','rx-api','--test','http','export_operator_package_fixture','--locked','--','--ignored','--exact'],cwd=platform,env=env,check=True,capture_output=True)
 helper='/Users/ojaehong/.codex/skills/webapp-testing/scripts/with_server.py'
 command=[sys.executable,helper,'--server',shlex.join([str(binary),'serve',str(fixture/'installation'),'127.0.0.1:8080','http://127.0.0.1:5173']),'--port','8080','--server',shlex.join(['npm','--prefix',str(project),'run','dev']),'--port','5173','--',sys.executable,str(project/'tests/browser_packages.py')]
 env.update(RX_BROWSER_PACKAGE_FIXTURE=str(fixture),RX_PACKAGE_EVIDENCE=str(args.evidence_dir.resolve()),RX_PLATFORM_REPO=str(platform))
 result=subprocess.run(command,env=env,capture_output=True,text=True)
 (args.evidence_dir/'browser-run.log').write_text(result.stdout+result.stderr);print(result.stdout+result.stderr);raise SystemExit(result.returncode)
