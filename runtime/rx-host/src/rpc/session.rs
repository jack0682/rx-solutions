use super::*;

#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static> base::session_service_server::SessionService
    for RpcHost<N, C>
{
    async fn open(
        &self,
        request: RpcRequest<base::PeerHello>,
    ) -> std::result::Result<Response<base::Session>, Status> {
        let (fingerprint, peer) = self.certificate(&request)?;
        let hello = request.into_inner();
        if hello.peer_id != peer.as_str()
            || hello.role != base::Role::Platform as i32
            || hello.installation_id != self.shared.configuration.installation.as_str()
            || digest(&hello.release_digest)? != self.shared.configuration.release_digest
            || hello.shared_clock_id != self.shared.host.current_time().clock_id
            || !hello
                .supported_versions
                .iter()
                .any(|v| v.major == 1 && v.minor == 0 && v.schema_hash == base_manifest_hash())
        {
            return Err(Status::failed_precondition(
                "peer/release/clock/base contract mismatch",
            ));
        }
        parse_id(&hello.boot_id)?;
        parse_id(&hello.store_generation)?;
        let hello_digest =
            canonical::digest("RX-HOST-HELLO-v1", &rx_protocol::json::to_value(&hello)?)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let handshake = self.shared.handshake.clone().lock_owned().await;
        let shared = self.shared.clone();
        let session = self
            .run(move |host| {
                let _handshake = handshake;
                let existing = {
                    let sessions = shared
                        .sessions
                        .lock()
                        .map_err(|_| Status::unavailable("session registry"))?;
                    sessions
                        .values()
                        .find(|s| s.fingerprint == fingerprint && s.hello_digest == hello_digest)
                        .map(|s| s.session.clone())
                };
                if let Some(existing) = existing {
                    return Ok(existing);
                }
                let session = base::Session {
                    session_id: crate::journal::id().to_string(),
                    peer_id: peer.to_string(),
                    boot_id: hello.boot_id,
                    selected_version: Some(base::Version {
                        major: 1,
                        minor: 0,
                        schema_hash: base_manifest_hash(),
                    }),
                    required_features: vec!["rx.cell.v1".into(), "strict-wire-v1".into()],
                    limits: Some(base::Limits {
                        max_message_bytes: 1_048_576,
                        max_batch_records: 128,
                        subscriber_buffer_bytes: 4_194_304,
                        max_inflight: 32,
                    }),
                };
                let caller = Caller {
                    peer: peer.clone(),
                    session: parse_id(&session.session_id)?,
                };
                host.bind_platform(caller).map_err(failure)?;
                let mut sessions = shared
                    .sessions
                    .lock()
                    .map_err(|_| Status::unavailable("session registry"))?;
                sessions.retain(|_, s| s.session.peer_id != peer.as_str());
                sessions.insert(
                    parse_id(&session.session_id)?,
                    SessionState {
                        fingerprint,
                        hello_digest,
                        session: session.clone(),
                        cells: BTreeMap::new(),
                    },
                );
                Ok(session)
            })
            .await?;
        Ok(Response::new(session))
    }
}

#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static> cell::cell_service_server::CellService
    for RpcHost<N, C>
{
    async fn open(
        &self,
        request: RpcRequest<cell::CellHello>,
    ) -> std::result::Result<Response<cell::CellSession>, Status> {
        let (fingerprint, peer) = self.certificate(&request)?;
        let hello = request.into_inner();
        let session_id = parse_id(&hello.base_session_id)?;
        if hello.base_manifest_hash != base_manifest_hash()
            || hello.cell_manifest_hash != cell_manifest_hash()
            || hello.peer_id != peer.as_str()
            || hello.shared_clock_id != self.shared.host.current_time().clock_id
        {
            return Err(Status::failed_precondition(
                "UPGRADE_REQUIRED: cell manifest/clock mismatch",
            ));
        }
        let matching: Vec<_> = self
            .shared
            .bindings
            .iter()
            .filter(|b| b.definition.sha256.as_bytes().as_slice() == hello.cell_definition_digest)
            .collect();
        if matching.len() != 1 {
            return Err(Status::failed_precondition(
                "unknown or ambiguous CellDefinition",
            ));
        }
        let cell_id = matching[0].cell.clone();
        let mut sessions = self
            .shared
            .sessions
            .lock()
            .map_err(|_| Status::unavailable("session registry"))?;
        let base = sessions
            .get_mut(&session_id)
            .ok_or_else(|| Status::unauthenticated("base session required"))?;
        if base.fingerprint != fingerprint || base.session.peer_id != peer.as_str() {
            return Err(Status::unauthenticated("session/certificate mismatch"));
        }
        let result = base
            .cells
            .entry(cell_id)
            .or_insert_with(|| cell::CellSession {
                session_id: crate::journal::id().to_string(),
                base_session_id: session_id.to_string(),
                peer_id: peer.to_string(),
                manifest_hash: cell_manifest_hash(),
            })
            .clone();
        Ok(Response::new(result))
    }
    async fn inspect(
        &self,
        _: RpcRequest<cell::CellCall>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(wrong_owner())
    }

    async fn evaluate(
        &self,
        _: RpcRequest<cell::EvaluateRequest>,
    ) -> std::result::Result<Response<cell::EvaluationSet>, Status> {
        Err(wrong_owner())
    }

    async fn start_run(
        &self,
        _: RpcRequest<cell::StartRunRequest>,
    ) -> std::result::Result<Response<cell::StartAttempt>, Status> {
        Err(wrong_owner())
    }

    async fn get_start_attempt(
        &self,
        _: RpcRequest<cell::GetStartAttemptRequest>,
    ) -> std::result::Result<Response<cell::StartAttempt>, Status> {
        Err(wrong_owner())
    }

    async fn begin_part_attempt(
        &self,
        _: RpcRequest<cell::BeginPartAttemptRequest>,
    ) -> std::result::Result<Response<cell::PartAttempt>, Status> {
        Err(wrong_owner())
    }

    async fn submit_operation(
        &self,
        _: RpcRequest<cell::SubmitOperationRequest>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        Err(wrong_owner())
    }

    async fn hold(
        &self,
        _: RpcRequest<cell::HoldRequest>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(wrong_owner())
    }

    async fn clear_transient_block(
        &self,
        _: RpcRequest<cell::ClearTransientBlockRequest>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(wrong_owner())
    }

    async fn open_case(
        &self,
        _: RpcRequest<cell::OpenCaseRequest>,
    ) -> std::result::Result<Response<cell::InterventionCase>, Status> {
        Err(wrong_owner())
    }

    async fn record_procedure(
        &self,
        _: RpcRequest<cell::RecordProcedureRequest>,
    ) -> std::result::Result<Response<cell::InterventionCase>, Status> {
        Err(wrong_owner())
    }

    async fn set_recovery_plan(
        &self,
        _: RpcRequest<cell::SetRecoveryPlanRequest>,
    ) -> std::result::Result<Response<cell::InterventionCase>, Status> {
        Err(wrong_owner())
    }

    async fn prepare_restart(
        &self,
        _: RpcRequest<cell::PrepareRestartRequest>,
    ) -> std::result::Result<Response<cell::RestartPreparation>, Status> {
        Err(wrong_owner())
    }

    async fn get_restart_preparation(
        &self,
        _: RpcRequest<cell::GetRestartPreparationRequest>,
    ) -> std::result::Result<Response<cell::RestartPreparation>, Status> {
        Err(wrong_owner())
    }

    async fn prepare_close(
        &self,
        _: RpcRequest<cell::PrepareCloseRequest>,
    ) -> std::result::Result<Response<cell::Clearance>, Status> {
        Err(wrong_owner())
    }

    async fn restart_run(
        &self,
        _: RpcRequest<cell::RestartRunRequest>,
    ) -> std::result::Result<Response<cell::StartAttempt>, Status> {
        Err(wrong_owner())
    }

    async fn close_without_restart(
        &self,
        _: RpcRequest<cell::CloseWithoutRestartRequest>,
    ) -> std::result::Result<Response<cell::CellContext>, Status> {
        Err(wrong_owner())
    }

    async fn record_change(
        &self,
        _: RpcRequest<cell::RecordChangeRequest>,
    ) -> std::result::Result<Response<cell::ChangeRecord>, Status> {
        Err(wrong_owner())
    }

    async fn register_qualification(
        &self,
        _: RpcRequest<cell::RegisterQualificationRequest>,
    ) -> std::result::Result<Response<cell::Qualification>, Status> {
        Err(wrong_owner())
    }

    async fn get_case(
        &self,
        _: RpcRequest<cell::GetCaseRequest>,
    ) -> std::result::Result<Response<cell::InterventionCase>, Status> {
        Err(wrong_owner())
    }

    async fn get_material(
        &self,
        _: RpcRequest<cell::GetMaterialRequest>,
    ) -> std::result::Result<Response<cell::MaterialState>, Status> {
        Err(wrong_owner())
    }
}
