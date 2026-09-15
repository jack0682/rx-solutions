#!/usr/bin/env python3
"""Run real JTC + GenericSystem mock hardware in an isolated, device-free container."""
import argparse
import json
from pathlib import Path
import subprocess
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', default='rx-solutions:jtc-controller-validation')
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    args.evidence.parent.mkdir(parents=True, exist_ok=True)
    name = 'rx-n5-jtc-' + uuid.uuid4().hex[:12]
    command = ['docker', 'create', '--name', name, '--network', 'none', '--read-only',
               '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
               '--tmpfs', '/tmp:rw,uid=10001,gid=10001,mode=700',
               '-e', 'HOME=/tmp', args.image]
    try:
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=30)
        inspected = json.loads(subprocess.check_output(['docker', 'inspect', name], text=True))[0]
        host = inspected['HostConfig']
        assert inspected['Config']['User'] == '10001:10001', 'validation image must be non-root'
        assert host['ReadonlyRootfs'] and host['CapDrop'] == ['ALL']
        assert host['NetworkMode'] == 'none' and not host['Privileged']
        assert not host['Devices'] and not host['Binds']
        result = subprocess.run(['docker', 'start', '-a', name], capture_output=True, text=True, timeout=240)
        args.evidence.with_suffix('.stdout.log').write_text(result.stdout)
        args.evidence.with_suffix('.stderr.log').write_text(result.stderr)
        after = json.loads(subprocess.check_output(['docker', 'inspect', name], text=True))[0]
        if result.returncode or after['State']['ExitCode']:
            raise RuntimeError(f'controller test exited {after["State"]["ExitCode"]}; see {args.evidence.with_suffix(".stderr.log")}')
        records = []
        for line in result.stdout.splitlines():
            try:
                value = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(value, dict) and value.get('schema') == 'rx.jtc-controller-validation.v1':
                records.append(value)
        assert len(records) == 1 and records[0]['status'] == 'PASS', 'missing unique controller result'
        evidence = records[0]
        image = json.loads(subprocess.check_output(['docker', 'image', 'inspect', args.image], text=True))[0]
        evidence['container'] = {'image_id': image['Id'], 'os': image['Os'],
                                 'architecture': image['Architecture'], 'user': image['Config']['User'],
                                 'network': host['NetworkMode'], 'read_only_root': host['ReadonlyRootfs'],
                                 'cap_drop': host['CapDrop'], 'privileged': host['Privileged'],
                                 'devices': host['Devices'], 'host_bind_mounts': host['Binds'] or [],
                                 'security_options': host['SecurityOpt'], 'exit_code': after['State']['ExitCode']}
        args.evidence.write_text(json.dumps(evidence, indent=2) + '\n')
        print(json.dumps({'status': 'PASS', 'controller': evidence['controller'],
                          'hardware': evidence['hardware'], 'image': image['Id'],
                          'exchanges': len(evidence['exchanges']),
                          'feedback_samples': len(evidence['action_feedback']),
                          'physical_qualification': evidence['physical_qualification']}))
    finally:
        # Only this invocation's isolated mock-hardware container, including timeout failures.
        subprocess.run(['docker', 'rm', '-f', name], capture_output=True)


if __name__ == '__main__':
    main()
