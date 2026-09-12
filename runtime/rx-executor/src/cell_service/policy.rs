//! Decisions over authenticated reads; call sites separately enforce view expiration.
use super::*;
use crate::{
    PeerPin,
    assignment_journal::Identity,
    journal::{Basis, Scope},
    lifecycle::VersionedStop,
    service::PlannerCleanup,
};
use rx_domain::canonical;
use rx_process_contract::{
    assignment,
    execution::{Purpose, RunState},
    production,
};

pub(super) enum Discovery {
    Idle,
    Arming(Id),
    Executing(Id),
}
fn invalid(message: &str) -> Error {
    Error::Invalid(message.into())
}
pub(super) fn observation_shutdown(status: &mut Status) {
    if status.attachment.is_some() || status.run.is_some() {
        status.phase = Phase::Attention;
        status.detail = Some(
            "shutdown while observing unresolved Run; original attachment/start retained".into(),
        );
    } else {
        status.phase = Phase::Stopped;
    }
}
pub(super) fn check_options(options: &service::Options) -> Result<(), Error> {
    if options.coordination != CoordinationMode::SerialProduction
        || !(10..=1000).contains(&options.poll_ms)
        || !(100..=60000).contains(&options.communication_grace_ms)
        || !(100..=60000).contains(&options.stop_timeout_ms)
    {
        return Err(invalid(
            "cell service requires serial production and bounded timing options",
        ));
    }
    Ok(())
}
pub(super) fn check_pin(identity: &Identity, pin: &PeerPin) -> Result<(), Error> {
    let scope = &identity.scope;
    if scope.installation != pin.installation
        || scope.store_generation != pin.store_generation
        || scope.principal != pin.principal
        || scope.release != pin.release
        || scope.cell != pin.cell
        || scope.definition != pin.definition
    {
        return Err(invalid("cell service and client scopes differ"));
    }
    Ok(())
}
pub(super) fn discover(view: &assignment::View) -> Result<Discovery, Error> {
    match view.cardinality {
        assignment::Cardinality::None => Ok(Discovery::Idle),
        assignment::Cardinality::Ambiguous => {
            Err(invalid("ambiguous assignment requires explicit resolution"))
        }
        assignment::Cardinality::Single => {
            let candidate = view
                .candidates
                .first()
                .ok_or_else(|| invalid("single candidate missing"))?;
            if !candidate.configuration_current
                || candidate.definition != view.definition
                || candidate.purpose != Some(Purpose::Production)
            {
                return Err(invalid(
                    "assignment configuration/purpose requires attention",
                ));
            }
            if candidate.state == RunState::Prepared {
                let pending = candidate
                    .pending_attempt
                    .as_ref()
                    .ok_or_else(|| invalid("start attempt missing"))?;
                if pending.executor_session != view.caller_session
                    || pending.valid_until.clock_id != view.checked_at.clock_id
                    || pending.valid_until.ticks_ns <= view.checked_at.ticks_ns
                {
                    return Err(invalid("old or expired start attempt requires attention"));
                }
                return Ok(Discovery::Arming(candidate.run.clone()));
            }
            if candidate.state != RunState::Executing
                || candidate.executor_session.as_ref() != Some(&view.caller_session)
                || candidate.mandate.is_none()
            {
                return Err(invalid(
                    "restricted or old-session assignment requires attention",
                ));
            }
            Ok(Discovery::Executing(candidate.run.clone()))
        }
    }
}
pub(super) fn prepare(
    identity: &Identity,
    discovery: &assignment::View,
    view: &production::View,
) -> Result<Preparation, Error> {
    let Discovery::Executing(run) = discover(discovery)? else {
        return Err(invalid("executing discovery required"));
    };
    let candidate = &discovery.candidates[0];
    if view.run.run.id != run
        || view.runtime_boot != discovery.runtime_boot
        || view.caller_session != discovery.caller_session
        || view.cell_epoch != discovery.cell_epoch
        || view.scope_epochs != discovery.scope_epochs
        || view.resolved != candidate.resolved
        || view.definition != candidate.definition
        || view.run.run.mandate != candidate.mandate
        || view.run.revision < candidate.revision
        || view.sequence < discovery.sequence
        || view.cell_revision < discovery.cell_revision
        || view.checked_at.clock_id != discovery.checked_at.clock_id
        || view.checked_at.ticks_ns < discovery.checked_at.ticks_ns
    {
        return Err(invalid("assignment changed before production attachment"));
    }
    let scope = &identity.scope;
    let preparation = Preparation {
        id: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
        run_journal: Id::new(uuid::Uuid::new_v4().to_string()).expect("UUID"),
        scope: Scope {
            installation: scope.installation.clone(),
            store_generation: scope.store_generation.clone(),
            principal: scope.principal.clone(),
            release: scope.release,
            cell: scope.cell.clone(),
            definition: scope.definition,
            run,
            resolved_digest: view.resolved.sha256,
        },
        executor_session: view.caller_session.clone(),
        epoch: view.cell_epoch,
        basis: Basis {
            runtime_boot: view.runtime_boot.clone(),
            sequence: view.sequence,
            run_revision: view.run.revision,
            cell_revision: view.cell_revision,
            checked_at: view.checked_at.clone(),
        },
    };
    check_existing(&preparation, view)?;
    Ok(preparation)
}
fn check_binding(preparation: &Preparation, view: &production::View) -> Result<(), Error> {
    let scope = &preparation.scope;
    if view.installation != scope.installation
        || view.store_generation != scope.store_generation
        || view.run.run.id != scope.run
        || view.run.run.cell != scope.cell
        || view.definition.sha256 != scope.definition
        || view.resolved.sha256 != scope.resolved_digest
        || view.run.run.recipe_digest != scope.resolved_digest
        || view.runtime_boot != preparation.basis.runtime_boot
        || view.caller_session != preparation.executor_session
        || view.run.run.executor_session.as_ref() != Some(&preparation.executor_session)
        || view.cell_epoch != preparation.epoch
        || view.run.run.purpose != Some(Purpose::Production)
        || view.sequence < preparation.basis.sequence
        || view.run.revision < preparation.basis.run_revision
        || view.cell_revision < preparation.basis.cell_revision
        || view.checked_at.clock_id != preparation.basis.checked_at.clock_id
        || view.checked_at.ticks_ns < preparation.basis.checked_at.ticks_ns
    {
        return Err(invalid(
            "current production view differs from original attachment",
        ));
    }
    Ok(())
}
pub(super) fn check_existing(
    preparation: &Preparation,
    view: &production::View,
) -> Result<(), Error> {
    check_binding(preparation, view)?;
    if view.run.run.state != RunState::Executing
        || !view.admission_allowed
        || view.run.run.mandate.is_none()
    {
        return Err(invalid(
            "attachment is not currently admitted EXECUTING production; automatic resume is unsupported",
        ));
    }
    Ok(())
}
pub(super) fn check_completion(
    preparation: &Preparation,
    view: &production::View,
) -> Result<(), Error> {
    check_binding(preparation, view)?;
    if view.run.run.state != RunState::Completed {
        return Err(invalid("fresh exact P COMPLETED view required"));
    }
    Ok(())
}
pub(super) fn same_preparation(a: &Preparation, b: &Preparation) -> Result<(), Error> {
    if bytes(a)? != bytes(b)? {
        return Err(invalid("current attachment identity changed"));
    }
    Ok(())
}
pub(super) fn check_exit(
    preparation: &Preparation,
    report: &service::Report,
    cleanup: PlannerCleanup,
    saved: Option<&VersionedStop>,
) -> Result<(), Error> {
    if !cleanup.is_confirmed()
        || report.durability_fault.is_some()
        || report.phase != StopPhase::PauseObserved
        || report.stop.phase != report.phase
        || report.stop.reason != StopReason::Completed
        || report.stop.run != preparation.scope.run
        || report.stop_id != report.stop.id
        || report.stop.origin_session != preparation.executor_session
        || report.stop.origin_context.as_ref().is_some_and(|c| {
            c.run != preparation.scope.run
                || c.executor_session != preparation.executor_session
                || c.epoch != preparation.epoch
                || c.resolved_digest != preparation.scope.resolved_digest
        })
        || report
            .stop
            .observation
            .as_ref()
            .is_none_or(|v| v.state != RunState::Completed)
    {
        return Err(invalid(
            "normal completion and confirmed planner cleanup required",
        ));
    }
    let saved = saved.ok_or_else(|| invalid("durable completion stop record missing"))?;
    if bytes(&saved.record)? != bytes(&report.stop)? {
        return Err(invalid(
            "durable stop record differs from returned completion",
        ));
    }
    Ok(())
}
fn bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    canonical::bytes(value).map_err(|e| Error::Invalid(e.to_string()))
}
pub(super) fn transient(error: &Error) -> bool {
    matches!(error, Error::Expired)
        || matches!(error, Error::Rpc(status) if matches!(status.code(), tonic::Code::Unavailable
            | tonic::Code::DeadlineExceeded | tonic::Code::Cancelled | tonic::Code::ResourceExhausted
            | tonic::Code::Internal | tonic::Code::Unknown))
}
