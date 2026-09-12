import { useCallback, useEffect, useRef, useState } from 'react';
import { z } from 'zod';
import { api, explain } from './api';
import {
  downloadJson,
  intakeContextSchema,
  type Intake,
  type ReviewReceipt,
} from './package-schema';
import { type DeviceCatalogDetail } from './device-catalog';
import {
  canApproveDevice,
  deviceDecisionSchema,
  deviceDetailSchema,
  deviceJobSchema,
  devicePageSchema,
  deviceReviewStamp,
  deviceVersionSchema,
  type DeviceReviewBuffer,
  type DeviceReviewDetail,
  type DeviceReviewPage,
} from './device-review-schema';
import { short } from './labels';
import type { Pending } from './schema';
import { stableDocument } from './draft-schema';
type Context = z.infer<typeof intakeContextSchema>;
type Target = {
  stamp: string;
  command: Record<string, unknown>;
  choice: 'APPROVE' | 'REJECT';
  revision: string;
  digest: string;
};
const checks = {
  CONTENT_SIGNATURE: '서명과 패키지 내용',
  DEVICE_SOURCE_CONSISTENCY: '장비 정의와 원본 일관성',
  CATALOG_REQUEST_BINDING: '검토 요청과 작업 선언 연결',
};
const states = { PASSED: '통과', FAILED: '실패', NOT_PERFORMED: '미수행' };
export function DeviceReviews({
  intake,
  declaration,
  context,
  contextFresh,
  principal,
  roles,
  active,
  canWrite,
  buffer,
  onBuffer,
  onSubmit,
  receipt,
}: {
  intake: Intake;
  declaration: DeviceCatalogDetail | null;
  context: Context | null;
  contextFresh: boolean;
  principal: string;
  roles: string[];
  active: boolean;
  canWrite: boolean;
  buffer: DeviceReviewBuffer;
  onBuffer: (v: DeviceReviewBuffer) => void;
  onSubmit: (
    route: Pending['route'],
    command: Record<string, unknown>,
    label: string,
  ) => Promise<void>;
  receipt: ReviewReceipt | null;
}) {
  const [page, setPage] = useState<DeviceReviewPage | null>(null),
    [detail, setDetail] = useState<DeviceReviewDetail | null>(null);
  const [history, setHistory] = useState(''),
    [historyInput, setHistoryInput] = useState(''),
    [error, setError] = useState(''),
    [notice, setNotice] = useState('');
  const [loading, setLoading] = useState(false),
    [readAt, setReadAt] = useState(0),
    [now, setNow] = useState(performance.now());
  const [checked, setChecked] = useState(''),
    [target, setTarget] = useState<Target | null>(null);
  const dialog = useRef<HTMLDialogElement>(null),
    seq = useRef(0),
    abort = useRef<AbortController | null>(null),
    previous = useRef(''),
    lastReceipt = useRef('');
  const expanded = useRef(false);
  const latest = useRef(buffer),
    update = useRef(onBuffer);
  latest.current = buffer;
  update.current = onBuffer;
  const patch = (v: Partial<DeviceReviewBuffer>) => onBuffer({ ...buffer, ...v });
  const invalidate = useCallback(() => {
    setChecked('');
    setTarget(null);
  }, []);
  const ctxStamp = JSON.stringify([
    context?.configuration_digest,
    context?.registration,
    context?.device_review_authority_digest,
    principal,
    roles,
  ]);
  const load = useCallback(async () => {
    if (!active) return;
    abort.current?.abort();
    const controller = new AbortController();
    abort.current = controller;
    const generation = ++seq.current;
    const started = performance.now();
    setLoading(true);
    try {
      const list = devicePageSchema.parse(
        await api(
          `/api/v1/device-reviews?cell=${encodeURIComponent(intake.cell)}&intake=${intake.id}`,
          { signal: controller.signal },
        ),
      );
      if (list.cell !== intake.cell || list.intake !== intake.id) throw Error('device list scope');
      let value: DeviceReviewDetail | null = null;
      if (buffer.selectedReview) {
        value = deviceDetailSchema.parse(
          await api(
            `/api/v1/device-review?cell=${encodeURIComponent(intake.cell)}&id=${buffer.selectedReview}${history ? `&revision=${history}` : ''}`,
            { signal: controller.signal },
          ),
        );
        const r = value.job.request;
        if (
          r.id !== buffer.selectedReview ||
          r.intake !== intake.id ||
          r.cell !== intake.cell ||
          r.package_manifest !== intake.object.manifest ||
          r.package_signature !== intake.object.signature ||
          r.catalog.sha256 !== intake.device_catalog?.sha256 ||
          r.catalog.size_bytes !== intake.device_catalog?.size_bytes ||
          r.catalog.schema_id !== intake.device_catalog?.schema_id ||
          (declaration?.catalog && r.installation !== declaration.catalog.installation)
        )
          throw Error('device review target differs');
      }
      if (generation !== seq.current) return;
      const stamp = value ? deviceReviewStamp(value, ctxStamp) : ctxStamp;
      if (previous.current && previous.current !== stamp) {
        invalidate();
        setNotice('장비 검토 자료가 변경되었습니다. 이 버전을 다시 확인해 주세요.');
      }
      previous.current = stamp;
      setPage((old) =>
        expanded.current && old
          ? {
              ...list,
              next: old.next,
              reviews: [
                ...list.reviews,
                ...old.reviews.filter((v) => !list.reviews.some((r) => r.id === v.id)),
              ],
            }
          : list,
      );
      setDetail(value);
      setReadAt(started);
      setError('');
    } catch (e) {
      if (!controller.signal.aborted && generation === seq.current) {
        setError(explain(e));
        invalidate();
      }
    } finally {
      if (generation === seq.current) setLoading(false);
    }
  }, [
    active,
    intake.id,
    intake.cell,
    intake.object.manifest,
    intake.object.signature,
    intake.device_catalog?.sha256,
    intake.device_catalog?.schema_id,
    intake.device_catalog?.size_bytes,
    declaration?.catalog?.installation,
    buffer.selectedReview,
    history,
    ctxStamp,
    invalidate,
  ]);
  useEffect(() => {
    void load();
    if (!active) return;
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible') void load();
    }, 3000);
    return () => {
      clearInterval(timer);
      seq.current++;
      abort.current?.abort();
    };
  }, [load, active]);
  useEffect(() => {
    const t = setInterval(() => setNow(performance.now()), 500);
    return () => clearInterval(t);
  }, []);
  const fresh = !!page && !error && now - readAt < 10000 && contextFresh;
  const canAct = fresh && canWrite && active && !loading;
  const matches =
    !!detail &&
    !!context &&
    detail.job.request.configuration_digest === context.configuration_digest &&
    detail.job.request.verification_authority_digest === context.device_review_authority_digest &&
    stableDocument(detail.job.registration) === stableDocument(context.registration);
  const stamp = detail ? deviceReviewStamp(detail, ctxStamp) : '';
  const approve = !!detail && matches && canApproveDevice(detail, principal, roles, canAct);
  const reject = canAct && roles.includes('VERIFIER') && !!detail?.version && !!detail?.is_latest;
  useEffect(() => {
    if (!fresh || !active || !canWrite) invalidate();
  }, [fresh, active, canWrite, invalidate]);
  useEffect(() => {
    if (target) dialog.current?.showModal();
    else dialog.current?.close();
  }, [target]);
  useEffect(() => {
    if (!receipt || lastReceipt.current === receipt.key) return;
    lastReceipt.current = receipt.key;
    if (receipt.route === '/api/v1/device-reviews') {
      const v = deviceJobSchema.parse(receipt.value);
      if (v.request.intake === intake.id) {
        update.current({
          ...latest.current,
          selectedReview: v.request.id,
          reportPath: '',
          reportDigest: '',
          note: '',
        });
        setHistory('');
        invalidate();
      }
    } else if (receipt.route === '/api/v1/device-review/reports') {
      const v = deviceVersionSchema.parse(receipt.value);
      if (v.report.request.intake === intake.id) {
        update.current({ ...latest.current, selectedReview: v.review });
        setHistory('');
        invalidate();
        void load();
      }
    } else if (receipt.route === '/api/v1/device-review/decisions') {
      const v = deviceDecisionSchema.parse(receipt.value);
      if (v.review === latest.current.selectedReview && v.cell === intake.cell) {
        update.current({ ...latest.current, note: '' });
        invalidate();
        void load();
      }
    }
  }, [receipt, intake.id, intake.cell, invalidate, load]);
  async function create() {
    if (!context || !canAct) return;
    await onSubmit(
      '/api/v1/device-reviews',
      {
        id: crypto.randomUUID(),
        intake: intake.id,
        cell: intake.cell,
        configuration_digest: context.configuration_digest,
        policy_generation: context.registration?.generation,
      },
      `${intake.title} 장비 검토 요청`,
    );
  }
  function decide(choice: 'APPROVE' | 'REJECT') {
    if (
      !detail?.version ||
      checked !== stamp ||
      !buffer.note.trim() ||
      !(choice === 'APPROVE' ? approve : reject)
    )
      return;
    const v = detail.version;
    setTarget({
      choice,
      stamp,
      revision: v.revision,
      digest: v.review_digest,
      command: {
        review: v.review,
        cell: v.cell,
        report_revision: v.revision,
        review_digest: v.review_digest,
        expected: detail.decision?.revision ?? null,
        choice,
        note: buffer.note.trim(),
      },
    });
  }
  async function confirm() {
    if (
      !target ||
      target.stamp !== stamp ||
      checked !== stamp ||
      !(target.choice === 'APPROVE' ? approve : reject)
    ) {
      invalidate();
      return;
    }
    const exact = target;
    setTarget(null);
    await onSubmit(
      '/api/v1/device-review/decisions',
      exact.command,
      `${intake.title} 장비 소프트웨어 ${exact.choice === 'APPROVE' ? '승인' : '반려'}`,
    );
  }
  return (
    <section className="device-review-workspace" aria-label="장비 소프트웨어 검토">
      <div className="panel">
        <div className="section-heading">
          <h3>장비 소프트웨어 검토</h3>
          <span className="pill">실물 검증·운전 자격 별도</span>
        </div>
        <p>
          패키지의 서명·원본 일관성·작업 선언 연결을 검토합니다. 실제 로봇 동작이나 교정 적합성을
          승인하는 절차는 아닙니다.
        </p>
        <div className="package-actions">
          <button
            onClick={() => void create()}
            disabled={
              !canAct ||
              !context?.device_review_authority_digest ||
              !context?.registration ||
              !intake.device_catalog ||
              !declaration?.catalog ||
              intake.configuration_digest !== context.configuration_digest
            }
          >
            장비 검토 요청 만들기
          </button>
          <button onClick={() => void load()} disabled={loading}>
            장비 검토 새로고침
          </button>
        </div>
        {!context?.device_review_authority_digest && (
          <p className="muted">장비 검증 서명자 설정이 필요합니다.</p>
        )}
        {(error || !fresh) && (
          <p role="alert" className="notice error">
            {error || '최신 장비 검토 자료를 확인해야 조작할 수 있습니다.'}
          </p>
        )}
        {notice && (
          <p role="status" className="notice">
            {notice}
          </p>
        )}
        <div className="review-list device-review-list">
          {page?.reviews.map((r) => (
            <button
              key={r.id}
              className={r.id === buffer.selectedReview ? 'selected' : ''}
              onClick={() => {
                invalidate();
                setHistory('');
                setHistoryInput('');
                setDetail(null);
                patch({ selectedReview: r.id, reportPath: '', reportDigest: '', note: '' });
              }}
            >
              <b>장비 검토 {short(r.id)}</b>
              <span>
                {r.report_revision ? `보고서 r${r.report_revision}` : '보고서 대기'} ·{' '}
                {r.requested_by}
                {r.approval_matches_current_review ? ' · 소프트웨어 승인 기록' : ''}
              </span>
            </button>
          ))}
        </div>
        {!page?.reviews.length && <p className="muted">아직 장비 검토 요청이 없습니다.</p>}
        {page?.next && (
          <button
            onClick={async () => {
              try {
                const p = devicePageSchema.parse(
                  await api(
                    `/api/v1/device-reviews?cell=${encodeURIComponent(intake.cell)}&intake=${intake.id}&after=${page.next}`,
                  ),
                );
                if (p.cell !== intake.cell || p.intake !== intake.id)
                  throw Error('device page scope');
                expanded.current = true;
                setPage((old) =>
                  old
                    ? {
                        ...p,
                        reviews: [
                          ...old.reviews,
                          ...p.reviews.filter((v) => !old.reviews.some((o) => o.id === v.id)),
                        ],
                      }
                    : p,
                );
              } catch (e) {
                setError(explain(e));
              }
            }}
          >
            장비 검토 더 보기
          </button>
        )}
      </div>
      {detail && (
        <div className="panel device-review-material">
          <div className="section-heading">
            <h3>
              {detail.version
                ? `장비 검증 보고서 r${detail.version.revision}`
                : '장비 검증 보고서 대기'}
            </h3>
            <span className="pill">
              {detail.is_latest ? '최신 검토 버전' : '과거 장비 검토 · 읽기 전용'}
            </span>
          </div>
          <div className="package-actions">
            <button
              onClick={() =>
                downloadJson(detail.job.request, `rx-device-review-${detail.job.request.id}.json`)
              }
            >
              장비 검증 요청 내려받기
            </button>
            <button
              onClick={() =>
                downloadJson(detail, `rx-device-review-material-${detail.job.request.id}.json`)
              }
            >
              장비 검토 자료 내려받기
            </button>
          </div>
          <p className="muted">
            검증 도구가 만든 보고서와 별도 서명을 서버 반입 폴더에 준비한 뒤 등록하세요.
          </p>
          <details>
            <summary>장비 검증 보고서 등록</summary>
            <fieldset
              disabled={!canAct || !detail.is_latest || !detail.context_current || !matches}
            >
              <label>
                장비 보고서 상대 경로
                <input
                  value={buffer.reportPath}
                  onChange={(e) => patch({ reportPath: e.target.value })}
                />
              </label>
              <label>
                장비 보고서 식별자
                <input
                  className="mono"
                  value={buffer.reportDigest}
                  maxLength={64}
                  onChange={(e) => patch({ reportDigest: e.target.value })}
                />
              </label>
              <button
                disabled={
                  !buffer.reportPath.trim() || !/^[0-9a-f]{64}$/.test(buffer.reportDigest.trim())
                }
                onClick={() =>
                  void onSubmit(
                    '/api/v1/device-review/reports',
                    {
                      review: detail.job.request.id,
                      cell: intake.cell,
                      expected: detail.latest_report_revision,
                      directory: buffer.reportPath.trim(),
                      report_digest: buffer.reportDigest.trim(),
                    },
                    `${intake.title} 장비 보고서 등록`,
                  )
                }
              >
                장비 보고서 등록 요청
              </button>
            </fieldset>
          </details>
          {detail.version && (
            <>
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>검사 항목</th>
                      <th>결과</th>
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(detail.version.report.checks).map(([name, state]) => (
                      <tr key={name}>
                        <td>{checks[name as keyof typeof checks]}</td>
                        <td>{states[state]}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {detail.version.report.issues.map((issue, i) => (
                <p key={i} className="notice error">
                  {issue.location}: {issue.detail}
                </p>
              ))}
              <p>
                {detail.version.ready_for_software_approval
                  ? '장비 패키지 소프트웨어 검사 통과'
                  : '소프트웨어 승인 조건을 충족하지 못했습니다.'}
              </p>
              {detail.approval_matches_current_review && (
                <p className="notice" role="status">
                  이 장비 검토 버전에 소프트웨어 승인 기록이 있습니다.
                </p>
              )}
              {(!detail.context_current || !matches) && (
                <p className="notice error">
                  셀 구성 또는 검증 정책이 변경되어 이 자료로 승인할 수 없습니다.
                </p>
              )}
              <details>
                <summary>장비 검토 원문·서명·결정 보기</summary>
                <pre>{JSON.stringify(detail, null, 2)}</pre>
              </details>
              <div className="package-history">
                <label>
                  확인할 장비 보고서 버전
                  <input
                    value={historyInput}
                    onChange={(e) => setHistoryInput(e.target.value)}
                    inputMode="numeric"
                  />
                </label>
                <button
                  disabled={
                    !/^[1-9][0-9]{0,19}$/.test(historyInput) ||
                    BigInt(historyInput) > 18446744073709551615n
                  }
                  onClick={() => {
                    invalidate();
                    setHistory(historyInput);
                  }}
                >
                  과거 장비 버전 보기
                </button>
                {history && (
                  <button
                    onClick={() => {
                      invalidate();
                      setHistory('');
                      setHistoryInput('');
                    }}
                  >
                    최신 장비 검토 보기
                  </button>
                )}
              </div>
              <fieldset disabled={!canAct || !detail.is_latest || !roles.includes('VERIFIER')}>
                <label>
                  장비 검토 의견
                  <textarea
                    value={buffer.note}
                    maxLength={1000}
                    onChange={(e) => {
                      invalidate();
                      patch({ note: e.target.value });
                    }}
                  />
                </label>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={!!stamp && checked === stamp}
                    onChange={(e) => setChecked(e.target.checked ? stamp : '')}
                  />
                  장비 원본·서명·검사 범위와 이 보고서 버전을 확인했습니다.
                </label>
                <div className="package-actions">
                  <button
                    className="primary"
                    disabled={!approve || checked !== stamp || !buffer.note.trim()}
                    onClick={() => decide('APPROVE')}
                  >
                    장비 소프트웨어 승인
                  </button>
                  <button
                    disabled={!reject || checked !== stamp || !buffer.note.trim()}
                    onClick={() => decide('REJECT')}
                  >
                    장비 소프트웨어 반려
                  </button>
                </div>
              </fieldset>
              {principal === detail.job.submitted_by && (
                <p className="muted">패키지 제출자와 다른 검토자가 승인해야 합니다.</p>
              )}
            </>
          )}
        </div>
      )}
      <dialog
        ref={dialog}
        className="confirm-dialog"
        onCancel={(e) => {
          e.preventDefault();
          setTarget(null);
        }}
      >
        {target && (
          <>
            <p className="eyebrow">DEVICE PACKAGE SOFTWARE</p>
            <h3>이 장비 검토 버전을 {target.choice === 'APPROVE' ? '승인' : '반려'}합니다</h3>
            <p>
              {intake.title} · 보고서 r{target.revision}
            </p>
            <p className="mono">{target.digest}</p>
            <p>소프트웨어 패키지 검토 결정입니다. 실물 검증이나 운전 자격을 생성하지 않습니다.</p>
            <p>{String(target.command.note)}</p>
            <div className="package-actions">
              <button onClick={() => setTarget(null)}>돌아가기</button>
              <button
                className="primary"
                disabled={
                  target.stamp !== stamp || !(target.choice === 'APPROVE' ? approve : reject)
                }
                onClick={() => void confirm()}
              >
                {target.choice === 'APPROVE' ? '이 장비 버전 승인 기록' : '이 장비 버전 반려 기록'}
              </button>
            </div>
          </>
        )}
      </dialog>
    </section>
  );
}
