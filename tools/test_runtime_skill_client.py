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

if __name__=='__main__':unittest.main()
