import { ServiceHealthCard } from './service-health';
import { useEffect, useState } from 'react';
import type { CellOverview } from './schema';
import type { Diagnostics, Expression, Value } from './diagnostics-schema';
import { diagnosticDeadline, diagnosticFresh } from './diagnostic-time';
import { count, text, short } from './labels';
const issueText: Record<string, string> = {
  MISSING_OBSERVATION: 'Not observed yet',
  HOST_UNREGISTERED: 'Host registration required',
  GENERATION_UNREGISTERED: 'Observation source generation not registered',
  GENERATION_MISMATCH: 'Device generation changed',
  WRONG_SOURCE_HOST: 'Observation is not from the designated device',
  SCHEMA_MISMATCH: 'Observation type mismatch',
  UNIT_MISMATCH: 'Unit mismatch',
  BAD_QUALITY: 'Observation quality needs verification',
  ORIGIN_AGE_UNBOUNDED: 'Acquisition time cannot be guaranteed',
  DISPUTED: 'Contradictory evidence',
  CLOCK_MISMATCH: 'Clock basis mismatch',
  FUTURE_TIMESTAMP: 'Acquisition time needs verification',
  UNCERTAINTY_EXCEEDED: 'Acquisition time error exceeds the limit',
  AGE_EXCEEDED: 'Observation validity period exceeded',
  AGE_OVERFLOW: 'Cannot calculate observation time',
};
const hostText: Record<string, string> = {
  UNREGISTERED: 'Not registered yet',
  IDENTITY_UNAVAILABLE: 'Current authentication needs verification',
  CONTEXT_STALE: 'Operating context needs revalidation',
  CLOCK_MISMATCH: 'Clock basis needs verification',
  LEASE_EXPIRED: 'Lease validity period exceeded',
  CURRENT: 'Registered lease valid',
};
const reasonText: Record<string, string> = {
  SATISFIED: 'The condition was met at the time of the query.',
  NOT_SATISFIED: 'The observed value does not meet the required condition.',
  SOURCE_UNAVAILABLE: 'Some observation evidence is unusable.',
  EXPRESSION_MISMATCH: 'Check the units and value types in the condition expression.',
};
export function formatValue(value: Value): string {
  if ('boolean' in value) return value.boolean ? 'True' : 'False';
  if ('integer' in value) return value.integer;
  if ('real' in value) return String(value.real);
  if ('symbol' in value) return value.symbol;
  return `[${value.reals.values.slice(0, 6).join(', ')}${value.reals.values.length > 6 ? `, … (${value.reals.values.length} items)` : ''}]`;
}
function duration(ns: string | null): string {
  if (ns === null) return 'Cannot calculate';
  const n = BigInt(ns);
  return n < 1000000n
    ? `${Number(n) / 1000} μs`
    : n < 1000000000n
      ? `${Math.round(Number(n) / 10000) / 100} ms`
      : `${Math.round(Number(n) / 10000000) / 100} s`;
}
function ExpressionView({ expression: e }: { expression: Expression }) {
  if (e.op === 'ALL' || e.op === 'ANY')
    return (
      <div className="condition-expression">
        <b>{e.op === 'ALL' ? 'All conditions met' : 'At least one condition met'}</b>
        <ul>
          {e.children.map((child, i) => (
            <li key={i}>
              <ExpressionView expression={child} />
            </li>
          ))}
        </ul>
      </div>
    );
  return (
    <span className="condition-expression">
      <span className="source-name">{e.fact}</span>
      {e.op === 'EQ'
        ? ` = ${formatValue(e.expected)}`
        : e.op === 'RANGE'
          ? ` · ${e.min} ~ ${e.max}`
          : ` · ${e.expected} Contains`}
      {e.unit !== 'unitless' && <small> {e.unit}</small>}
    </span>
  );
}
function Verdict({ value, fresh }: { value: string; fresh: boolean }) {
  return (
    <span className={`badge ${fresh && value === 'PASS' ? 'good' : 'warning'}`}>
      {!fresh
        ? 'Refresh required'
        : value === 'PASS'
          ? 'Met'
          : value === 'FAIL'
            ? 'Not met'
            : 'Undetermined'}
    </span>
  );
}
export function Conditions({
  cell,
  requestStarted,
  queryFresh,
}: {
  cell: CellOverview;
  requestStarted: number;
  queryFresh: boolean;
}) {
  const d = cell.diagnostics;
  const [, render] = useState(0);
  const deadline = diagnosticDeadline(requestStarted, d.display_valid_for_ns);
  useEffect(() => {
    const timer = setTimeout(
      () => render((n) => n + 1),
      Math.max(0, deadline - performance.now()) + 1,
    );
    const visible = () => render((n) => n + 1);
    document.addEventListener('visibilitychange', visible);
    return () => {
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', visible);
    };
  }, [deadline]);
  const fresh = diagnosticFresh(
    requestStarted,
    d.display_valid_for_ns,
    performance.now(),
    queryFresh,
  );
  const groups: [Diagnostics['conditions'][number]['group'], string, string][] = [
    ['START', 'Pre-start checks', 'Conditions checked before approving a start request.'],
    [
      'MAINTAINED',
      'Conditions to maintain during operation',
      'Losing these conditions during operation leads to permission revocation.',
    ],
    [
      'OPERATION',
      'Per-operation entry conditions',
      'Checked again when each device operation is submitted.',
    ],
  ];
  return (
    <div className="conditions-page">
      <section className="panel conditions-intro">
        <div>
          <p className="eyebrow">CONDITIONS & EVIDENCE</p>
          <h2>Operating conditions and observation evidence</h2>
          <p className="muted">
            Meeting conditions does not grant operating permission. Check qualification, blocks, and
            the start procedure together.
          </p>
        </div>
        <div className="condition-context">
          <span className="pill">{text(cell.cell.value.commissioning ?? 'UNKNOWN')}</span>
          <span>
            {count(cell.cell.value.blocks.length, 'block')} ·{' '}
            {count(cell.cell.value.open_cases.length, 'intervention case')}
          </span>
        </div>
      </section>
      {!fresh && (
        <div className="notice warning" role="status">
          The displayed evidence has expired or the query connection was lost. These are the last
          retrieved records; refresh to obtain a current determination.
        </div>
      )}
      <section className="panel host-context-panel">
        <div className="section-heading">
          <h3>Device integration authority</h3>
          <span className="muted">Registration, authentication, and lease criteria</span>
        </div>
        <p className="muted">
          Physical device readiness and communication connectivity require separate verification.
        </p>
        <div className="host-context-list">
          {d.hosts.map((h) => (
            <div key={h.host}>
              <b>{h.host}</b>
              <span className={`badge ${fresh && h.context === 'CURRENT' ? 'good' : 'warning'}`}>
                {fresh ? hostText[h.context] : 'Refresh required'}
              </span>
            </div>
          ))}
        </div>
      </section>
      <section className="panel runtime-services">
        <div className="section-heading">
          <div>
            <p className="eyebrow">RUNTIME SERVICES</p>
            <h3>Execution service status</h3>
          </div>
        </div>
        {d.hosts.map((h) => (
          <ServiceHealthCard key={h.host} host={h.host} health={h.runtime} frameFresh={fresh} />
        ))}
      </section>
      {groups.map(([kind, label, note]) => {
        const rows = d.conditions.filter((c) => c.group === kind);
        return (
          <section className="panel condition-group" key={kind} data-condition-group={kind}>
            <div className="section-heading">
              <div>
                <h3>{label}</h3>
                <p className="muted">{note}</p>
              </div>
              <span className="count">{rows.length}</span>
            </div>
            {!rows.length ? (
              <p className="empty-inline">No conditions are registered.</p>
            ) : (
              rows.map((c) => (
                <article key={c.path} className="condition-row">
                  <div>
                    <p className="condition-label">{c.step ?? label}</p>
                    <ExpressionView expression={c.expression} />
                    <p className="muted">
                      {reasonText[c.reason]}
                      {!fresh &&
                        ` Previous determination: ${c.verdict === 'PASS' ? 'Met' : c.verdict === 'FAIL' ? 'Not met' : 'Undetermined'}`}
                    </p>
                    <details>
                      <summary>View determination evidence</summary>
                      <p>Evidence: {count(c.evidence_ids.length, 'item')}</p>
                      {c.evidence_ids.map((id) => (
                        <code key={id}>{id}</code>
                      ))}
                    </details>
                  </div>
                  <Verdict value={c.verdict} fresh={fresh} />
                </article>
              ))
            )}
          </section>
        );
      })}
      <section className="panel source-panel">
        <div className="section-heading">
          <div>
            <p className="eyebrow">SOURCE OBSERVATIONS</p>
            <h3>Observation evidence</h3>
          </div>
          <span className="count">{d.sources.length}</span>
        </div>
        {!d.sources.length ? (
          <p className="empty-inline">No observation sources are registered.</p>
        ) : (
          d.sources.map((s) => (
            <article className="source-row" key={s.source}>
              <div className="source-row-heading">
                <div>
                  <b>{s.source}</b>
                  <small>{s.host}</small>
                </div>
                <span className={`badge ${fresh && s.usable ? 'good' : 'warning'}`}>
                  {!fresh
                    ? 'Refresh required'
                    : s.usable
                      ? 'Observation usable'
                      : 'Observation unusable'}
                </span>
              </div>
              <div className="source-values">
                <div>
                  <small>Last observed value</small>
                  <strong>
                    {s.observation ? formatValue(s.observation.value) : 'No observation'}
                  </strong>
                </div>
                <div>
                  <small>Age at query time</small>
                  <strong>{s.observation ? duration(s.age_ns) : '—'}</strong>
                </div>
                <div>
                  <small>Maximum permitted age</small>
                  <strong>{duration(s.maximum_age_ns)}</strong>
                </div>
              </div>
              {s.issues.length > 0 && (
                <ul className="source-issues">
                  {s.issues.map((reason) => (
                    <li key={reason}>{issueText[reason]}</li>
                  ))}
                </ul>
              )}
              <details>
                <summary>Check source and acquisition time</summary>
                {s.observation ? (
                  <dl className="source-details">
                    <div>
                      <dt>Evidence ID</dt>
                      <dd title={s.observation.evidence_id}>{short(s.observation.evidence_id)}</dd>
                    </div>
                    <div>
                      <dt>Device generation</dt>
                      <dd>{s.observation.source_generation}</dd>
                    </div>
                    <div>
                      <dt>Registered generation</dt>
                      <dd>{s.expected_generation ?? 'Not registered'}</dd>
                    </div>
                    <div>
                      <dt>Clock</dt>
                      <dd>{s.observation.acquired_at.clock_id}</dd>
                    </div>
                    <div>
                      <dt>Acquisition time (ns)</dt>
                      <dd>{s.observation.acquired_at.ticks_ns}</dd>
                    </div>
                    <div>
                      <dt>Acquisition uncertainty</dt>
                      <dd>{duration(s.observation.acquisition_uncertainty_ns)}</dd>
                    </div>
                    <div>
                      <dt>Type / unit</dt>
                      <dd>
                        {s.observation.schema} / {s.observation.unit}
                      </dd>
                    </div>
                  </dl>
                ) : (
                  <p className="muted">No acquisition evidence to display.</p>
                )}
              </details>
            </article>
          ))
        )}
      </section>
    </div>
  );
}
