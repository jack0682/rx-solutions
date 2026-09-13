#!/usr/bin/env python3
"""Check declared immutable Git sources or local simulation fixtures, without device access."""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path, PurePosixPath


def check(catalog_path, source_root, workspace):
    catalog = json.loads(catalog_path.read_text())
    assert catalog['schema'] == 'rx.device-support.v1', 'catalog schema'
    pins = {row['repository']: row for row in catalog['repositories']}
    assert len(pins) == len(catalog['repositories']), 'duplicate repository'
    for repository, pin in pins.items():
        assert re.fullmatch('[0-9a-f]{40}', pin['commit']), 'immutable Git commit required'
        assert pin['url'].startswith('https://') and len(pin['url']) > 8, 'HTTPS source required'
        assert source_root is not None, '--source-root is required for external repositories'
        actual = subprocess.check_output(['git', '-C', str(source_root/repository), 'rev-parse', f"{pin['commit']}^{{commit}}"], text=True).strip()
        assert actual == pin['commit'], 'Git commit differs'
    files = {}
    ids = set()
    for profile in catalog['profiles']:
        assert profile['support_id'] not in ids, 'duplicate support profile'
        ids.add(profile['support_id'])
        fixtures = profile.get('fixture_sources', [])
        sources = profile['sources']
        assert {'native-authority','calibration','startup-effects','handover','completion-evidence'}.issubset(profile['commissioning_inputs']), 'commissioning inputs missing'
        level = profile['evidence_level']
        assert (level == 'SOURCE_OBSERVED' and sources and not fixtures) or (level == 'SIMULATION_FIXTURE' and fixtures and not sources), 'provenance was erased or relabeled'
        for source in sources + fixtures:
            path = PurePosixPath(source['path'])
            assert not path.is_absolute() and '..' not in path.parts and '\\' not in source['path'] and ':' not in source['path'], 'invalid source path'
            if source in fixtures:
                absolute = (workspace/path).resolve()
                assert absolute.is_relative_to(workspace.resolve()), 'fixture escapes workspace'
                blob = absolute.read_bytes()
                fixture = json.loads(blob)
                declaration = next(p for p in fixture['profiles'] if p['support_id'] == profile['support_id'])
                assert all(declaration[k] == profile[k] for k in ('model','role','declared_update_hz','controllers')), 'fixture declaration differs'
                key = ('local-fixture', source['path'])
            else:
                repository = source['repository']
                assert repository in pins and source['commit'] == pins[repository]['commit'], 'source revision differs'
                key = (repository, source['commit'], source['path'])
                blob = subprocess.check_output(['git','-C',str(source_root/repository),'cat-file','blob',f"{source['commit']}:{source['path']}"])
            files[key] = hashlib.sha256(blob).hexdigest()
            assert files[key] == source['sha256'], f"catalog source mismatch: {source['path']}"
    assert ids, 'catalog profiles missing'
    return {'schema':'rx.catalog-source-check.v1','status':'PASS','external_repositories':len(pins),'profiles':len(ids),'unique_source_files':len(files),'catalog_sha256':hashlib.sha256(catalog_path.read_bytes()).hexdigest(),'scope':'Source identity and declarations only; synthetic fixtures are not physical equipment qualification.'}

if __name__ == '__main__':
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser()
    parser.add_argument('--source-root', type=Path)
    parser.add_argument('--catalog', type=Path, default=root/'catalogs/device-support.v1.json')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result = check(args.catalog, args.source_root, root)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result))
