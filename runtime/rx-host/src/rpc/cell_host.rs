use super::*;
use rx_domain::types::Counter;

#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static> cell::cell_host_service_server::CellHostService
    for RpcHost<N, C>
{
    async fn inspect(
        &self,
        request: RpcRequest<cell::CellCall>,
    ) -> std::result::Result<Response<cell::HostCellState>, Status> {
        let caller = self.cell_caller(&request, Some(request.get_ref()), false)?;
        let cell = parse_name(&request.into_inner().cell_id)?;
        let output = self
            .run(move |host| {
                let i = host.inspect_cell(&caller, &cell).map_err(failure)?;
                Ok(cell::HostCellState {
                    cell: Some(cell_ref(&cell, &i.state)),
                    host_boot_id: i.host_boot.to_string(),
                    block_ids: i
                        .state
                        .blocked
                        .into_iter()
                        .map(|id| id.to_string())
                        .collect(),
                    pending_operation_ids: i
                        .pending_operations
                        .into_iter()
                        .map(|id| id.to_string())
                        .collect(),
                    pending_permit_ids: i
                        .pending_permits
                        .into_iter()
                        .map(|id| id.to_string())
                        .collect(),
                    definition_digest: i.definition.as_bytes().to_vec(),
                })
            })
            .await?;
        Ok(Response::new(output))
    }
    async fn fence_cell(
        &self,
        request: RpcRequest<cell::HostFenceCellRequest>,
    ) -> std::result::Result<Response<cell::FenceReceipt>, Status> {
        let caller = self.cell_caller(&request, request.get_ref().call.as_ref(), true)?;
        let rpc = rpc_identity(
            request.get_ref(),
            request
                .get_ref()
                .call
                .as_ref()
                .and_then(|c| c.context.as_ref()),
        )?;
        let input = request.into_inner();
        let call = input.call.ok_or_else(|| Status::invalid_argument("call"))?;
        let target = input
            .target
            .ok_or_else(|| Status::invalid_argument("target"))?;
        if target.cell_id != call.cell_id {
            return Err(Status::invalid_argument("cell mismatch"));
        }
        let scope = scopes(&target)?;
        let cell = parse_name(&target.cell_id)?;
        let request_id = parse_id(&input.invalidation_id)?;
        let blocks = ids(&input.block_ids)?;
        let output = self
            .run(move |host| {
                claim(
                    &host,
                    &caller,
                    "CellHost.FenceCell",
                    &rpc,
                    Some(request_id.to_string()),
                )?;
                let ack = host
                    .fence(
                        &caller,
                        request_id,
                        &cell,
                        Counter(target.cell_epoch),
                        scope,
                        blocks,
                    )
                    .map_err(failure)?;
                Ok(cell::FenceReceipt {
                    target: Some(cell_ref(&cell, &ack.state)),
                    invalidation_id: ack.id.to_string(),
                    host_boot_id: ack.host_boot.to_string(),
                    delivery_journal_id: ack.journal.to_string(),
                    seq: ack.sequence.0,
                })
            })
            .await?;
        Ok(Response::new(output))
    }
    async fn arm_cell(
        &self,
        request: RpcRequest<cell::HostArmCellRequest>,
    ) -> std::result::Result<Response<cell::ArmReceipt>, Status> {
        let caller = self.cell_caller(&request, request.get_ref().call.as_ref(), true)?;
        let rpc = rpc_identity(
            request.get_ref(),
            request
                .get_ref()
                .call
                .as_ref()
                .and_then(|c| c.context.as_ref()),
        )?;
        let input = request.into_inner();
        let call = input.call.ok_or_else(|| Status::invalid_argument("call"))?;
        let target = input
            .target
            .ok_or_else(|| Status::invalid_argument("target"))?;
        if target.cell_id != call.cell_id {
            return Err(Status::invalid_argument("cell mismatch"));
        }
        let scope = scopes(&target)?;
        let cell = parse_name(&target.cell_id)?;
        let attempt = parse_id(&input.attempt_id)?;
        ids(&input.clearance_ids)?;
        let clear = ids(&input.block_ids_to_clear)?;
        let output = self
            .run(move |host| {
                claim(
                    &host,
                    &caller,
                    "CellHost.ArmCell",
                    &rpc,
                    Some(attempt.to_string()),
                )?;
                let ack = host
                    .arm(
                        &caller,
                        attempt,
                        &cell,
                        Counter(target.cell_epoch),
                        &scope,
                        &clear,
                    )
                    .map_err(failure)?;
                Ok(cell::ArmReceipt {
                    target: Some(cell_ref(&cell, &ack.state)),
                    attempt_id: ack.id.to_string(),
                    host_boot_id: ack.host_boot.to_string(),
                    delivery_journal_id: ack.journal.to_string(),
                    seq: ack.sequence.0,
                })
            })
            .await?;
        Ok(Response::new(output))
    }
    async fn prepare(
        &self,
        request: RpcRequest<cell::HostPrepareRequest>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        let caller = self.cell_caller(&request, request.get_ref().call.as_ref(), true)?;
        let rpc = rpc_identity(
            request.get_ref(),
            request
                .get_ref()
                .call
                .as_ref()
                .and_then(|c| c.context.as_ref()),
        )?;
        let input = request.into_inner();
        let call = input.call.ok_or_else(|| Status::invalid_argument("call"))?;
        let base = input
            .base_request
            .ok_or_else(|| Status::invalid_argument("base request"))?;
        if call.context != base.context {
            return Err(Status::invalid_argument("nested context mismatch"));
        }
        let permit = input
            .permit
            .ok_or_else(|| Status::invalid_argument("permit"))?;
        if permit.cell.as_ref().map(|c| &c.cell_id) != Some(&call.cell_id) {
            return Err(Status::invalid_argument("cell mismatch"));
        }
        let output = self
            .run(move |host| {
                claim(
                    &host,
                    &caller,
                    "CellHost.Prepare",
                    &rpc,
                    Some(permit.permit_id.clone()),
                )?;
                let intent = base
                    .intent
                    .ok_or_else(|| Status::invalid_argument("intent"))?;
                let grant = base
                    .grant
                    .ok_or_else(|| Status::invalid_argument("grant"))?;
                let request = request_from_wire(
                    &host,
                    &caller,
                    &base.operation_id,
                    &intent,
                    &base.intent_digest,
                    &grant,
                    &permit,
                )?;
                let op = request.operation.clone();
                match host.prepare(&caller, request) {
                    Ok(_) | Err(HostError::Voided) => {
                        host.receipt_view(&caller, &op).map_err(failure)
                    }
                    Err(e) => Err(failure(e)),
                }
            })
            .await?;
        Ok(Response::new(output))
    }
    async fn authorize(
        &self,
        request: RpcRequest<cell::HostAuthorizeRequest>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        let caller = self.cell_caller(&request, request.get_ref().call.as_ref(), true)?;
        let rpc = rpc_identity(
            request.get_ref(),
            request
                .get_ref()
                .call
                .as_ref()
                .and_then(|c| c.context.as_ref()),
        )?;
        let input = request.into_inner();
        let call = input.call.ok_or_else(|| Status::invalid_argument("call"))?;
        let base = input
            .base_request
            .ok_or_else(|| Status::invalid_argument("base request"))?;
        if call.context != base.context {
            return Err(Status::invalid_argument("nested context mismatch"));
        }
        let permit = input
            .permit
            .ok_or_else(|| Status::invalid_argument("permit"))?;
        if permit.cell.as_ref().map(|c| &c.cell_id) != Some(&call.cell_id) {
            return Err(Status::invalid_argument("cell mismatch"));
        }
        let output = self
            .run(move |host| {
                claim(
                    &host,
                    &caller,
                    "CellHost.Authorize",
                    &rpc,
                    Some(permit.permit_id.clone()),
                )?;
                let operation = parse_id(&base.operation_id)?;
                let invocation = parse_id(&base.invocation_id)?;
                let record = host.receipt(&caller, &operation).map_err(failure)?;
                let intent: base::Intent = rx_protocol::json::from_slice(
                    &canonical::bytes(&record.intent)
                        .map_err(|e| Status::internal(e.to_string()))?,
                )?;
                let grant = base
                    .grant
                    .ok_or_else(|| Status::invalid_argument("grant"))?;
                let request = request_from_wire(
                    &host,
                    &caller,
                    &base.operation_id,
                    &intent,
                    &base.intent_digest,
                    &grant,
                    &permit,
                )?;
                host.authorize(&caller, request, &invocation)
                    .map_err(failure)?;
                host.receipt_view(&caller, &operation).map_err(failure)
            })
            .await?;
        Ok(Response::new(output))
    }
}
