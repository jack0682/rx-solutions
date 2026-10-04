"""SIM transport fixture: forward exact gRPC payloads; withhold one entered call's results.
No runtime database, native device or application authority is owned here.
"""
import argparse
import asyncio
import fcntl
import hashlib
import json
import os
from pathlib import Path
import ssl
import uuid


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


class State:
    def __init__(self, directory):
        self.directory = Path(directory)
        if not self.directory.is_absolute() or self.directory.is_symlink():
            raise ValueError('owned absolute control directory required')
        self.directory.mkdir(mode=0o700, exist_ok=True)
        self.path = self.directory/'fault.json'

    def update(self, action=None):
        with (self.directory/'fault.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            value = json.loads(self.path.read_bytes()) if self.path.exists() else {'phase':'IDLE'}
            if action is not None:
                action(value)
                temporary = self.directory/(str(uuid.uuid4())+'.tmp')
                with temporary.open('xb') as stream:
                    stream.write(encoded(value)); stream.flush(); os.fsync(stream.fileno())
                os.replace(temporary, self.path)
                fd = os.open(self.directory, os.O_RDONLY)
                try: os.fsync(fd)
                finally: os.close(fd)
            return value

    def audit(self, event):
        event = dict(event, environment='SIMULATION')
        with (self.directory/'transport.jsonl').open('ab') as stream:
            stream.write(encoded(event)+b'\n'); stream.flush(); os.fsync(stream.fileno())


def control(args):
    state = State(args.state)
    if args.action == 'arm':
        def arm(value):
            if value['phase'] not in ('IDLE','RELEASED'):
                raise ValueError('prior fault must be explicitly released')
            value.clear()
            value.update(schema='rx.sim-result-loss.v1', phase='ARMED', generation=str(uuid.uuid4()),
                         publication=str(uuid.UUID(args.publication)), ordinal=args.ordinal, node=args.node)
        result = state.update(arm)
    elif args.action == 'release':
        def release(value):
            if value['phase'] not in ('BLOCKED','RELEASED'):
                raise ValueError('no confirmed native-entry fault to release')
            value['phase'] = 'RELEASED'
        result = state.update(release)
    else:
        result = state.update()
    print(json.dumps(result, indent=2))


async def serve(args):
    import grpc
    from rx.contract.v1 import contract_pb2 as base
    from rx.host.execution.v2 import execution_pb2 as execution
    config = json.loads(Path(args.config).read_bytes())
    if config['environment'] != 'SIMULATION':
        raise ValueError('this fixture is simulation only')
    state = State(args.state)
    tasks = set()

    class Relay(grpc.GenericRpcHandler):
        def __init__(self, cfg):
            self.cfg = cfg
            read = lambda name: Path(cfg[name]).read_bytes()
            creds = grpc.ssl_channel_credentials(read('ca'), read('client_key'), read('client_certificate'))
            self.channel = grpc.aio.secure_channel(cfg['upstream'], creds, options=[
                ('grpc.ssl_target_name_override', cfg['server_name']),
                ('grpc.max_receive_message_length', 1048576),
                ('grpc.max_send_message_length', 1048576)])
            self.expected_peer = hashlib.sha256(ssl.PEM_cert_to_DER_cert(read('allowed_peer').decode())).hexdigest()

        async def authenticate(self, context):
            certificates = context.auth_context().get('x509_pem_cert', [])
            if len(certificates) != 1 or hashlib.sha256(ssl.PEM_cert_to_DER_cert(certificates[0].decode())).hexdigest() != self.expected_peer:
                await context.abort(grpc.StatusCode.UNAUTHENTICATED, 'relay peer differs')

        async def hold(self, operation, method):
            value = state.update()
            if value.get('operation') != operation or value['phase'] not in ('TARGETED','BLOCKED'):
                return
            generation = value['generation']
            state.audit({'event':'WITHHELD', 'operation':operation, 'method':method, 'generation':generation})
            while True:
                value = state.update()
                if value.get('generation') != generation or value['phase'] not in ('TARGETED','BLOCKED'):
                    break
                await asyncio.sleep(.05)

        def service(self, details):
            method = details.method
            if method.endswith('/WatchObservations') or method.endswith('/Subscribe'):
                async def stream(raw, context):
                    await self.authenticate(context)
                    call = self.channel.unary_stream(method, request_serializer=lambda x:x, response_deserializer=lambda x:x)
                    async for response in call(raw, timeout=context.time_remaining()):
                        yield response
                return grpc.unary_stream_rpc_method_handler(stream, request_deserializer=lambda x:x, response_serializer=lambda x:x)

            async def unary(raw, context):
                await self.authenticate(context)
                target = state.update()
                operation = None
                prepare = method == '/rx.host.execution.v2.HostExecutionService/Prepare'
                authorize = method == '/rx.host.execution.v2.HostExecutionService/Authorize'
                if prepare:
                    value = execution.PrepareExecution.FromString(raw)
                    binding = json.loads(value.payload)
                    selection = binding['selection']
                    if (target['phase'] == 'ARMED' and binding['publication']['id'] == target['publication']
                            and int(selection['ordinal']) == target['ordinal'] and selection['node'] == target['node']):
                        def selected(saved):
                            if saved == target:
                                saved.update(phase='TARGETED', run=selection['run'], operation=binding['operation'])
                        target = state.update(selected)
                if authorize:
                    value = execution.AuthorizeExecution.FromString(raw).request.base_request
                    operation = value.operation_id
                elif method in ('/rx.host.execution.v2.HostExecutionService/GetReceipt',
                                '/rx.host.execution.v2.HostExecutionService/Reconcile'):
                    operation = execution.ReadExecution.FromString(raw).request.operation_id
                    await self.hold(operation, method)
                elif method in ('/rx.contract.v1.HostService/GetReceipt','/rx.contract.v1.HostService/Reconcile'):
                    operation = base.OperationRef.FromString(raw).operation_id
                    await self.hold(operation, method)
                elif method == '/rx.contract.v1.EvidenceService/Publish':
                    batch = base.EvidenceBatch.FromString(raw)
                    for evidence in batch.records:
                        kind = evidence.body.WhichOneof('value')
                        if kind in ('native_result','observation'):
                            operation = getattr(evidence.body, kind).correlation.operation_id
                            await self.hold(operation, method)

                async def forward():
                    remaining = context.time_remaining()
                    timeout = remaining if remaining is not None and remaining < 3600 else None
                    call = self.channel.unary_unary(method, request_serializer=lambda x:x, response_deserializer=lambda x:x)
                    response = await call(raw, timeout=timeout)
                    if authorize:
                        receipt = execution.ExecutionReceipt.FromString(response).receipt
                        expected = state.update()
                        if expected.get('operation') == operation and expected['phase'] == 'TARGETED':
                            if receipt.operation_id != operation or receipt.host_state not in (
                                    base.HOST_RECEIPT_STATE_NATIVE_ACCEPTED, base.HOST_RECEIPT_STATE_RESULT_CAPTURED):
                                raise ValueError('target did not confirm native entry')
                            def entered(saved):
                                if saved.get('generation') == expected['generation'] and saved['phase'] == 'TARGETED':
                                    saved.update(phase='BLOCKED', invocation=receipt.invocation_id,
                                                 native_entry_state=base.HostReceiptState.Name(receipt.host_state),
                                                 authorize_request_sha256=hashlib.sha256(raw).hexdigest())
                            state.update(entered)
                            state.audit({'event':'NATIVE_ENTRY_CONFIRMED', 'operation':operation,
                                         'invocation':receipt.invocation_id, 'method':method})
                    if operation and state.update().get('operation') == operation:
                        state.audit({'event':'UPSTREAM_RESPONSE', 'method':method, 'operation':operation})
                    return response
                task = asyncio.create_task(forward())
                tasks.add(task); task.add_done_callback(tasks.discard)
                try:
                    response = await asyncio.shield(task)
                    if authorize:
                        await self.hold(operation, method)
                    return response
                except grpc.aio.AioRpcError as error:
                    await context.abort(error.code(), error.details())
            return grpc.unary_unary_rpc_method_handler(unary, request_deserializer=lambda x:x, response_serializer=lambda x:x)

    servers = []
    for endpoint in config['relays']:
        server = grpc.aio.server(options=[('grpc.max_receive_message_length',1048576),
                                         ('grpc.max_send_message_length',1048576)])
        server.add_generic_rpc_handlers((Relay(endpoint),))
        credentials = grpc.ssl_server_credentials([(Path(endpoint['server_key']).read_bytes(),
            Path(endpoint['server_certificate']).read_bytes())], root_certificates=Path(endpoint['ca']).read_bytes(),
            require_client_auth=True)
        if not server.add_secure_port(endpoint['listen'], credentials): raise ValueError('relay bind failed')
        await server.start(); servers.append(server)
    print('SIM result-link ready', flush=True)
    await asyncio.gather(*(server.wait_for_termination() for server in servers))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--state', required=True)
    commands = parser.add_subparsers(dest='action',required=True)
    serve_parser = commands.add_parser('serve'); serve_parser.add_argument('--config',required=True)
    arm = commands.add_parser('arm'); arm.add_argument('--publication',required=True)
    arm.add_argument('--ordinal',type=int,required=True); arm.add_argument('--node',required=True)
    commands.add_parser('status'); commands.add_parser('release')
    args = parser.parse_args()
    if args.action == 'serve': asyncio.run(serve(args))
    else: control(args)


if __name__ == '__main__':
    main()
