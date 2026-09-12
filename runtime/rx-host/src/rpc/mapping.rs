use super::*;
pub(super) fn rpc_identity<M: prost::Message + prost::Name>(
    message: &M,
    context: Option<&base::CallContext>,
) -> std::result::Result<(Id, Digest), Status> {
    let key = parse_id(
        context
            .and_then(|c| c.request_key.as_deref())
            .ok_or_else(|| Status::invalid_argument("mutation key required"))?,
    )?;
    let mut value = rx_protocol::json::to_value(message)?;
    normalize(&mut value);
    let hash = canonical::digest("RX-HOST-RPC-v1", &value)
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
    Ok((key, hash))
}
fn normalize(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.retain(|key, _| key != "context" && !key.starts_with("expected_"));
            for (key, value) in object {
                normalize(value);
                if let Some(array) = value.as_array_mut() {
                    if matches!(
                        key.as_str(),
                        "block_ids"
                            | "clearance_ids"
                            | "block_ids_to_clear"
                            | "scope_ids"
                            | "condition_ids"
                            | "source_ids"
                    ) {
                        array.sort_by_cached_key(|v| v.to_string());
                    } else if key == "scopes" {
                        array.sort_by_cached_key(|v| v["scope_id"].to_string());
                    } else if key == "conditions" {
                        array.sort_by_cached_key(|v| v["condition_id"].to_string());
                    }
                }
            }
        }
        serde_json::Value::Array(array) => {
            for value in array {
                normalize(value);
            }
        }
        _ => {}
    }
}
pub(super) fn claim<N: NativeAdapter, C: Clock>(
    host: &Host<N, C>,
    caller: &Caller,
    method: &str,
    rpc: &(Id, Digest),
    effect: Option<String>,
) -> std::result::Result<(), Status> {
    let effect = effect.map(|e| parse_name(&e)).transpose()?;
    host.claim_rpc(caller, &parse_name(method)?, &rpc.0, rpc.1, effect.as_ref())
        .map_err(failure)
}
pub(super) fn parse_id(value: &str) -> std::result::Result<Id, Status> {
    Id::new(value).map_err(|e| Status::invalid_argument(e.to_string()))
}
pub(super) fn parse_name(value: &str) -> std::result::Result<Name, Status> {
    Name::new(value).map_err(|e| Status::invalid_argument(e.to_string()))
}
pub(super) fn digest(value: &[u8]) -> std::result::Result<Digest, Status> {
    Ok(Digest::from_bytes(value.try_into().map_err(|_| {
        Status::invalid_argument("Digest must have 32 bytes")
    })?))
}
pub(super) fn time(
    value: Option<&base::TimePoint>,
) -> std::result::Result<rx_domain::types::TimePoint, Status> {
    let value = value.ok_or_else(|| Status::invalid_argument("TimePoint required"))?;
    if value.clock_id.is_empty() {
        return Err(Status::invalid_argument("clock ID required"));
    }
    Ok(rx_domain::types::TimePoint {
        clock_id: value.clock_id.clone(),
        ticks_ns: rx_domain::types::Counter(value.ticks_ns),
    })
}
pub(super) fn ids(values: &[String]) -> std::result::Result<BTreeSet<Id>, Status> {
    let set: BTreeSet<_> = values
        .iter()
        .map(|s| parse_id(s))
        .collect::<std::result::Result<_, _>>()?;
    if set.len() != values.len() {
        return Err(Status::invalid_argument("duplicate ID"));
    }
    Ok(set)
}
pub(super) fn names(values: &[String]) -> std::result::Result<Vec<Name>, Status> {
    let mut values: Vec<_> = values
        .iter()
        .map(|s| parse_name(s))
        .collect::<std::result::Result<_, _>>()?;
    values.sort();
    if values.windows(2).any(|v| v[0] == v[1]) {
        return Err(Status::invalid_argument("duplicate name"));
    }
    Ok(values)
}
pub(super) fn scopes(
    reference: &cell::CellRef,
) -> std::result::Result<BTreeMap<Name, rx_domain::types::Counter>, Status> {
    if reference.cell_epoch == 0 || reference.scopes.is_empty() {
        return Err(Status::invalid_argument(
            "positive cell/scope epoch required",
        ));
    }
    let mut result = BTreeMap::new();
    for scope in &reference.scopes {
        if scope.epoch == 0
            || result
                .insert(
                    parse_name(&scope.scope_id)?,
                    rx_domain::types::Counter(scope.epoch),
                )
                .is_some()
        {
            return Err(Status::invalid_argument("invalid scope vector"));
        }
    }
    Ok(result)
}
pub(super) fn cell_ref(id: &Name, state: &CellState) -> cell::CellRef {
    cell::CellRef {
        cell_id: id.to_string(),
        cell_epoch: state.epoch.0,
        scopes: state
            .scopes
            .iter()
            .map(|(id, e)| cell::ScopeEpoch {
                scope_id: id.to_string(),
                epoch: e.0,
            })
            .collect(),
    }
}
pub(super) fn grant_view(grant: &StoredGrant) -> std::result::Result<base::Grant, Status> {
    if !grant.ttl_ns.0.is_multiple_of(1_000_000) {
        return Err(Status::failed_precondition(
            "grant TTL is not a whole millisecond",
        ));
    }
    Ok(base::Grant {
        grant_id: grant.id.to_string(),
        host_boot_id: grant.host_boot.to_string(),
        fence: grant.fence.0,
        resource_set: grant.resources.iter().map(ToString::to_string).collect(),
        ttl_ms: grant.ttl_ns.0 / 1_000_000,
        owner_id: grant.owner.to_string(),
    })
}
pub(super) fn failure(error: HostError) -> Status {
    match error {
        HostError::Forbidden => Status::permission_denied(error.to_string()),
        HostError::Conflict => Status::already_exists(error.to_string()),
        HostError::NotFound => Status::not_found(error.to_string()),
        HostError::Store(rx_ports::StoreError::Invalid(detail)) => {
            Status::failed_precondition(detail)
        }
        HostError::Store(rx_ports::StoreError::KeyConflict) => {
            Status::already_exists("key conflict")
        }
        HostError::Store(_) => Status::unavailable(error.to_string()),
        HostError::Invalid(_) => Status::invalid_argument(error.to_string()),
        _ => Status::failed_precondition(error.to_string()),
    }
}
pub(super) fn request_from_wire<N: NativeAdapter, C: Clock>(
    host: &Host<N, C>,
    caller: &Caller,
    operation: &str,
    intent: &base::Intent,
    intent_digest: &[u8],
    grant: &base::Grant,
    permit: &cell::DispatchPermit,
) -> std::result::Result<crate::Request, Status> {
    let operation = parse_id(operation)?;
    let intent = rx_protocol::json::intent_to_domain(intent)?;
    let supplied = digest(intent_digest)?;
    if intent
        .digest()
        .map_err(|e| Status::invalid_argument(e.to_string()))?
        != supplied
        || permit.operation_id != operation.as_str()
        || digest(&permit.intent_digest)? != supplied
        || permit.state != cell::PermitState::Issued as i32
    {
        return Err(Status::invalid_argument(
            "operation/permit identity mismatch",
        ));
    }
    let stored = host
        .stored_grant(caller, &parse_id(&grant.grant_id)?)
        .map_err(failure)?;
    if grant_view(&stored)? != *grant || permit.grant.as_ref() != Some(grant) {
        return Err(Status::invalid_argument("grant body mismatch"));
    }
    let target = permit
        .cell
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("permit cell required"))?;
    let scope_map = scopes(target)?;
    let expires = time(permit.expires_at.as_ref())?;
    let issued = time(permit.issued_at.as_ref())?;
    if issued.clock_id != expires.clock_id || issued.ticks_ns >= expires.ticks_ns {
        return Err(Status::invalid_argument("permit validity interval"));
    }
    let purpose = match cell::Purpose::try_from(permit.purpose) {
        Ok(cell::Purpose::Production) => Purpose::Production,
        Ok(cell::Purpose::Setup) => Purpose::Setup,
        Ok(cell::Purpose::Recovery) => Purpose::Recovery,
        _ => return Err(Status::invalid_argument("invalid purpose")),
    };
    let parent = match permit.parent.as_ref().and_then(|p| p.value.as_ref()) {
        Some(cell::permit_parent::Value::MandateId(id)) => PermitParent::Mandate(parse_id(id)?),
        Some(cell::permit_parent::Value::Recovery(r)) => PermitParent::Recovery {
            case: parse_id(&r.case_id)?,
            plan: digest(
                &r.plan
                    .as_ref()
                    .ok_or_else(|| Status::invalid_argument("recovery plan required"))?
                    .sha256,
            )?,
            step: parse_name(&r.step_id)?,
            visit: rx_domain::types::Counter(r.visit),
        },
        None => return Err(Status::invalid_argument("permit parent required")),
    };
    let mut conditions = BTreeSet::new();
    for condition in &permit.conditions {
        let until = time(condition.valid_until.as_ref())?;
        if condition.verdict != cell::Verdict::Pass as i32
            || condition.condition_revision == 0
            || condition.cell.as_ref() != Some(target)
            || condition.evidence_ids.is_empty()
            || ids(&condition.evidence_ids)?.is_empty()
            || until.clock_id != expires.clock_id
            || until.ticks_ns < expires.ticks_ns
            || !conditions.insert(parse_name(&condition.condition_id)?)
        {
            return Err(Status::invalid_argument("permit condition proof mismatch"));
        }
    }
    let mut wire_permit = rx_protocol::json::to_value(permit)?;
    normalize(&mut wire_permit);
    let source_digest = canonical::digest("RX-HOST-WIRE-PERMIT-v1", &wire_permit)
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
    Ok(crate::Request {
        operation: operation.clone(),
        intent,
        digest: supplied,
        grant: stored.id.clone(),
        permit: Permit {
            id: parse_id(&permit.permit_id)?,
            operation,
            digest: supplied,
            cell: parse_name(&target.cell_id)?,
            epoch: rx_domain::types::Counter(target.cell_epoch),
            scopes: scope_map,
            envelope: digest(&permit.envelope_digest)?,
            qualification: parse_id(&permit.qualification_id)?,
            qualification_revision: rx_domain::types::Counter(permit.qualification_revision),
            grant: stored.id,
            host_boot: parse_id(&permit.host_boot_id)?,
            conditions,
            expires_at: expires,
            purpose,
            parent,
            source_digest: Some(source_digest),
        },
    })
}
pub(crate) fn evidence_view(record: EvidenceRecord) -> base::Evidence {
    base::Evidence {
        evidence_id: record.evidence_id.to_string(),
        evidence_schema: "rx.native-result.v1".into(),
        profile_digest: record.profile_digest.as_bytes().to_vec(),
        body: Some(base::EvidenceBody {
            value: Some(base::evidence_body::Value::NativeResult(
                base::NativeResult {
                    correlation: Some(base::Correlation {
                        operation_id: Some(record.operation.to_string()),
                        invocation_id: Some(record.invocation.to_string()),
                        native_id: record.capture.native_id,
                        device_session_id: record.capture.device_session.to_string(),
                        profile_digest: record.profile_digest.as_bytes().to_vec(),
                        cancel_id: None,
                    }),
                    native_status_schema: record.capture.status_schema.to_string(),
                    native_status: record.capture.status,
                    native_data: None,
                    captured_at: Some(base::TimePoint {
                        clock_id: record.capture.captured_at.clock_id,
                        ticks_ns: record.capture.captured_at.ticks_ns.0,
                    }),
                },
            )),
        }),
    }
}
