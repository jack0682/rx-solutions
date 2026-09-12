use super::*;
use rx_protocol::host_read as wire;
#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static> wire::host_read_service_server::HostReadService
    for RpcHost<N, C>
{
    async fn inspect(
        &self,
        request: RpcRequest<wire::InspectHost>,
    ) -> std::result::Result<Response<wire::HostPayload>, Status> {
        use sha2::Digest as _;
        let binding: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../sdk/spec/host-read/v1/binding.json"
        ))
        .map_err(|_| Status::internal("Host binding"))?;
        let bytes =
            canonical::bytes(&binding).map_err(|_| Status::internal("Host binding bytes"))?;
        if request.get_ref().binding_hash != sha2::Sha256::digest(&bytes).as_slice() {
            return Err(Status::failed_precondition("Host read binding mismatch"));
        }
        let call = request
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("cell call"))?;
        if call.expected_cell_revision.is_some()
            || call
                .context
                .as_ref()
                .is_some_and(|c| c.request_key.is_some())
        {
            return Err(Status::invalid_argument(
                "read does not consume key/revision",
            ));
        }
        let caller = self.cell_caller(&request, Some(call), false)?;
        let cell = parse_name(&call.cell_id)?;
        let sources = request
            .get_ref()
            .source_ids
            .iter()
            .map(|s| parse_name(s))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let snapshot = self
            .run(move |host| {
                host.bootstrap_snapshot(&caller, &cell, &sources)
                    .map_err(failure)
            })
            .await?;
        let payload =
            canonical::bytes(&snapshot).map_err(|_| Status::internal("snapshot encode"))?;
        if payload.len() > 1_000_000 {
            return Err(Status::resource_exhausted("Host snapshot size"));
        }
        Ok(Response::new(wire::HostPayload {
            reference: Some(base::ArtifactRef {
                sha256: sha2::Sha256::digest(&payload).to_vec(),
                schema_id: "rx.host-snapshot.v1".into(),
                size_bytes: payload.len() as u64,
            }),
            payload,
        }))
    }
}
