#!/usr/bin/env python3
"""Client request-journal controls. Transport fixtures are not runtime acceptance."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
import uuid

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/"deployment/local-skills"))
from runtime_client import RuntimeClient

def uid(): return str(uuid.uuid4())

class Peer:
    fingerprint="one-connection"
    principal="operator"
    def __init__(self):
        self.installation={"id":uid(),"store_generation":uid(),"runtime_boot":uid(),"clock_id":"clock"}
        self.binding={"name":"skill/test","cell":"cell/a","binding_digest":"a"*64,"recipe":{"sha256":"b"*64},"envelope":{"sha256":"c"*64},"site_config_digest":"d"*64,"maximum_budget":"2","input_mode":"BOUND_CONFIGURATION"}
        self.run={"id":uid(),"cell":"cell/a","recipe_digest":"b"*64,"envelope_digest":"c"*64,"state":"PREPARED"}
        self.calls=[];self.lost=True;self.can_start=True;self.started=False
    def get(self,path,**query):
        if path=="/api/v1/runtime-skills":return {"schema":"rx.runtime-skill-catalog.v1","installation":dict(self.installation),"bindings":[{"binding":dict(self.binding),"cell_revision":"1"}],"truncated":False}
        if path=="/api/v1/run/start-context":return {"installation":dict(self.installation),"cell":"cell/a","run":dict(self.run),"recipe":self.binding['recipe'],"site_config_digest":"d"*64,"cell_revision":"2","run_revision":"1","can_request":self.can_start,"blocking_reason":None if self.can_start else "NOT_COMMISSIONED","request":{"run":self.run['id'],"envelope_digest":"c"*64,"purpose":"PRODUCTION","budget_unit":"PART_ATTEMPT","budget_limit":"1","expected_cell":"2","expected_run":"1"}}
        if path=="/api/v1/runtime-skill-result":return {"schema":"rx.runtime-skill-result.v1","result_owner":"PLATFORM","installation":dict(self.installation),"binding":dict(self.binding),"run":{"value":dict(self.run)}}
        raise AssertionError(path)
    def request(self,path,body):
        self.calls.append((path,json.loads(json.dumps(body))))
        if path=="/api/v1/runs":return dict(self.run)
        if path=="/api/v1/runs/start":
            self.run['state']='COMPLETED';self.started=True
            if self.lost:self.lost=False;raise OSError('real transport may lose this reply')
            c=body['command'];return {'id':'00000000-0000-4000-8000-000000000001','run':self.run['id'],'cell':'cell/a','actor':'operator','expected_cell_revision':c['expected_cell'],'expected_run_revision':str(int(c['expected_run']) + 1)}
        raise AssertionError(path)

class AuthoringPeer(Peer):
    def __init__(self):
        super().__init__();self.source=None;self.bindings=None;self.stale=False
    def get(self,path,**query):
        if path=="/api/v1/process-draft/binding-options":
            return {"cell":"cell/a","catalog_digest":"e"*64,"candidates":[{"step":"pick"},{"step":"place"}]}
        if path=="/api/v1/process-draft-compile-input":
            if self.stale:raise ValueError("P rejected stale composition")
            return {"draft":self.source['id'],"cell":"cell/a","source":self.source['document'],
                    "source_revision":"1","binding_revision":"1","catalog_digest":"e"*64,
                    "bindings":{a:{} for a in self.bindings['selections']}}
        return super().get(path,**query)
    def request(self,path,body):
        self.calls.append((path,json.loads(json.dumps(body))))
        if path=="/api/v1/process-draft/binding-options":
            return dict(self.get(path),device_plans=body['device_plans'])
        c=body['command']
        if path=="/api/v1/process-drafts":
            self.source=c
            return {"version":{"id":c['id'],"cell":c['cell'],"revision":"1","validation":{"structurally_valid":True}},"document":c['document']}
        if path=="/api/v1/process-draft-bindings":
            self.bindings=c
            if self.lost:self.lost=False;raise OSError("lost binding receipt")
            return dict(c,revision="1",complete=True)
        raise AssertionError(path)

class ClientTests(unittest.TestCase):
    def test_lost_start_reply_survives_client_recreation_without_new_request(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Peer();key=uid();a=RuntimeClient(p,Path(temp))
            with self.assertRaises(OSError):a.invoke(key,'skill/test','cell/a',1,0)
            request=Path(temp)/key/'start.request.json';before=request.read_bytes()
            self.assertFalse((request.parent/'start.reply.json').exists())
            b=RuntimeClient(p,Path(temp));result=b.recover(key,0)
            self.assertEqual(result['result']['run']['value']['state'],'COMPLETED')
            starts=[body for path,body in p.calls if path.endswith('/start')]
            self.assertEqual(len(starts),2);self.assertEqual(starts[0],starts[1]);self.assertEqual(before,request.read_bytes())
            count=len(p.calls);b.recover(key,0);self.assertEqual(len(p.calls),count)
            with self.assertRaises(ValueError):b.invoke(key,'skill/test','cell/a',2,0)
    def test_start_receipt_from_different_run_revision_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Peer();p.lost=False;original=p.request
            def wrong_revision(path,body):
                reply=original(path,body)
                if path.endswith('/start'):reply['expected_run_revision']=body['command']['expected_run']
                return reply
            p.request=wrong_revision
            with self.assertRaisesRegex(ValueError,'receipt correlation differs'):
                RuntimeClient(p,Path(temp)).invoke(uid(),'skill/test','cell/a',1,0)

    def test_runtime_change_blocks_pending_replay(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Peer();key=uid();a=RuntimeClient(p,Path(temp))
            with self.assertRaises(OSError):a.invoke(key,'skill/test','cell/a',1,0)
            count=len(p.calls);p.installation['runtime_boot']=uid()
            with self.assertRaisesRegex(ValueError,'runtime/clock changed'):a.recover(key,0)
            self.assertEqual(len(p.calls),count)
    def test_no_start_authority_is_inferred_from_catalog(self):
        with tempfile.TemporaryDirectory() as temp:
            p=Peer();p.can_start=False;a=RuntimeClient(p,Path(temp))
            with self.assertRaisesRegex(ValueError,'not admitted start'):a.invoke(uid(),'skill/test','cell/a',1,0)
            self.assertFalse(p.started)
            self.assertFalse(any(path.endswith('/start') for path,_ in p.calls))

class CompositionTests(unittest.TestCase):
    def test_lost_binding_reply_recovers_same_draft_and_business_order(self):
        with tempfile.TemporaryDirectory() as temp:
            p=AuthoringPeer();key=uid();a=RuntimeClient(p,Path(temp))
            with self.assertRaises(OSError):a.compose(key,'transfer','cell/a',['pick','place','pick'])
            original=json.loads(json.dumps(p.calls[-1]))
            result=RuntimeClient(p,Path(temp)).recover_composition(key)
            self.assertEqual(result['status'],'DRAFT_READY_FOR_COMPILER')
            self.assertFalse(result['execution_authorized'])
            self.assertEqual(json.loads(json.dumps(p.calls[-1])),original)
            self.assertEqual(list(p.bindings['selections'].values()),['pick','place','pick'])
            kinds=[n['body']['kind'] for n in result['compile_input']['source']['flows'][0]['nodes']]
            self.assertEqual(kinds,['SEQUENCE','OPERATION','OPERATION','OPERATION'])
            count=len(p.calls);a.recover_composition(key);self.assertEqual(len(p.calls),count)
            p.stale=True
            with self.assertRaisesRegex(ValueError,'stale composition'):a.recover_composition(key)
    def test_changed_order_or_store_cannot_retarget_saved_request(self):
        with tempfile.TemporaryDirectory() as temp:
            p=AuthoringPeer();p.lost=False;key=uid();a=RuntimeClient(p,Path(temp))
            a.compose(key,'transfer','cell/a',['pick','place']);count=len(p.calls)
            with self.assertRaises(ValueError):a.compose(key,'transfer','cell/a',['place','pick'])
            p.installation['store_generation']=uid()
            with self.assertRaisesRegex(ValueError,'installation/store'):a.recover_composition(key)
            self.assertEqual(len(p.calls),count)
    def test_reviewed_plan_is_retained_across_receipt_loss_and_cannot_be_retargeted(self):
        with tempfile.TemporaryDirectory() as temp:
            p=AuthoringPeer();key=uid();a=RuntimeClient(p,Path(temp));plan={"id":uid(),"revision":"2","plan_digest":"f"*64}
            with self.assertRaises(OSError):a.compose(key,'transfer','cell/a',['pick'],[plan])
            a.recover_composition(key)
            self.assertEqual(p.bindings['device_plans'],[plan])
            count=len(p.calls);changed=dict(plan,revision="3")
            with self.assertRaises(ValueError):a.compose(key,'transfer','cell/a',['pick'],[changed])
            self.assertEqual(len(p.calls),count)

    def test_missing_step_does_not_save_server_draft(self):
        with tempfile.TemporaryDirectory() as temp:
            p=AuthoringPeer()
            with self.assertRaisesRegex(ValueError,'steps not found'):
                RuntimeClient(p,Path(temp)).compose(uid(),'transfer','cell/a',['missing'])
            self.assertFalse(p.calls)

if __name__=='__main__':unittest.main()
