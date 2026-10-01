#!/usr/bin/env python3
"""Check the user-confirmed process/fixture relationships, not physical execution."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / 'examples/process/laser-heat-treatment'
def read(name):
    return json.loads((ROOT / name).read_text())

def sequence(source):
    flows = {f['id']: f for f in source['flows']}
    result = []
    def visit(flow_id, node_id, path):
        key = (flow_id, node_id)
        assert key not in path, 'Example contains a structural cycle'
        nodes = {n['id']: n['body'] for n in flows[flow_id]['nodes']}
        body = nodes[node_id]
        path = path | {key}
        kind = body['kind']
        if kind == 'SEQUENCE':
            for child in body['children']: visit(flow_id, child, path)
        elif kind == 'CALL': visit(body['flow'], flows[body['flow']]['root'], path)
        elif kind == 'REPEAT':
            assert body['count'] == '1', 'This authoring example is one steady-state cycle'
            visit(flow_id, body['child'], path)
        else: result.append((node_id, body))
    visit(source['entry'], flows[source['entry']]['root'], set())
    return result

def before(rows, first, second):
    ids = [i for i, _ in rows]
    assert ids.index(first) < ids.index(second), (first, second)

profiles = read('gripper-profiles.json')['profiles']
orientations = []
for profile in profiles:
    source = read(profile['source'])
    rows = sequence(source)
    operations = [body['binding'] for _, body in rows if body['kind'] == 'OPERATION']
    assert profile['rotary_alignment_station_required']
    assert 'alignment-station/rotate-to-required-orientation' in operations
    orientation = next(f for f in source['flows'] if f['id'] == 'orient-next')
    orientations.append(orientation)
    before(rows, 'step-5-rotate-align', 'orientation-confirmed')
    before(rows, 'rotation-stopped', 'step-6-regrip')
    before(rows, 'finished-held', 'step-10-unclamp')
    before(rows, 'step-14-1-seat-check', 'step-15-clamp')
    before(rows, 'clamped', 'step-16-release-next')
    before(rows, 'robot-clear', 'step-18-close-door')
    if profile['id'] == 'dual':
        assert profile['independent_grasp_slots'] == 2
        before(rows, 'step-2-pick-next', 'step-9-grip-finished')
        before(rows, 'step-18-cycle-start', 'step-19-deposit-finished')
        assert 'robot/select-incoming-end-effector' in operations
    else:
        assert profile['independent_grasp_slots'] == 1
        assert 'robot/select-incoming-end-effector' not in operations
        if profile['id'] == 'single': before(rows, 'step-19-deposit-finished', 'step-2-pick-next')
        else:
            assert profile['temporary_buffer_required'] and profile['buffer_resource'] == 'UNSPECIFIED'
            before(rows, 'buffer-support-confirmed', 'step-9-grip-finished')
            before(rows, 'step-19-deposit-finished', 'pick-from-buffer')
    assert all(body['timeout_ns'] == '0' for _, body in rows if body['kind'] == 'WAIT'), 'Unreviewed physical deadlines must stay explicitly unset'
assert all(f == orientations[0] for f in orientations)
reference = read('tooling-reference.json')
assert len(reference['part_variants']) == 10 and len(reference['setups']) == 12
assert len({(s['jig']['family'], s['jig']['form']) for s in reference['setups']}) == 8
for setup in reference['setups']:
    assert setup['part'] in {v['id'] for v in reference['part_variants']}
    assert setup['finger']['actual_part_number'] is None
    assert all(v is None for v in setup['execution_parameters'].values())
assert read('equipment-reference.json')['equipment'][1]['performed_by'] == 'DEDICATED_EQUIPMENT_NOT_ROBOT_WRIST'
print('PASS: dual/single/buffered process ordering, separate rotary alignment and product/tooling reference; physical qualification not established')
