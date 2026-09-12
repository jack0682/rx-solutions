use super::*;
use rx_domain::host_qualification as data;
use rx_protocol::host_qualification as wire;
fn check_binding(hash: &[u8]) -> std::result::Result<(), Status> {
    let value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../sdk/spec/host-qualification/v1/binding.json"
    ))
    .map_err(|_| Status::internal("binding manifest"))?;
    if hash
        != rx_domain::canonical::digest("RX-HOST-CONFIGURATION-BINDING-v1", &value)
            .map_err(|_| Status::internal("binding digest"))?
            .as_bytes()
    {
        return Err(Status::failed_precondition(
            "Host qualification binding mismatch",
        ));
    }
    Ok(())
}
fn payload(
    value: &data::Observation,
) -> std::result::Result<Response<wire::QualificationPayload>, Status> {
    use sha2::Digest as _;
    let bytes =
        canonical::bytes(value).map_err(|_| Status::internal("qualification observation"))?;
    if bytes.len() > 1_000_000 {
        return Err(Status::resource_exhausted("qualification observation size"));
    }
    Ok(Response::new(wire::QualificationPayload {
        reference: Some(base::ArtifactRef {
            sha256: sha2::Sha256::digest(&bytes).to_vec(),
            schema_id: "rx.host-qualification-observation.v1".into(),
            size_bytes: bytes.len() as u64,
        }),
        payload: bytes,
    }))
}
#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static>
    wire::host_qualification_service_server::HostQualificationService for RpcHost<N, C>
{
    async fn inspect(
        &self,
        r: RpcRequest<wire::InspectQualification>,
    ) -> std::result::Result<Response<wire::QualificationPayload>, Status> {
        check_binding(&r.get_ref().binding_hash)?;
        if r.get_ref()
            .context
            .as_ref()
            .is_some_and(|c| c.request_key.is_some() || c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument("read context"));
        }
        let caller = self.caller(&r, r.get_ref().context.as_ref(), None, false)?;
        let v = self
            .run(move |h| h.inspect_qualification(&caller).map_err(failure))
            .await?;
        payload(&v)
    }
    async fn lookup(
        &self,
        r: RpcRequest<wire::LookupQualification>,
    ) -> std::result::Result<Response<wire::QualificationPayload>, Status> {
        check_binding(&r.get_ref().binding_hash)?;
        if r.get_ref()
            .context
            .as_ref()
            .is_some_and(|c| c.request_key.is_some() || c.expected_revision.is_some())
        {
            return Err(Status::invalid_argument("read context"));
        }
        let caller = self.caller(&r, r.get_ref().context.as_ref(), None, false)?;
        let id = parse_id(&r.get_ref().request_id)?;
        let v = self
            .run(move |h| h.lookup_qualification(&caller, &id).map_err(failure))
            .await?;
        payload(&v)
    }
    async fn accept(
        &self,
        r: RpcRequest<wire::AcceptQualification>,
    ) -> std::result::Result<Response<wire::QualificationPayload>, Status> {
        use sha2::Digest as _;
        check_binding(&r.get_ref().binding_hash)?;
        let call = r
            .get_ref()
            .call
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("cell call"))?;
        if call.expected_cell_revision.is_some() {
            return Err(Status::invalid_argument("unexpected cell revision"));
        }
        let caller = self.cell_caller(&r, Some(call), true)?;
        let bytes = &r.get_ref().payload;
        let reference = r
            .get_ref()
            .reference
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("payload reference"))?;
        if bytes.len() > 1_000_000
            || reference.schema_id != "rx.host-qualification-request.v1"
            || reference.size_bytes != bytes.len() as u64
            || reference.sha256 != sha2::Sha256::digest(bytes).as_slice()
        {
            return Err(Status::invalid_argument("qualification payload integrity"));
        }
        let input: data::Request = canonical::decode_json(bytes)
            .map_err(|_| Status::invalid_argument("qualification payload shape"))?;
        input.validate().map_err(Status::invalid_argument)?;
        if call.context.as_ref().and_then(|c| c.request_key.as_deref()) != Some(input.id.as_str())
            || !input.cells.iter().any(|c| c.cell.as_str() == call.cell_id)
        {
            return Err(Status::invalid_argument("qualification call identity"));
        }
        {
            let sessions = self
                .shared
                .sessions
                .lock()
                .map_err(|_| Status::unavailable("sessions"))?;
            let session = sessions
                .get(&caller.session)
                .ok_or_else(|| Status::unauthenticated("session"))?;
            if input
                .cells
                .iter()
                .any(|c| !session.cells.contains_key(&c.cell))
            {
                return Err(Status::failed_precondition(
                    "all qualification cohort cells must be negotiated",
                ));
            }
        }
        let v = self
            .run(move |h| h.accept_qualification(&caller, input).map_err(failure))
            .await?;
        #[cfg(feature = "test-harness")]
        if self
            .shared
            .lose_qualification_reply
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Status::unavailable(
                "injected qualification reply loss after commit",
            ));
        }
        payload(&v)
    }
}
