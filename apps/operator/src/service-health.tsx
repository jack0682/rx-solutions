import type { ServiceHealth } from './service-health-schema';
export function servicePresentation(health: ServiceHealth | null | undefined, frameFresh: boolean) {
  if (!health) return { summary: 'Diagnostics unavailable', current: false };
  if (health.availability === 'NOT_CONFIGURED')
    return { summary: 'Execution service not configured', current: false };
  if (health.availability === 'WAITING_REPORT')
    return { summary: 'Awaiting first status report', current: false };
  if (health.availability === 'CONTEXT_MISMATCH')
    return { summary: 'Configuration changed · service check required', current: false };
  if (!frameFresh || health.availability !== 'FRESH')
    return { summary: 'Recent status needs verification', current: false };
  return { summary: 'Status report received', current: true };
}
const connection: Record<string, string> = {
  WAITING_FOR_PEER: 'Awaiting Host authentication',
  CONNECTING: 'Checking connection',
  BOUND: 'Connection registered',
  ATTENTION: 'Connection or renewal check required',
  STOPPED: 'Service stopped',
};
const observation: Record<string, string> = {
  WAITING: 'Awaiting observation',
  RECEIVED: 'Observation response received',
  UNAVAILABLE: 'Observation query needs verification',
  INTEGRITY_ATTENTION: 'Evidence continuity needs verification',
  STOPPED: 'Inspection service stopped',
};
export function ServiceHealthCard({
  host,
  health,
  frameFresh,
}: {
  host: string;
  health: ServiceHealth | null | undefined;
  frameFresh: boolean;
}) {
  const presentation = servicePresentation(health, frameFresh);
  const active = presentation.current ? health : null;
  const observing = active?.observation === 'RECEIVED' && active.observation_recent;
  const delivering = active?.delivery === 'ACTIVE' && active.delivery_recent;
  return (
    <article className="service-health" data-service-host={host}>
      <div className="source-row-heading">
        <b>{host}</b>
        <span className={`badge ${presentation.current ? '' : 'warning'}`}>
          {presentation.summary}
        </span>
      </div>
      <div className="service-states">
        <div>
          <small>Connection and lease coordination</small>
          <strong>
            {active && active.connection ? connection[active.connection] : 'Needs verification'}
          </strong>
        </div>
        <div>
          <small>Observation query</small>
          <strong>
            {active && active.observation
              ? active.observation === 'RECEIVED' && !observing
                ? 'Recent observation response needs verification'
                : observation[active.observation]
              : 'Needs verification'}
          </strong>
        </div>
        <div>
          <small>Command dispatch loop</small>
          <strong>
            {active
              ? active.delivery === 'STOPPED'
                ? 'Dispatch service stopped'
                : active.delivery === 'NOT_STARTED'
                  ? 'Not started yet'
                  : delivering
                    ? 'Dispatch loop verified'
                    : 'Recent loop activity needs verification'
              : 'Needs verification'}
          </strong>
        </div>
      </div>
      {health?.delivery_error_history && (
        <p className="service-history">
          Dispatch errors are recorded. Check run records for the current operation outcome and
          reasons for action.
        </p>
      )}
      <p className="muted">
        Receiving a status report does not establish device readiness. A registered connection does
        not guarantee recent successful communication.
      </p>
    </article>
  );
}
