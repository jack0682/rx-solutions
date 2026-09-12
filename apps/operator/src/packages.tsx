import { useCallback, useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import { bindingCatalogSchema, type BindingCatalog } from './draft-bindings-schema';
import { editableSourceSchema } from './draft-schema';
import { previewRows } from './process-preview';
import { short } from './labels';
import {
  intakeContextSchema,
  intakePageSchema,
  reviewPageSchema,
  reviewDetailSchema,
  reviewJobSchema,
  intakeSchema,
  reviewVersionSchema,
  reviewDecisionSchema,
  digest,
  canApprove,
  reviewStamp,
  downloadJson,
  type Intake,
  type ReviewDetail,
  type PackageBuffer,
  type ReviewReceipt,
} from './package-schema';
import type { Pending } from './schema';
import { z } from 'zod';
import {
  correlateDeviceCatalog,
  DeviceCatalogPanel,
  type DeviceCatalogDetail,
} from './device-catalog';
import { intakeView } from './package-schema';
import { DeviceReviews } from './device-reviews';
import {
  deviceJobSchema,
  deviceVersionSchema,
  emptyDeviceReviewBuffer,
} from './device-review-schema';
type Context = z.infer<typeof intakeContextSchema>;
type Reviews = z.infer<typeof reviewPageSchema>;
type DecisionTarget = {
  choice: 'APPROVE' | 'REJECT';
  stamp: string;
  review: string;
  revision: string;
  digest: string;
  decisionRevision: string | null;
};
const nodeLabels: Record<string, string> = {
  SEQUENCE: '순서',
  PARALLEL_ALL: '병렬',
  OPERATION: '장비 작업',
  BRANCH: '조건 분기',
  REPEAT: '반복',
  CALL: '하위 공정',
  WAIT: '조건 대기',
  INTERVENTION: '작업자 개입',
};
export function Packages({
  cell,
  principal,
  roles,
  active,
  canWrite,
  buffer,
  onBuffer,
  onSubmit,
  receipt,
}: {
  cell: string;
  principal: string;
  roles: string[];
  active: boolean;
  canWrite: boolean;
  buffer: PackageBuffer;
  onBuffer: (v: PackageBuffer) => void;
  onSubmit: (
    route: Pending['route'],
    command: Record<string, unknown>,
    label: string,
  ) => Promise<void>;
  receipt: ReviewReceipt | null;
}) {
  const [context, setContext] = useState<Context | null>(null);
  const [catalog, setCatalog] = useState<BindingCatalog | null>(null);
  const [intakes, setIntakes] = useState<Intake[]>([]);
  const [intakeNext, setIntakeNext] = useState<string | null>(null);
  const [reviews, setReviews] = useState<Reviews | null>(null);
  const [detail, setDetail] = useState<ReviewDetail | null>(null);
  const [device, setDevice] = useState<DeviceCatalogDetail | null>(null);
  const [history, setHistory] = useState('');
  const [historyInput, setHistoryInput] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [loading, setLoading] = useState(false);
  const [readAt, setReadAt] = useState(0);
  const [now, setNow] = useState(performance.now());
  const [checked, setChecked] = useState('');
  const [dialog, setDialog] = useState<DecisionTarget | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const generation = useRef(0);
  const abort = useRef<AbortController | null>(null);
  const previousStamp = useRef('');
  const previousContext = useRef('');
  const intakeExpanded = useRef(false);
  const reviewsExpanded = useRef(false);
  const latestBuffer = useRef(buffer);
  latestBuffer.current = buffer;
  const updateBuffer = useRef(onBuffer);
  updateBuffer.current = onBuffer;
  const selected = intakes.find((v) => v.id === buffer.selectedIntake);
  const patch = (v: Partial<PackageBuffer>) => onBuffer({ ...buffer, ...v });
  const invalidate = useCallback(() => {
    setChecked('');
    setDialog(null);
  }, []);
  const load = useCallback(async () => {
    if (!active) return;
    abort.current?.abort();
    const controller = new AbortController();
    abort.current = controller;
    const seq = ++generation.current;
    const started = performance.now();
    setLoading(true);
    try {
      const [c, p, b] = await Promise.all([
        api(`/api/v1/package-intake-context?cell=${encodeURIComponent(cell)}`, {
          signal: controller.signal,
        }),
        api(`/api/v1/package-intakes?cell=${encodeURIComponent(cell)}`, {
          signal: controller.signal,
        }),
        api(`/api/v1/process-draft/binding-options?cell=${encodeURIComponent(cell)}`, {
          signal: controller.signal,
        }),
      ]);
      const ctx = intakeContextSchema.parse(c),
        page = intakePageSchema.parse(p),
        bindings = bindingCatalogSchema.parse(b);
      if (ctx.cell !== cell || page.cell !== cell || bindings.cell !== cell)
        throw Error('cell correlation');
      let list: Reviews | null = null;
      let value: ReviewDetail | null = null;
      let deviceValue: DeviceCatalogDetail | null = null;
      let selectedReceipt: Intake | null = null;
      if (buffer.selectedIntake) {
        selectedReceipt = intakeView.parse(
          await api(
            `/api/v1/package-intake?cell=${encodeURIComponent(cell)}&id=${buffer.selectedIntake}`,
            { signal: controller.signal },
          ),
        ).receipt;
        if (selectedReceipt.cell !== cell || selectedReceipt.id !== buffer.selectedIntake)
          throw Error('selected intake correlation');
      }
      if (selectedReceipt?.manifest.entry.kind === 'PROCESS') {
        list = reviewPageSchema.parse(
          await api(
            `/api/v1/process-reviews?cell=${encodeURIComponent(cell)}&intake=${buffer.selectedIntake}`,
            { signal: controller.signal },
          ),
        );
        if (list.cell !== cell || list.intake !== buffer.selectedIntake)
          throw Error('intake correlation');
      }
      if (
        selectedReceipt &&
        ['DEVICE', 'DEVICE_REFERENCE'].includes(selectedReceipt.manifest.entry.kind)
      ) {
        deviceValue = correlateDeviceCatalog(
          await api(
            `/api/v1/package-intake/device-catalog?cell=${encodeURIComponent(cell)}&id=${selectedReceipt.id}`,
            { signal: controller.signal },
          ),
          selectedReceipt,
        );
      }
      if (selectedReceipt?.manifest.entry.kind === 'PROCESS' && buffer.selectedReview) {
        value = reviewDetailSchema.parse(
          await api(
            `/api/v1/process-review?cell=${encodeURIComponent(cell)}&id=${buffer.selectedReview}${history ? `&revision=${history}` : ''}`,
            { signal: controller.signal },
          ),
        );
        if (
          value.job.request.cell !== cell ||
          value.job.request.id !== buffer.selectedReview ||
          value.job.request.intake !== buffer.selectedIntake
        )
          throw Error('review correlation');
      }
      if (seq !== generation.current) return;
      const ctxStamp = JSON.stringify([
        ctx.configuration_digest,
        ctx.registration?.generation,
        ctx.review_authority_digest,
        bindings.catalog_digest,
      ]);
      if (previousContext.current && previousContext.current !== ctxStamp) {
        invalidate();
        updateBuffer.current({ ...latestBuffer.current, selections: {} });
        setNotice('셀 또는 검토 설정이 변경되었습니다. 작업 연결을 다시 확인해 주세요.');
      }
      previousContext.current = ctxStamp;
      const stamp = value ? reviewStamp(value) : '';
      if (previousStamp.current && stamp !== previousStamp.current) {
        invalidate();
        setNotice('검토 대상이 변경되었습니다. 자료를 다시 확인해 주세요.');
      }
      previousStamp.current = stamp;
      setContext(ctx);
      setCatalog(bindings);
      setIntakes((old) => {
        const records = page.packages.map((v) => v.receipt);
        if (selectedReceipt && !records.some((v) => v.id === selectedReceipt.id))
          records.push(selectedReceipt);
        return intakeExpanded.current
          ? [...records, ...old.filter((v) => !records.some((r) => r.id === v.id))]
          : records;
      });
      setIntakeNext((old) => (intakeExpanded.current ? old : page.next));
      setReviews((old) =>
        reviewsExpanded.current && old && list && old.intake === list.intake
          ? {
              ...list,
              next: old.next,
              reviews: [
                ...list.reviews,
                ...old.reviews.filter((v) => !list.reviews.some((p) => p.id === v.id)),
              ],
            }
          : list,
      );
      setDetail(value);
      setDevice(deviceValue);
      setReadAt(started);
      setError('');
    } catch (e) {
      if (controller.signal.aborted || seq !== generation.current) return;
      setError(explain(e));
      invalidate();
    } finally {
      if (seq === generation.current) setLoading(false);
    }
  }, [active, cell, buffer.selectedIntake, buffer.selectedReview, history, invalidate]);
  useEffect(() => {
    previousStamp.current = '';
    previousContext.current = '';
    intakeExpanded.current = false;
    reviewsExpanded.current = false;
    setDetail(null);
    setDevice(null);
    setReviews(null);
    setIntakes([]);
    setContext(null);
    setReadAt(0);
    setHistory('');
    setHistoryInput('');
    invalidate();
  }, [cell, principal, invalidate]);
  useEffect(() => {
    void load();
    if (!active) return;
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible') void load();
    }, 3000);
    return () => {
      clearInterval(timer);
      ++generation.current;
      abort.current?.abort();
    };
  }, [load, active]);
  useEffect(() => {
    const t = setInterval(() => setNow(performance.now()), 500);
    return () => clearInterval(t);
  }, []);
  useEffect(() => {
    if (dialog) dialogRef.current?.showModal();
    else dialogRef.current?.close();
  }, [dialog]);
  useEffect(() => {
    if (!receipt) return;
    if (receipt.route === '/api/v1/device-reviews') {
      const v = deviceJobSchema.parse(receipt.value);
      if (v.request.cell === cell)
        updateBuffer.current({
          ...latestBuffer.current,
          selectedIntake: v.request.intake,
          device: { ...emptyDeviceReviewBuffer(), selectedReview: v.request.id },
        });
    }
    if (receipt.route === '/api/v1/device-review/reports') {
      const v = deviceVersionSchema.parse(receipt.value);
      if (v.cell === cell)
        updateBuffer.current({
          ...latestBuffer.current,
          selectedIntake: v.report.request.intake,
          device: { ...latestBuffer.current.device, selectedReview: v.review },
        });
    }
    if (receipt.route === '/api/v1/package-intakes') {
      const v = intakeSchema.parse(receipt.value);
      if (v.cell === cell) {
        updateBuffer.current({
          ...latestBuffer.current,
          selectedIntake: v.id,
          selectedReview: '',
          device: emptyDeviceReviewBuffer(),
        });
        setHistory('');
      }
    }
    if (receipt.route === '/api/v1/process-reviews') {
      const v = reviewJobSchema.parse(receipt.value);
      if (v.request.cell === cell) {
        updateBuffer.current({
          ...latestBuffer.current,
          selectedIntake: v.request.intake,
          selectedReview: v.request.id,
        });
        setHistory('');
      }
    }
    if (receipt.route === '/api/v1/process-review/reports') {
      const v = reviewVersionSchema.parse(receipt.value);
      if (v.cell === cell) {
        setHistory('');
        invalidate();
      }
    }
    if (receipt.route === '/api/v1/process-review/decisions') {
      const v = reviewDecisionSchema.parse(receipt.value);
      if (v.cell === cell) {
        invalidate();
        updateBuffer.current({ ...latestBuffer.current, note: '' });
      }
    }
    void load();
    // A receipt is consumed once; selection changes trigger their own guarded fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [receipt?.key]);
  const fresh = !!context && !error && now - readAt < 10000;
  const canAct = canWrite && fresh && !loading;
  const aliases = [
    ...new Set(
      selected?.manifest.permissions
        .filter((p) => p.kind === 'OPERATION_SUBMIT')
        .map((p) => p.operation)
        .filter((v): v is string => !!v) ?? [],
    ),
  ].sort();
  const selectionsValid = aliases.every((k) =>
    catalog?.candidates.some((c) => c.step === buffer.selections[k]),
  );
  const stamp = detail ? reviewStamp(detail) : '';
  const approved = detail ? canApprove(detail, principal, roles, canAct) : false;
  const acknowledged = !!stamp && checked === stamp;
  async function intake() {
    await onSubmit(
      '/api/v1/package-intakes',
      {
        id: crypto.randomUUID(),
        cell,
        title: buffer.title.trim(),
        relative_path: buffer.path.trim(),
        object: { manifest: buffer.manifest.trim(), signature: buffer.signature.trim() },
        configuration_digest: context?.configuration_digest,
        policy_generation: context?.registration?.generation,
      },
      `${cell} 패키지 반입`,
    );
  }
  async function create() {
    if (!selected) return;
    await onSubmit(
      '/api/v1/process-reviews',
      {
        id: crypto.randomUUID(),
        intake: selected.id,
        cell,
        configuration_digest: context?.configuration_digest,
        policy_generation: context?.registration?.generation,
        binding_selections: Object.fromEntries(aliases.map((k) => [k, buffer.selections[k]])),
      },
      `${selected.title} 검토 요청`,
    );
  }
  function target(choice: 'APPROVE' | 'REJECT') {
    if (!detail?.verification) return;
    setDialog({
      choice,
      stamp,
      review: detail.job.request.id,
      revision: detail.verification.revision,
      digest: detail.verification.review_digest,
      decisionRevision: detail.decision?.revision ?? null,
    });
  }
  async function decide() {
    if (
      !dialog ||
      !detail?.verification ||
      !canAct ||
      !acknowledged ||
      dialog.stamp !== reviewStamp(detail) ||
      !detail.is_latest ||
      !roles.includes('VERIFIER') ||
      (dialog.choice === 'APPROVE' && !approved)
    ) {
      invalidate();
      setNotice('검토 대상과 최신 상태를 다시 확인해 주세요.');
      return;
    }
    const record = {
      review: dialog.review,
      cell,
      report_revision: dialog.revision,
      review_digest: dialog.digest,
      expected: dialog.decisionRevision,
      choice: dialog.choice,
      note: buffer.note.trim(),
    };
    invalidate();
    await onSubmit(
      '/api/v1/process-review/decisions',
      record,
      `${selected?.title ?? '패키지'} 소프트웨어 ${record.choice === 'APPROVE' ? '승인' : '반려'}`,
    );
  }
  const source = editableSourceSchema.safeParse(detail?.source);
  return (
    <section className="package-workspace" hidden={!active} aria-label="패키지 검토 작업 공간">
      <div className="package-intro">
        <div>
          <p className="eyebrow">SOFTWARE REVIEW</p>
          <h2>패키지를 검토하고 기록합니다</h2>
          <p>반입한 장비 작업 선언과 공정 검증 자료를 확인하고 검토 기록을 관리합니다.</p>
        </div>
        <span className="pill">운전 활성화 별도</span>
      </div>
      <div className="package-toolbar">
        <span>{fresh ? '조회된 검토 자료' : '최신 자료 확인 필요'}</span>
        <button onClick={() => void load()} disabled={loading}>
          {loading ? '조회 중…' : '자료 새로고침'}
        </button>
      </div>
      {(error || !fresh) && (
        <div className="notice error" role="alert">
          {error || '최신 검토 자료를 확인해야 조작할 수 있습니다.'}
        </div>
      )}
      {notice && (
        <div className="notice" role="status">
          {notice}
        </div>
      )}
      <div className="package-layout">
        <aside className="panel package-library">
          <div className="section-heading">
            <h3>반입 기록</h3>
            <span>{intakes.length}</span>
          </div>
          {!intakes.length && <p className="muted">이 셀에 반입된 패키지가 없습니다.</p>}
          {intakes.map((v) => (
            <button
              className={`package-item ${v.id === buffer.selectedIntake ? 'selected' : ''}`}
              key={v.id}
              onClick={() => {
                invalidate();
                setHistory('');
                setDetail(null);
                setDevice(null);
                setReviews(null);
                reviewsExpanded.current = false;
                patch({
                  selectedIntake: v.id,
                  selectedReview: '',
                  selections: {},
                  device: emptyDeviceReviewBuffer(),
                });
              }}
            >
              <b>{v.title}</b>
              <small>
                {v.manifest.package} · {v.manifest.version}
              </small>
              <span>
                {v.submitted_by} · {short(v.id)}
              </span>
            </button>
          ))}
          {intakeNext && (
            <button
              onClick={async () => {
                try {
                  const p = intakePageSchema.parse(
                    await api(
                      `/api/v1/package-intakes?cell=${encodeURIComponent(cell)}&after=${intakeNext}`,
                    ),
                  );
                  if (p.cell !== cell) throw Error('scope');
                  intakeExpanded.current = true;
                  setIntakes((old) => [
                    ...old,
                    ...p.packages
                      .map((v) => v.receipt)
                      .filter((v) => !old.some((o) => o.id === v.id)),
                  ]);
                  setIntakeNext(p.next);
                } catch (e) {
                  setError(explain(e));
                }
              }}
            >
              반입 기록 더 보기
            </button>
          )}
          {roles.includes('ENGINEER') && (
            <details className="package-import">
              <summary>서명 패키지 반입</summary>
              <p className="muted">
                설정된 서버 반입 폴더의 파일을 가져옵니다. 식별자는 서명 도구의 결과를 사용하세요.
              </p>
              <fieldset disabled={!canWrite}>
                <label>
                  반입 제목
                  <input
                    value={buffer.title}
                    onChange={(e) => patch({ title: e.target.value })}
                    maxLength={120}
                  />
                </label>
                <label>
                  반입 폴더의 상대 경로
                  <input
                    value={buffer.path}
                    onChange={(e) => patch({ path: e.target.value })}
                    placeholder="packages/tending-v1"
                  />
                </label>
                <label>
                  패키지 내용 식별자
                  <input
                    className="mono"
                    value={buffer.manifest}
                    onChange={(e) => patch({ manifest: e.target.value })}
                    maxLength={64}
                  />
                </label>
                <label>
                  패키지 서명 식별자
                  <input
                    className="mono"
                    value={buffer.signature}
                    onChange={(e) => patch({ signature: e.target.value })}
                    maxLength={64}
                  />
                </label>
                <button
                  className="primary"
                  onClick={() => void intake()}
                  disabled={
                    !canAct ||
                    !context?.registration ||
                    !buffer.title.trim() ||
                    !buffer.path.trim() ||
                    !digest.safeParse(buffer.manifest.trim()).success ||
                    !digest.safeParse(buffer.signature.trim()).success
                  }
                >
                  패키지 반입 요청
                </button>
              </fieldset>
              {!context?.registration && <p>패키지 반입 서비스 설정이 필요합니다.</p>}
            </details>
          )}
        </aside>
        <div className="package-content">
          {!selected ? (
            <div className="panel empty">
              <h3>반입 기록을 선택하세요</h3>
              <p>패키지의 검토 요청과 자료를 확인할 수 있습니다.</p>
            </div>
          ) : (
            <>
              <section className="panel">
                <div className="section-heading">
                  <div>
                    <p className="eyebrow">SELECTED PACKAGE</p>
                    <h3>{selected.title}</h3>
                  </div>
                  <span className="pill">{selected.manifest.version}</span>
                </div>
                <dl className="facts">
                  <div>
                    <dt>제출 계정</dt>
                    <dd>{selected.submitted_by}</dd>
                  </div>
                  <div>
                    <dt>패키지</dt>
                    <dd>{selected.manifest.package}</dd>
                  </div>
                </dl>
                <details>
                  <summary>패키지 식별·선언 보기</summary>
                  <pre>{JSON.stringify(selected, null, 2)}</pre>
                </details>
                {selected.manifest.entry.kind === 'PROCESS' && (
                  <details className="package-create">
                    <summary>새 검토 요청 만들기</summary>
                    {selected.manifest.entry.kind !== 'PROCESS' ? (
                      <p>현재 검토 도구는 공정 패키지를 지원합니다.</p>
                    ) : (
                      <>
                        <p>공정의 작업 이름을 이 셀에 등록된 장비 작업과 연결하세요.</p>
                        <fieldset disabled={!canWrite}>
                          {aliases.map((k) => (
                            <label key={k}>
                              {k} 작업 연결
                              <select
                                aria-label={`${k} 작업 연결`}
                                value={buffer.selections[k] ?? ''}
                                onChange={(e) =>
                                  patch({
                                    selections: { ...buffer.selections, [k]: e.target.value },
                                  })
                                }
                              >
                                <option value="">장비 작업 선택</option>
                                {catalog?.candidates.map((c) => (
                                  <option value={c.step} key={c.step}>
                                    {c.step} · {c.target}
                                  </option>
                                ))}
                              </select>
                            </label>
                          ))}
                          <button
                            className="primary"
                            onClick={() => void create()}
                            disabled={
                              !canAct || !context?.review_authority_digest || !selectionsValid
                            }
                          >
                            검토 요청 생성
                          </button>
                        </fieldset>
                        {!context?.review_authority_digest && <p>검증 서명자 설정이 필요합니다.</p>}
                      </>
                    )}
                  </details>
                )}
              </section>
              {['DEVICE', 'DEVICE_REFERENCE'].includes(selected.manifest.entry.kind) && (
                <>
                  <DeviceCatalogPanel detail={device} intake={selected} fresh={fresh && !error} />
                  <DeviceReviews
                    key={`${principal}/${selected.id}`}
                    intake={selected}
                    declaration={device}
                    context={context}
                    contextFresh={fresh}
                    principal={principal}
                    roles={roles}
                    active={active}
                    canWrite={canWrite}
                    buffer={buffer.device}
                    onBuffer={(v) => onBuffer({ ...buffer, device: v })}
                    onSubmit={onSubmit}
                    receipt={receipt}
                  />
                </>
              )}
              {selected.manifest.entry.kind === 'PROCESS' && (
                <section className="panel">
                  <div className="section-heading">
                    <h3>검토 요청</h3>
                    <span>{reviews?.reviews.length ?? 0}</span>
                  </div>
                  <div className="review-list">
                    {reviews?.reviews.map((r) => (
                      <button
                        className={r.id === buffer.selectedReview ? 'selected' : ''}
                        key={r.id}
                        onClick={() => {
                          invalidate();
                          setHistory('');
                          setHistoryInput('');
                          setDetail(null);
                          patch({ selectedReview: r.id });
                        }}
                      >
                        <b>검토 {short(r.id)}</b>
                        <span>
                          {r.report_revision ? `검증 자료 r${r.report_revision}` : '검증 자료 대기'}{' '}
                          · {r.requested_by}
                        </span>
                      </button>
                    ))}
                  </div>
                  {!reviews?.reviews.length && (
                    <p className="muted">새 검토 요청을 만들어 검증 도구에 전달하세요.</p>
                  )}
                  {reviews?.next && (
                    <button
                      onClick={async () => {
                        try {
                          const p = reviewPageSchema.parse(
                            await api(
                              `/api/v1/process-reviews?cell=${encodeURIComponent(cell)}&intake=${selected.id}&after=${reviews.next}`,
                            ),
                          );
                          if (p.cell !== cell || p.intake !== selected.id) throw Error('scope');
                          reviewsExpanded.current = true;
                          setReviews((old) =>
                            old
                              ? {
                                  ...p,
                                  reviews: [
                                    ...old.reviews,
                                    ...p.reviews.filter(
                                      (v) => !old.reviews.some((o) => o.id === v.id),
                                    ),
                                  ],
                                }
                              : p,
                          );
                        } catch (e) {
                          setError(explain(e));
                        }
                      }}
                    >
                      검토 요청 더 보기
                    </button>
                  )}
                </section>
              )}
              {detail && (
                <>
                  <section className="panel review-material">
                    <div className="section-heading">
                      <div>
                        <p className="eyebrow">REVIEW MATERIAL</p>
                        <h3>
                          {detail.verification
                            ? `검증 자료 r${detail.verification.revision}`
                            : '검증 자료 대기'}
                        </h3>
                      </div>
                      <span className="pill">
                        {detail.is_latest ? '최신 검토' : '과거 검토 · 읽기 전용'}
                      </span>
                    </div>
                    <div className="package-actions">
                      <button
                        onClick={() =>
                          downloadJson(
                            detail.job.request,
                            `rx-review-${detail.job.request.id}.json`,
                          )
                        }
                      >
                        검증 요청 내보내기
                      </button>
                      <button
                        onClick={() =>
                          downloadJson(detail, `rx-review-material-${detail.job.request.id}.json`)
                        }
                      >
                        검토 자료 내려받기
                      </button>
                    </div>
                    <p className="muted">
                      검증 도구가 만든 자료와 별도 서명을 서버 반입 폴더에 준비한 뒤 등록하세요.
                    </p>
                    <details className="package-report">
                      <summary>서명된 검증 자료 등록</summary>
                      <fieldset disabled={!canWrite || !detail.is_latest}>
                        <label>
                          검증 자료 상대 경로
                          <input
                            value={buffer.reportPath}
                            onChange={(e) => patch({ reportPath: e.target.value })}
                            placeholder="reviews/tending-v1"
                          />
                        </label>
                        <label>
                          검증 보고서 식별자
                          <input
                            className="mono"
                            value={buffer.reportDigest}
                            onChange={(e) => patch({ reportDigest: e.target.value })}
                            maxLength={64}
                          />
                        </label>
                        <button
                          onClick={() =>
                            void onSubmit(
                              '/api/v1/process-review/reports',
                              {
                                review: detail.job.request.id,
                                cell,
                                expected: detail.latest_report_revision,
                                directory: buffer.reportPath.trim(),
                                report_digest: buffer.reportDigest.trim(),
                              },
                              `${selected.title} 검증 자료 등록`,
                            )
                          }
                          disabled={
                            !canAct ||
                            !detail.context_current ||
                            !buffer.reportPath.trim() ||
                            !digest.safeParse(buffer.reportDigest.trim()).success
                          }
                        >
                          검증 자료 등록 요청
                        </button>
                      </fieldset>
                    </details>
                    {detail.latest_report_revision && (
                      <div className="review-history">
                        <label>
                          확인할 검증 버전
                          <input
                            inputMode="numeric"
                            maxLength={20}
                            value={historyInput}
                            onChange={(e) => setHistoryInput(e.target.value)}
                            placeholder={detail.latest_report_revision}
                          />
                        </label>
                        <button
                          disabled={
                            !/^[1-9][0-9]*$/.test(historyInput) ||
                            BigInt(historyInput || '0') > BigInt(detail.latest_report_revision)
                          }
                          onClick={() => {
                            invalidate();
                            setHistory(historyInput);
                          }}
                        >
                          과거 버전 보기
                        </button>
                        <button
                          onClick={() => {
                            invalidate();
                            setHistory('');
                            setHistoryInput('');
                          }}
                        >
                          최신 검토 보기
                        </button>
                      </div>
                    )}
                    {!detail.context_current && (
                      <div className="notice error">
                        검토 당시의 구성 또는 검증 정책과 현재 문맥이 다릅니다. 새 검토가
                        필요합니다.
                      </div>
                    )}
                    {detail.verification && (
                      <>
                        <div
                          className={`review-verdict ${detail.verification.ready_for_software_approval ? 'ready' : 'blocked'}`}
                        >
                          <b>
                            {detail.verification.ready_for_software_approval
                              ? '소프트웨어 검토 자료 준비됨'
                              : '검증 결과 확인 필요'}
                          </b>
                          <span>
                            서명자 {detail.verification.signature.key} · 검토 식별{' '}
                            {short(detail.verification.review_digest)}
                          </span>
                        </div>
                        {[
                          ...detail.verification.report.issues,
                          ...detail.verification.platform_issues,
                        ].map((v, i) => (
                          <div className="notice error" key={`${v.code}-${i}`}>
                            <b>{v.code}</b>
                            <p>{v.detail}</p>
                            <small>{v.location}</small>
                          </div>
                        ))}
                        {source.success && (
                          <div className="review-flow">
                            <h4>공정 원문 · {source.data.process}</h4>
                            {source.data.flows.map((f, i) => (
                              <div key={f.id}>
                                <b>{f.id}</b>
                                {previewRows(source.data, i).map((r, j) => (
                                  <div
                                    className="review-flow-row"
                                    style={{ paddingLeft: Math.min(r.depth, 8) * 16 }}
                                    key={`${f.id}-${j}`}
                                  >
                                    <span>{String(j + 1).padStart(2, '0')}</span>
                                    <strong>{r.nodeId || '표시 한도'}</strong>
                                    <small>
                                      {r.problem ??
                                        nodeLabels[String(f.nodes[r.nodeIndex]?.body.kind)] ??
                                        '정의 확인'}
                                    </small>
                                  </div>
                                ))}
                              </div>
                            ))}
                          </div>
                        )}
                        <details>
                          <summary>원문과 조건 전체 보기</summary>
                          <pre>{JSON.stringify(detail.source, null, 2)}</pre>
                        </details>
                        <details>
                          <summary>컴파일 결과 전체 보기</summary>
                          <pre>{JSON.stringify(detail.resolved, null, 2)}</pre>
                        </details>
                        {detail.job.device_context && (
                          <p>
                            장비 변경 후보를 검토 중입니다. 현재 셀 설정은 바뀌지 않았으며, 적용 전
                            장비 연결과 운전 조건 검증이 필요합니다.
                          </p>
                        )}
                        <details>
                          <summary>셀 작업 연결과 검증 근거 전체 보기</summary>
                          <pre>
                            {JSON.stringify(
                              {
                                bindings: detail.job.request.binding_selections,
                                configuration: detail.job.configuration,
                                device_candidates: detail.job.device_context,
                                verification: detail.verification,
                              },
                              null,
                              2,
                            )}
                          </pre>
                        </details>
                      </>
                    )}
                  </section>
                  {detail.verification && (
                    <section className="panel review-decision">
                      <p className="eyebrow">REVIEW DECISION</p>
                      <h3>이 검토 버전에 결정을 남깁니다</h3>
                      <p>
                        소프트웨어 검토 결정입니다. 실제 장비 운전과 활성화는 별도로 확인합니다.
                      </p>
                      {detail.decision && (
                        <div className="inset">
                          <b>
                            {detail.approval_matches_current_review
                              ? '이 버전에 소프트웨어 승인 기록이 있습니다'
                              : '이전 검토 결정 기록'}
                          </b>
                          <p>
                            {detail.decision.choice === 'APPROVE' ? '승인' : '반려'} · 검증 자료 r
                            {detail.decision.report_revision} · {detail.decision.decided_by}
                          </p>
                          <p>{detail.decision.note}</p>
                        </div>
                      )}
                      {!roles.includes('VERIFIER') ? (
                        <p className="muted">
                          검토자 권한이 있는 계정으로 승인 또는 반려할 수 있습니다.
                        </p>
                      ) : (
                        <>
                          <fieldset disabled={!canWrite || !detail.is_latest}>
                            <label>
                              검토 의견
                              <textarea
                                value={buffer.note}
                                onChange={(e) => patch({ note: e.target.value })}
                                maxLength={1000}
                              />
                            </label>
                            <label className="review-check">
                              <input
                                type="checkbox"
                                checked={acknowledged}
                                onChange={(e) => setChecked(e.target.checked ? stamp : '')}
                              />
                              원문·컴파일 결과·셀 연결과 이 검토 버전을 확인했습니다.
                            </label>
                          </fieldset>
                          {principal === detail.job.submitted_by && (
                            <p className="muted">
                              제출 계정은 자기 패키지를 승인할 수 없습니다. 다른 검토자 계정이
                              필요합니다.
                            </p>
                          )}
                          <div className="package-actions">
                            <button
                              className="primary"
                              disabled={!approved || !acknowledged || !buffer.note.trim()}
                              onClick={() => target('APPROVE')}
                            >
                              소프트웨어 검토 승인
                            </button>
                            <button
                              disabled={
                                !canAct || !detail.is_latest || !acknowledged || !buffer.note.trim()
                              }
                              onClick={() => target('REJECT')}
                            >
                              검토 반려
                            </button>
                          </div>
                        </>
                      )}
                    </section>
                  )}
                </>
              )}
            </>
          )}
        </div>
      </div>
      <dialog ref={dialogRef} onCancel={invalidate} className="review-confirm">
        <p className="eyebrow">CONFIRM REVIEW TARGET</p>
        <h2>{dialog?.choice === 'APPROVE' ? '소프트웨어 검토 승인' : '검토 반려'}</h2>
        <p>
          {selected?.title} · 검증 자료 r{dialog?.revision}
        </p>
        <p className="mono">{dialog?.digest}</p>
        <p>현재 표시된 검토 자료에 이 결정을 기록합니다. 장비 동작은 시작하지 않습니다.</p>
        <div className="package-actions">
          <button onClick={invalidate}>돌아가기</button>
          <button
            className="primary"
            onClick={() => void decide()}
            disabled={!canAct || !acknowledged}
          >
            {dialog?.choice === 'APPROVE' ? '이 버전 승인 기록' : '이 버전 반려 기록'}
          </button>
        </div>
      </dialog>
    </section>
  );
}
