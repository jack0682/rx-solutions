import { useEffect, useState } from 'react';
import { api, explain } from './api';
import { caseListSchema, type CaseSnapshot } from './schema';
import { count, text, short } from './labels';

export function Cases({
  cell,
  refreshToken,
  canAcknowledge,
  onAcknowledge,
}: {
  cell: string;
  refreshToken: string;
  canAcknowledge: boolean;
  onAcknowledge: (item: CaseSnapshot) => void;
}) {
  const [items, setItems] = useState<CaseSnapshot[]>([]);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [truncated, setTruncated] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    void api(`/api/v1/cases?cell=${encodeURIComponent(cell)}`, { signal: controller.signal })
      .then((value) => {
        if (controller.signal.aborted) return;
        const data = caseListSchema.parse(value);
        if (data.cell !== cell) throw new Error('case cell mismatch');
        setItems(data.cases);
        setTruncated(data.truncated);
        setError('');
      })
      .catch((value) => {
        if (!controller.signal.aborted) setError(explain(value));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [cell, refreshToken]);
  return (
    <section className="panel case-panel" aria-label="Intervention cases">
      <div className="section-heading">
        <div>
          <p className="eyebrow">INTERVENTION RECORDS</p>
          <h3>Intervention cases</h3>
        </div>
        <span className="count">
          {items.length}
          {truncated ? '+' : ''}
        </span>
      </div>
      <div className="inset">
        <b>Acknowledgment and task permission are separate</b>
        <p>
          An acknowledgment records that the notification was read. It does not confirm access,
          containment, or task completion, or authorize a restart.
        </p>
      </div>
      {error && (
        <p role="alert" className="error">
          {error} Cannot record a new acknowledgment.
        </p>
      )}
      {!items.length && (
        <p className="case-empty">
          {loading ? 'Loading case records.' : 'No intervention cases are registered.'}
        </p>
      )}
      <div className="case-list">
        {items.map((item) => (
          <article className="case-item" key={item.case.id}>
            <div>
              <p className="eyebrow">
                {text(item.case.kind)} · case {short(item.case.id)}
              </p>
              <h3>{text(item.case.state)}</h3>
              <p className="muted">
                Coordination lead {item.case.lead} · record r{item.revision}
              </p>
            </div>
            <dl>
              <div>
                <dt>Production blocks</dt>
                <dd>{item.case.block_ids.length} linked</dd>
              </div>
              <div>
                <dt>Notification acknowledgment</dt>
                <dd>{count(item.acknowledgment_count, 'record')}</dd>
              </div>
              <div>
                <dt>Participation records</dt>
                <dd>
                  {item.case.participants.length
                    ? count(item.case.participants.length, 'account')
                    : 'Not registered yet'}
                </dd>
              </div>
            </dl>
            {item.case.scope_uncertain && (
              <p className="muted">
                The affected scope is unresolved, so the hold covers the connected cell scope.
              </p>
            )}
            <div className="case-actions">
              <small>This action does not clear the case state or production blocks.</small>
              <button
                disabled={!canAcknowledge || loading || !!error}
                onClick={() => onAcknowledge(item)}
              >
                Notification acknowledgment
              </button>
            </div>
          </article>
        ))}
      </div>
      <p className="muted case-footnote">
        This screen does not authorize access or restart. A lack of participation records does not
        mean that no people are on site.
      </p>
    </section>
  );
}
