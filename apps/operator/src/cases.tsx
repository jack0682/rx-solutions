import { useEffect, useState } from 'react';
import { api, explain } from './api';
import { caseListSchema, type CaseSnapshot } from './schema';
import { text, short } from './labels';

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
    <section className="panel case-panel" aria-label="개입 사건">
      <div className="section-heading">
        <div>
          <p className="eyebrow">INTERVENTION RECORDS</p>
          <h3>개입 사건</h3>
        </div>
        <span className="count">
          {items.length}
          {truncated ? '+' : ''}
        </span>
      </div>
      <div className="inset">
        <b>알림 확인과 작업 허가는 별도입니다</b>
        <p>
          확인 기록은 알림을 읽었다는 뜻입니다. 접근·격리·작업 종료를 확인하거나 재시작을 허가하지
          않습니다.
        </p>
      </div>
      {error && (
        <p role="alert" className="error">
          {error} 확인 기록을 새로 남길 수 없습니다.
        </p>
      )}
      {!items.length && (
        <p className="case-empty">
          {loading ? '사건 기록을 읽고 있습니다.' : '등록된 개입 사건이 없습니다.'}
        </p>
      )}
      <div className="case-list">
        {items.map((item) => (
          <article className="case-item" key={item.case.id}>
            <div>
              <p className="eyebrow">
                {text(item.case.kind)} · 사건 {short(item.case.id)}
              </p>
              <h3>{text(item.case.state)}</h3>
              <p className="muted">
                조정 책임자 {item.case.lead} · 기록 r{item.revision}
              </p>
            </div>
            <dl>
              <div>
                <dt>생산 제한</dt>
                <dd>{item.case.block_ids.length}개 연결</dd>
              </div>
              <div>
                <dt>알림 확인 기록</dt>
                <dd>{item.acknowledgment_count}건</dd>
              </div>
              <div>
                <dt>참여 기록</dt>
                <dd>
                  {item.case.participants.length
                    ? `${item.case.participants.length}개 계정`
                    : '아직 등록되지 않음'}
                </dd>
              </div>
            </dl>
            {item.case.scope_uncertain && (
              <p className="muted">영향 범위가 미확정되어 연결된 셀 범위로 보류했습니다.</p>
            )}
            <div className="case-actions">
              <small>이 동작으로 사건 상태나 생산 제한이 해제되지 않습니다.</small>
              <button
                disabled={!canAcknowledge || loading || !!error}
                onClick={() => onAcknowledge(item)}
              >
                알림 확인 기록
              </button>
            </div>
          </article>
        ))}
      </div>
      <p className="muted case-footnote">
        이 화면에서는 접근·재시작을 허가하지 않습니다. 참여 기록이 없다는 표시를 ‘현장에 사람이
        없음’으로 해석하지 않습니다.
      </p>
    </section>
  );
}
