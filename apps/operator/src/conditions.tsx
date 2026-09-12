import { ServiceHealthCard } from './service-health';
import { useEffect, useState } from 'react';
import type { CellOverview } from './schema';
import type { Diagnostics, Expression, Value } from './diagnostics-schema';
import { diagnosticDeadline, diagnosticFresh } from './diagnostic-time';
import { text, short } from './labels';
const issueText: Record<string, string> = {
  MISSING_OBSERVATION: '아직 관측하지 못함',
  HOST_UNREGISTERED: 'Host 등록 필요',
  GENERATION_UNREGISTERED: '관측 출처의 세대 미등록',
  GENERATION_MISMATCH: '장비 세대가 달라짐',
  WRONG_SOURCE_HOST: '지정된 장비의 관측이 아님',
  SCHEMA_MISMATCH: '관측 형식 불일치',
  UNIT_MISMATCH: '단위 불일치',
  BAD_QUALITY: '관측 품질 확인 필요',
  ORIGIN_AGE_UNBOUNDED: '취득 시점을 보장할 수 없음',
  DISPUTED: '서로 모순되는 근거',
  CLOCK_MISMATCH: '시계 기준 불일치',
  FUTURE_TIMESTAMP: '취득 시각 확인 필요',
  UNCERTAINTY_EXCEEDED: '취득 시각 오차 초과',
  AGE_EXCEEDED: '관측 유효시간 초과',
  AGE_OVERFLOW: '관측 시간 계산 불가',
};
const hostText: Record<string, string> = {
  UNREGISTERED: '아직 등록되지 않음',
  IDENTITY_UNAVAILABLE: '현재 인증 확인 필요',
  CONTEXT_STALE: '운전 문맥 재확인 필요',
  CLOCK_MISMATCH: '시계 기준 확인 필요',
  LEASE_EXPIRED: '사용권 유효시간 초과',
  CURRENT: '등록된 사용권 유효',
};
const reasonText: Record<string, string> = {
  SATISFIED: '조회 시점에 조건을 충족했습니다.',
  NOT_SATISFIED: '관측값이 요구 조건과 다릅니다.',
  SOURCE_UNAVAILABLE: '사용할 수 없는 관측 근거가 있습니다.',
  EXPRESSION_MISMATCH: '조건식의 단위·값 형식을 확인해야 합니다.',
};
export function formatValue(value: Value): string {
  if ('boolean' in value) return value.boolean ? '참' : '거짓';
  if ('integer' in value) return value.integer;
  if ('real' in value) return String(value.real);
  if ('symbol' in value) return value.symbol;
  return `[${value.reals.values.slice(0, 6).join(', ')}${value.reals.values.length > 6 ? `, … (${value.reals.values.length}개)` : ''}]`;
}
function duration(ns: string | null): string {
  if (ns === null) return '계산 불가';
  const n = BigInt(ns);
  return n < 1000000n
    ? `${Number(n) / 1000} μs`
    : n < 1000000000n
      ? `${Math.round(Number(n) / 10000) / 100} ms`
      : `${Math.round(Number(n) / 10000000) / 100} 초`;
}
function ExpressionView({ expression: e }: { expression: Expression }) {
  if (e.op === 'ALL' || e.op === 'ANY')
    return (
      <div className="condition-expression">
        <b>{e.op === 'ALL' ? '모두 충족' : '하나 이상 충족'}</b>
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
          : ` · ${e.expected} 포함`}
      {e.unit !== 'unitless' && <small> {e.unit}</small>}
    </span>
  );
}
function Verdict({ value, fresh }: { value: string; fresh: boolean }) {
  return (
    <span className={`badge ${fresh && value === 'PASS' ? 'good' : 'warning'}`}>
      {!fresh ? '재조회 필요' : value === 'PASS' ? '충족' : value === 'FAIL' ? '미충족' : '미확정'}
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
    ['START', '시작 전 확인', '시작 요청을 승인하기 전에 확인하는 조건입니다.'],
    ['MAINTAINED', '운전 중 유지', '운전 중 상실되면 허가 철회로 이어지는 조건입니다.'],
    ['OPERATION', '작업별 진입 조건', '각 장비 작업을 제출할 때 다시 확인합니다.'],
  ];
  return (
    <div className="conditions-page">
      <section className="panel conditions-intro">
        <div>
          <p className="eyebrow">CONDITIONS & EVIDENCE</p>
          <h2>운전 조건과 관측 근거</h2>
          <p className="muted">
            조건 충족은 운전 허가가 아닙니다. 자격·차단·시작 절차를 함께 확인합니다.
          </p>
        </div>
        <div className="condition-context">
          <span className="pill">{text(cell.cell.value.commissioning ?? 'UNKNOWN')}</span>
          <span>
            {cell.cell.value.blocks.length}개 차단 · {cell.cell.value.open_cases.length}개 개입 사건
          </span>
        </div>
      </section>
      {!fresh && (
        <div className="notice warning" role="status">
          표시 근거의 유효시간이 지났거나 조회가 끊겼습니다. 아래는 마지막 조회의 기록이며 현재
          판정은 재조회가 필요합니다.
        </div>
      )}
      <section className="panel host-context-panel">
        <div className="section-heading">
          <h3>장비 연동 권한</h3>
          <span className="muted">등록·인증·사용권 기준</span>
        </div>
        <p className="muted">장비의 실제 동작 준비와 통신 연결 상태는 별도 확인 대상입니다.</p>
        <div className="host-context-list">
          {d.hosts.map((h) => (
            <div key={h.host}>
              <b>{h.host}</b>
              <span className={`badge ${fresh && h.context === 'CURRENT' ? 'good' : 'warning'}`}>
                {fresh ? hostText[h.context] : '재조회 필요'}
              </span>
            </div>
          ))}
        </div>
      </section>
      <section className="panel runtime-services">
        <div className="section-heading">
          <div>
            <p className="eyebrow">RUNTIME SERVICES</p>
            <h3>실행 서비스 상태</h3>
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
              <p className="empty-inline">등록된 조건이 없습니다.</p>
            ) : (
              rows.map((c) => (
                <article key={c.path} className="condition-row">
                  <div>
                    <p className="condition-label">{c.step ?? label}</p>
                    <ExpressionView expression={c.expression} />
                    <p className="muted">
                      {reasonText[c.reason]}
                      {!fresh &&
                        ` 이전 판정: ${c.verdict === 'PASS' ? '충족' : c.verdict === 'FAIL' ? '미충족' : '미확정'}`}
                    </p>
                    <details>
                      <summary>판정 근거 보기</summary>
                      <p>근거 {c.evidence_ids.length}개</p>
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
            <h3>관측 근거</h3>
          </div>
          <span className="count">{d.sources.length}</span>
        </div>
        {!d.sources.length ? (
          <p className="empty-inline">등록된 관측 출처가 없습니다.</p>
        ) : (
          d.sources.map((s) => (
            <article className="source-row" key={s.source}>
              <div className="source-row-heading">
                <div>
                  <b>{s.source}</b>
                  <small>{s.host}</small>
                </div>
                <span className={`badge ${fresh && s.usable ? 'good' : 'warning'}`}>
                  {!fresh ? '재조회 필요' : s.usable ? '관측 사용 가능' : '관측 사용 불가'}
                </span>
              </div>
              <div className="source-values">
                <div>
                  <small>마지막 관측값</small>
                  <strong>{s.observation ? formatValue(s.observation.value) : '관측 없음'}</strong>
                </div>
                <div>
                  <small>조회 시점의 경과 시간</small>
                  <strong>{s.observation ? duration(s.age_ns) : '—'}</strong>
                </div>
                <div>
                  <small>허용 경과 시간</small>
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
                <summary>출처·취득 시각 확인</summary>
                {s.observation ? (
                  <dl className="source-details">
                    <div>
                      <dt>근거 ID</dt>
                      <dd title={s.observation.evidence_id}>{short(s.observation.evidence_id)}</dd>
                    </div>
                    <div>
                      <dt>장비 세대</dt>
                      <dd>{s.observation.source_generation}</dd>
                    </div>
                    <div>
                      <dt>등록된 세대</dt>
                      <dd>{s.expected_generation ?? '미등록'}</dd>
                    </div>
                    <div>
                      <dt>시계</dt>
                      <dd>{s.observation.acquired_at.clock_id}</dd>
                    </div>
                    <div>
                      <dt>취득 시각(ns)</dt>
                      <dd>{s.observation.acquired_at.ticks_ns}</dd>
                    </div>
                    <div>
                      <dt>취득 오차</dt>
                      <dd>{duration(s.observation.acquisition_uncertainty_ns)}</dd>
                    </div>
                    <div>
                      <dt>형식 / 단위</dt>
                      <dd>
                        {s.observation.schema} / {s.observation.unit}
                      </dd>
                    </div>
                  </dl>
                ) : (
                  <p className="muted">표시할 취득 근거가 없습니다.</p>
                )}
              </details>
            </article>
          ))
        )}
      </section>
    </div>
  );
}
