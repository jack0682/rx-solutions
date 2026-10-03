//! Versioned ingress only: all mutations enter the existing Host gate and delivery journal.
use super::*;
use rx_domain::types::Counter;
use rx_process_contract::execution_v2::{self as v2, host_inputs::BoundInput};
use rx_protocol::host_execution as wire;
fn protocol(hash: &[u8]) -> std::result::Result<(), Status> {
    let manifest: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../sdk/spec/host-execution/v2/binding.json"
    ))
    .map_err(|_| Status::internal("binding manifest"))?;
    if hash
        != canonical::digest("RX-HOST-EXECUTION-BINDING-v2", &manifest)
            .map_err(|_| Status::internal("binding digest"))?
            .as_bytes()
    {
        return Err(Status::failed_precondition(
            "Host execution binding mismatch",
        ));
    }
    Ok(())
}
// The v1 JSON projection treats bytes as Digests. The v2 payload and parameter
// fields are opaque canonical artifacts, so bind their verified content separately.
fn prepare_identity(
    message: &wire::PrepareExecution,
    binding: rx_domain::types::Digest,
) -> std::result::Result<(rx_domain::types::Id, rx_domain::types::Digest), Status> {
    let request = message
        .request
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("prepare request"))?;
    let (key, base) = rpc_identity(
        request,
        request.call.as_ref().and_then(|c| c.context.as_ref()),
    )?;
    let digest = canonical::digest(
        "RX-HOST-EXECUTION-PREPARE-v2",
        &(
            base,
            binding,
            rx_package::content_digest(&message.parameters),
        ),
    )
    .map_err(|e| Status::invalid_argument(e.to_string()))?;
    Ok((key, digest))
}

#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static>
    wire::host_execution_service_server::HostExecutionService for RpcHost<N, C>
{
    async fn prepare(
        &self,
        r: RpcRequest<wire::PrepareExecution>,
    ) -> std::result::Result<Response<wire::ExecutionReceipt>, Status> {
        protocol(&r.get_ref().binding_hash)?;
        let input = r
            .get_ref()
            .request
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("prepare request"))?;
        let caller = self.cell_caller(&r, input.call.as_ref(), true)?;
        let reference = r
            .get_ref()
            .reference
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("binding reference"))?;
        let bytes = &r.get_ref().payload;
        if reference.schema_id != v2::OPERATION_SCHEMA
            || reference.size_bytes != bytes.len() as u64
            || digest(&reference.sha256)? != rx_package::content_digest(bytes)
            || bytes.len() > 1_000_000
            || r.get_ref().parameters.len() > v2::MAX_PARAMETER_BYTES as usize
        {
            return Err(Status::invalid_argument(
                "execution payload integrity/bound",
            ));
        }
        let binding: v2::OperationBinding = canonical::decode_json(bytes)
            .map_err(|_| Status::invalid_argument("execution binding shape"))?;
        if canonical::bytes(&binding).map_err(|_| Status::invalid_argument("execution bytes"))?
            != *bytes
        {
            return Err(Status::invalid_argument("noncanonical execution binding"));
        }
        binding.validate().map_err(Status::invalid_argument)?;
        let binding_digest = binding.digest().map_err(Status::invalid_argument)?;
        let rpc = prepare_identity(r.get_ref(), binding_digest)?;
        let outer = r.into_inner();
        let input = outer
            .request
            .ok_or_else(|| Status::invalid_argument("prepare request"))?;
        let call = input.call.ok_or_else(|| Status::invalid_argument("call"))?;
        let base = input
            .base_request
            .ok_or_else(|| Status::invalid_argument("base request"))?;
        let permit = input
            .permit
            .ok_or_else(|| Status::invalid_argument("permit"))?;
        if call.context != base.context
            || permit.cell.as_ref().map(|c| &c.cell_id) != Some(&call.cell_id)
        {
            return Err(Status::invalid_argument("nested context/cell mismatch"));
        }
        let value = self
            .run(move |h| {
                claim(
                    &h,
                    &caller,
                    "HostExecution.Prepare",
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
                    &h,
                    &caller,
                    &base.operation_id,
                    &intent,
                    &base.intent_digest,
                    &grant,
                    &permit,
                )?;
                let op = request.operation.clone();
                h.prepare_execution(
                    &caller,
                    request,
                    BoundInput {
                        binding,
                        parameters: outer.parameters,
                    },
                )
                .map_err(failure)?;
                h.receipt_view(&caller, &op).map_err(failure)
            })
            .await?;
        Ok(Response::new(wire::ExecutionReceipt {
            receipt: Some(value),
            operation_binding: binding_digest.as_bytes().to_vec(),
        }))
    }
    async fn authorize(
        &self,
        r: RpcRequest<wire::AuthorizeExecution>,
    ) -> std::result::Result<Response<wire::ExecutionReceipt>, Status> {
        protocol(&r.get_ref().binding_hash)?;
        let input = r
            .get_ref()
            .request
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("authorize request"))?;
        let caller = self.cell_caller(&r, input.call.as_ref(), true)?;
        let rpc = rpc_identity(
            r.get_ref(),
            input.call.as_ref().and_then(|c| c.context.as_ref()),
        )?;
        let binding = digest(&r.get_ref().operation_binding)?;
        let input = r
            .into_inner()
            .request
            .ok_or_else(|| Status::invalid_argument("authorize request"))?;
        let call = input.call.ok_or_else(|| Status::invalid_argument("call"))?;
        let base = input
            .base_request
            .ok_or_else(|| Status::invalid_argument("base request"))?;
        let permit = input
            .permit
            .ok_or_else(|| Status::invalid_argument("permit"))?;
        if call.context != base.context
            || permit.cell.as_ref().map(|c| &c.cell_id) != Some(&call.cell_id)
        {
            return Err(Status::invalid_argument("nested context/cell mismatch"));
        }
        let value = self
            .run(move |h| {
                claim(
                    &h,
                    &caller,
                    "HostExecution.Authorize",
                    &rpc,
                    Some(permit.permit_id.clone()),
                )?;
                let op = parse_id(&base.operation_id)?;
                let invocation = parse_id(&base.invocation_id)?;
                h.execution_binding(&caller, &op, binding)
                    .map_err(failure)?;
                let record = h.receipt(&caller, &op).map_err(failure)?;
                let intent: base::Intent = rx_protocol::json::from_slice(
                    &canonical::bytes(&record.intent).map_err(|_| Status::internal("intent"))?,
                )?;
                let grant = base
                    .grant
                    .ok_or_else(|| Status::invalid_argument("grant"))?;
                let request = request_from_wire(
                    &h,
                    &caller,
                    &base.operation_id,
                    &intent,
                    &base.intent_digest,
                    &grant,
                    &permit,
                )?;
                h.authorize_execution(&caller, request, &invocation, binding)
                    .map_err(failure)?;
                h.receipt_view(&caller, &op).map_err(failure)
            })
            .await?;
        Ok(Response::new(wire::ExecutionReceipt {
            receipt: Some(value),
            operation_binding: binding.as_bytes().to_vec(),
        }))
    }
    async fn get_receipt(
        &self,
        r: RpcRequest<wire::ReadExecution>,
    ) -> std::result::Result<Response<wire::ExecutionReceipt>, Status> {
        protocol(&r.get_ref().binding_hash)?;
        let input = r
            .get_ref()
            .request
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("read request"))?;
        let caller = self.caller(&r, input.context.as_ref(), None, false)?;
        let op = parse_id(&input.operation_id)?;
        let binding = digest(&r.get_ref().operation_binding)?;
        let value = self
            .run(move |h| {
                h.execution_binding(&caller, &op, binding)
                    .map_err(failure)?;
                h.receipt_view(&caller, &op).map_err(failure)
            })
            .await?;
        Ok(Response::new(wire::ExecutionReceipt {
            receipt: Some(value),
            operation_binding: binding.as_bytes().to_vec(),
        }))
    }
    async fn reconcile(
        &self,
        r: RpcRequest<wire::ReadExecution>,
    ) -> std::result::Result<Response<wire::ExecutionEvidence>, Status> {
        protocol(&r.get_ref().binding_hash)?;
        let input = r
            .get_ref()
            .request
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("read request"))?;
        let caller = self.caller(&r, input.context.as_ref(), None, false)?;
        let op = parse_id(&input.operation_id)?;
        let binding = digest(&r.get_ref().operation_binding)?;
        let context = input.context.clone();
        let value = self
            .run(move |h| {
                h.execution_binding(&caller, &op, binding)
                    .map_err(failure)?;
                h.reconcile(&caller, &op).map_err(failure)?;
                let meta = h.journals().map_err(failure)?;
                let records = h
                    .evidence_after(&caller, Counter(0), 128)
                    .map_err(failure)?;
                Ok(base::EvidenceBatch {
                    context,
                    producer_journal_id: meta.evidence_journal.to_string(),
                    first_seq: records.first().map(|(n, _)| n.0).unwrap_or(1),
                    records: records.into_iter().map(|(_, r)| evidence_view(r)).collect(),
                })
            })
            .await?;
        Ok(Response::new(wire::ExecutionEvidence {
            batch: Some(value),
            operation_binding: binding.as_bytes().to_vec(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepare_identity_binds_opaque_parameters_without_digest_json_projection() {
        let id = "00000000-0000-4000-8000-000000000001";
        let mut message = wire::PrepareExecution {
            request: Some(cell::HostPrepareRequest {
                call: Some(cell::CellCall {
                    context: Some(base::CallContext {
                        session_id: id.into(),
                        call_id: id.into(),
                        request_key: Some(id.into()),
                        expected_revision: None,
                    }),
                    cell_id: "cell/sim".into(),
                    expected_cell_revision: Some(1),
                }),
                ..Default::default()
            }),
            payload: vec![1; 1000],
            parameters: vec![2; 2000],
            ..Default::default()
        };
        let binding = rx_domain::types::Digest::from_bytes([3; 32]);
        let original = prepare_identity(&message, binding).unwrap();
        message
            .request
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap()
            .context
            .as_mut()
            .unwrap()
            .call_id = "00000000-0000-4000-8000-000000000002".into();
        assert_eq!(original, prepare_identity(&message, binding).unwrap());
        message.parameters[0] = 4;
        let changed = prepare_identity(&message, binding).unwrap();
        assert_eq!(original.0, changed.0);
        assert_ne!(original.1, changed.1);
        assert_ne!(
            changed.1,
            prepare_identity(&message, rx_domain::types::Digest::from_bytes([5; 32]))
                .unwrap()
                .1
        );
    }
}
