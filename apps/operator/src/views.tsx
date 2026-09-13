import type { CellOverview } from './schema';
import { short, text } from './labels';

export function Runs({ cell }: { cell: CellOverview }) {
  return (
    <section className="panel records">
      <div className="section-heading">
        <div>
          <p className="eyebrow">RUN RECORDS</p>
          <h3>Run records</h3>
        </div>
        <span className="count">
          {cell.runs.length}
          {cell.runs_truncated ? '+' : ''}
        </span>
      </div>
      {cell.runs.length ? (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Run ID</th>
                <th>State</th>
                <th>Purpose</th>
                <th>Attempt budget usage</th>
                <th>Record revision</th>
              </tr>
            </thead>
            <tbody>
              {cell.runs.map(({ revision, value: r }) => (
                <tr key={r.id}>
                  <td className="mono" title={r.id}>
                    {short(r.id)}
                  </td>
                  <td>
                    <span className={`badge ${r.state === 'RECOVERY_REQUIRED' ? 'warning' : ''}`}>
                      {text(r.state)}
                    </span>
                  </td>
                  <td>
                    {r.purpose === 'PRODUCTION'
                      ? 'Production'
                      : r.purpose === 'SETUP'
                        ? 'Setup'
                        : 'Set at start'}
                  </td>
                  <td>
                    {r.budget
                      ? `${r.budget.unit === 'PART_ATTEMPT' ? 'Material' : 'Operation'} ${r.budget.consumptions.length} / ${r.budget.limit}`
                      : 'Not specified'}
                  </td>
                  <td>r{revision}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="empty-inline">
          <span>↗</span>
          <div>
            <b>No run records yet</b>
            <p>Prepare a run to view its request and current state here.</p>
          </div>
        </div>
      )}
      {cell.runs_truncated && (
        <p className="muted">
          Displays up to 50 records. Full history browsing is planned separately.
        </p>
      )}
    </section>
  );
}
export function Work({ cell }: { cell: CellOverview }) {
  return (
    <section className="panel records">
      <div className="section-heading">
        <div>
          <p className="eyebrow">OPERATION EVIDENCE</p>
          <h3>Operation outcomes and resource handover</h3>
        </div>
      </div>
      <p className="muted">
        Review execution observations, outcome determinations, and resource handover separately.
      </p>
      {!cell.work.length ? (
        <p className="empty-inline">No device operations are recorded.</p>
      ) : (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Operation</th>
                <th>Execution observed</th>
                <th>Outcome</th>
                <th>Record integrity</th>
                <th>Resource disposition</th>
              </tr>
            </thead>
            <tbody>
              {cell.work.map((w) => (
                <tr key={w.operation.operation_id}>
                  <td className="mono" title={w.operation.operation_id}>
                    {short(w.operation.operation_id)}
                  </td>
                  <td>{text(w.operation.execution_knowledge)}</td>
                  <td>{text(w.operation.outcome)}</td>
                  <td>{text(w.operation.integrity)}</td>
                  <td>{text(w.operation.disposition)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {cell.work_truncated && (
        <p className="muted">
          Displays up to 100 operations. Do not infer the total operation count from this list.
        </p>
      )}
    </section>
  );
}
export function Configuration({ cell }: { cell: CellOverview }) {
  const c = cell.cell.value;
  return (
    <section className="panel config-panel">
      <p className="eyebrow">BOUND CONFIGURATION</p>
      <h2>Currently registered configuration</h2>
      <p className="muted">
        These identifiers refer to documents bound to execution. Changes require a new configuration
        and verification procedure.
      </p>
      <dl className="config-list">
        {[
          ['Cell', c.id],
          ['Environment', c.environment === 'SIMULATION' ? 'Simulation' : 'Physical equipment'],
          ['RX operating mode', text(c.mode ?? 'UNKNOWN')],
          ['Operating qualification state', text(c.commissioning ?? 'UNKNOWN')],
          ['Cell definition', c.definition.sha256],
          ['Operating envelope', c.envelope.sha256],
          ['Workflow', c.recipe.sha256],
          ['Site configuration', c.site_config_digest],
          ['Connected Hosts', c.hosts.join(', ')],
        ].map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="inset">
        <b>Configuration registration and verification are separate</b>
        <p>
          This screen displays the registered configuration. Workflow editing, verification reports,
          and deployment screens are connected in subsequent stages.
        </p>
      </div>
    </section>
  );
}
