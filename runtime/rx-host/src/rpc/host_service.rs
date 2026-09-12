use super::*;
use rx_domain::types::Counter;

#[tonic::async_trait]
impl<N: NativeAdapter + 'static, C: Clock + 'static> base::host_service_server::HostService
    for RpcHost<N, C>
{
    async fn acquire_grant(
        &self,
        request: RpcRequest<base::GrantRequest>,
    ) -> std::result::Result<Response<base::Grant>, Status> {
        let caller = self.caller(&request, request.get_ref().context.as_ref(), None, true)?;
        let rpc = rpc_identity(request.get_ref(), request.get_ref().context.as_ref())?;
        let input = request.into_inner();
        if input.owner_id != caller.peer.as_str() {
            return Err(Status::permission_denied("grant owner mismatch"));
        }
        let resources = names(&input.resource_set)?;
        let key = parse_id(
            input
                .context
                .as_ref()
                .and_then(|c| c.request_key.as_deref())
                .ok_or_else(|| Status::invalid_argument("key"))?,
        )?;
        let ttl = input
            .requested_ttl_ms
            .checked_mul(1_000_000)
            .ok_or_else(|| Status::invalid_argument("duration overflow"))?;
        {
            let sessions = self
                .shared
                .sessions
                .lock()
                .map_err(|_| Status::unavailable("sessions"))?;
            let session = sessions
                .get(&caller.session)
                .ok_or_else(|| Status::unauthenticated("session"))?;
            let permitted: BTreeSet<_> = self
                .shared
                .bindings
                .iter()
                .filter(|b| session.cells.contains_key(&b.cell))
                .flat_map(|b| {
                    b.allowed_intents
                        .iter()
                        .flat_map(|i| i.resource_set.iter().cloned())
                })
                .collect();
            if resources.iter().any(|r| !permitted.contains(r)) {
                return Err(Status::failed_precondition(
                    "cell contract not negotiated for resource",
                ));
            }
        }
        let output = self
            .run(move |host| {
                claim(&host, &caller, "Host.AcquireGrant", &rpc, None)?;
                grant_view(
                    &host
                        .acquire_grant(&caller, key, resources, Counter(input.fence), Counter(ttl))
                        .map_err(failure)?,
                )
            })
            .await?;
        Ok(Response::new(output))
    }
    async fn renew_grant(
        &self,
        request: RpcRequest<base::RenewGrant>,
    ) -> std::result::Result<Response<base::Grant>, Status> {
        let caller = self.caller(&request, request.get_ref().context.as_ref(), None, true)?;
        let rpc = rpc_identity(request.get_ref(), request.get_ref().context.as_ref())?;
        let input = request.into_inner();
        let grant = parse_id(&input.grant_id)?;
        let result = self
            .run(move |host| {
                claim(
                    &host,
                    &caller,
                    "Host.RenewGrant",
                    &rpc,
                    Some(format!("renew/{}/{}", grant, input.renew_seq)),
                )?;
                grant_view(
                    &host
                        .renew_grant(&caller, &grant, Counter(input.renew_seq))
                        .map_err(failure)?,
                )
            })
            .await?;
        Ok(Response::new(result))
    }
    async fn get_receipt(
        &self,
        request: RpcRequest<base::OperationRef>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        let caller = self.caller(&request, request.get_ref().context.as_ref(), None, false)?;
        let operation = parse_id(&request.into_inner().operation_id)?;
        Ok(Response::new(
            self.run(move |host| host.receipt_view(&caller, &operation).map_err(failure))
                .await?,
        ))
    }
    async fn reconcile(
        &self,
        request: RpcRequest<base::OperationRef>,
    ) -> std::result::Result<Response<base::EvidenceBatch>, Status> {
        let caller = self.caller(&request, request.get_ref().context.as_ref(), None, false)?;
        let input = request.into_inner();
        let operation = parse_id(&input.operation_id)?;
        Ok(Response::new(
            self.run(move |host| {
                host.reconcile(&caller, &operation).map_err(failure)?;
                let meta = host.journals().map_err(failure)?;
                let records = host
                    .evidence_after(&caller, Counter(0), 128)
                    .map_err(failure)?;
                Ok(base::EvidenceBatch {
                    context: input.context,
                    producer_journal_id: meta.evidence_journal.to_string(),
                    first_seq: records.first().map(|(n, _)| n.0).unwrap_or(1),
                    records: records.into_iter().map(|(_, r)| evidence_view(r)).collect(),
                })
            })
            .await?,
        ))
    }
    type WatchObservationsStream =
        tokio_stream::wrappers::ReceiverStream<std::result::Result<base::ObservationBatch, Status>>;
    async fn watch_observations(
        &self,
        request: RpcRequest<base::ObservationQuery>,
    ) -> std::result::Result<Response<Self::WatchObservationsStream>, Status> {
        let caller = self.caller(&request, request.get_ref().context.as_ref(), None, false)?;
        let input = request.into_inner();
        if input.value_schemas != ["rx.handover.v1"] || input.source_ids.len() != 3 {
            return Err(Status::invalid_argument("handover source set required"));
        }
        let mut operation = None;
        let mut kinds = BTreeSet::new();
        for source in input.source_ids {
            let parts: Vec<_> = source.split('/').collect();
            if parts.len() != 3
                || parts[0] != "handover"
                || !["no-pending", "control", "support"].contains(&parts[2])
            {
                return Err(Status::invalid_argument("unsupported observation source"));
            }
            let id = parse_id(parts[1])?;
            if operation.as_ref().is_some_and(|old| old != &id)
                || !kinds.insert(parts[2].to_string())
            {
                return Err(Status::invalid_argument("mixed/duplicate handover sources"));
            }
            operation = Some(id);
        }
        let operation = operation.ok_or_else(|| Status::invalid_argument("operation required"))?;
        let observations = self
            .run(move |host| {
                host.handover_observations(&caller, &operation)
                    .map_err(failure)
            })
            .await?;
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender
            .send(Ok(base::ObservationBatch {
                observations,
                dropped_count: 0,
            }))
            .await
            .map_err(|_| Status::unavailable("observation receiver"))?;
        drop(sender);
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }
    async fn prepare_operation(
        &self,
        _: RpcRequest<base::PrepareOperation>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn authorize_dispatch(
        &self,
        _: RpcRequest<base::AuthorizeDispatch>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn get_cancel_receipt(
        &self,
        _: RpcRequest<base::CancelRequest>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn cancel_operation(
        &self,
        _: RpcRequest<base::CancelRequest>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn revoke_grant(
        &self,
        _: RpcRequest<base::RevokeGrantRequest>,
    ) -> std::result::Result<Response<base::GrantRevocation>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn close_control_session(
        &self,
        _: RpcRequest<base::CancelRequest>,
    ) -> std::result::Result<Response<base::Receipt>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn next_stream_ticket(
        &self,
        _: RpcRequest<base::OperationRef>,
    ) -> std::result::Result<Response<base::StreamTicket>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }

    async fn push_sample(
        &self,
        _: RpcRequest<base::ControlSample>,
    ) -> std::result::Result<Response<base::Reason>, Status> {
        Err(Status::failed_precondition(
            "UPGRADE_REQUIRED or capability not advertised by this binding",
        ))
    }
}
