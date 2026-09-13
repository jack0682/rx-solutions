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
import { z } from './schema-runtime';
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
  SEQUENCE: 'Sequence',
  PARALLEL_ALL: 'Parallel',
  OPERATION: 'Device operation',
  BRANCH: 'Conditional branch',
  REPEAT: 'Repeat',
  CALL: 'Subworkflow',
  WAIT: 'Wait for condition',
  INTERVENTION: 'Operator intervention',
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
        setNotice(
          'The cell or review configuration has changed. Check the operation bindings again.',
        );
      }
      previousContext.current = ctxStamp;
      const stamp = value ? reviewStamp(value) : '';
      if (previousStamp.current && stamp !== previousStamp.current) {
        invalidate();
        setNotice('The review target has changed. Review the evidence again.');
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
      `${cell} Package intake`,
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
      `${selected.title} Review request`,
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
      setNotice('Check the review target and latest state again.');
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
      `${selected?.title ?? 'Package'} Software ${record.choice === 'APPROVE' ? 'Approve' : 'Reject'}`,
    );
  }
  const source = editableSourceSchema.safeParse(detail?.source);
  return (
    <section className="package-workspace" hidden={!active} aria-label="Package review workspace">
      <div className="package-intro">
        <div>
          <p className="eyebrow">SOFTWARE REVIEW</p>
          <h2>Review packages and record decisions</h2>
          <p>
            Inspect imported device operation declarations and workflow verification evidence, and
            manage review records.
          </p>
        </div>
        <span className="pill">Operating activation is separate</span>
      </div>
      <div className="package-toolbar">
        <span>{fresh ? 'Retrieved review evidence' : 'Latest evidence needs verification'}</span>
        <button onClick={() => void load()} disabled={loading}>
          {loading ? 'Loading…' : 'Refresh evidence'}
        </button>
      </div>
      {(error || !fresh) && (
        <div className="notice error" role="alert">
          {error || 'Check the latest review evidence before making changes.'}
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
            <h3>Intake records</h3>
            <span>{intakes.length}</span>
          </div>
          {!intakes.length && (
            <p className="muted">No packages have been imported into this cell.</p>
          )}
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
              Load more intake records
            </button>
          )}
          {roles.includes('ENGINEER') && (
            <details className="package-import">
              <summary>Import signed package</summary>
              <p className="muted">
                Import files from the configured server intake directory. Use identifiers produced
                by the signing tool.
              </p>
              <fieldset disabled={!canWrite}>
                <label>
                  Intake title
                  <input
                    value={buffer.title}
                    onChange={(e) => patch({ title: e.target.value })}
                    maxLength={120}
                  />
                </label>
                <label>
                  Relative path in intake directory
                  <input
                    value={buffer.path}
                    onChange={(e) => patch({ path: e.target.value })}
                    placeholder="packages/tending-v1"
                  />
                </label>
                <label>
                  Package content identifier
                  <input
                    className="mono"
                    value={buffer.manifest}
                    onChange={(e) => patch({ manifest: e.target.value })}
                    maxLength={64}
                  />
                </label>
                <label>
                  Package signature identifier
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
                  Request package intake
                </button>
              </fieldset>
              {!context?.registration && <p>Configure the package intake service first.</p>}
            </details>
          )}
        </aside>
        <div className="package-content">
          {!selected ? (
            <div className="panel empty">
              <h3>Select an intake record</h3>
              <p>View review requests and evidence for the package.</p>
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
                    <dt>Submitted by</dt>
                    <dd>{selected.submitted_by}</dd>
                  </div>
                  <div>
                    <dt>Package</dt>
                    <dd>{selected.manifest.package}</dd>
                  </div>
                </dl>
                <details>
                  <summary>View package identity and declarations</summary>
                  <pre>{JSON.stringify(selected, null, 2)}</pre>
                </details>
                {selected.manifest.entry.kind === 'PROCESS' && (
                  <details className="package-create">
                    <summary>Create new review request</summary>
                    {selected.manifest.entry.kind !== 'PROCESS' ? (
                      <p>The current review tool supports workflow packages.</p>
                    ) : (
                      <>
                        <p>
                          Bind workflow operation names to device operations registered in this
                          cell.
                        </p>
                        <fieldset disabled={!canWrite}>
                          {aliases.map((k) => (
                            <label key={k}>
                              {k} Operation bindings
                              <select
                                aria-label={`${k} Operation bindings`}
                                value={buffer.selections[k] ?? ''}
                                onChange={(e) =>
                                  patch({
                                    selections: { ...buffer.selections, [k]: e.target.value },
                                  })
                                }
                              >
                                <option value="">Select device operation</option>
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
                            Create review request
                          </button>
                        </fieldset>
                        {!context?.review_authority_digest && (
                          <p>Configure a verification signer first.</p>
                        )}
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
                    <h3>Review request</h3>
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
                        <b>Review {short(r.id)}</b>
                        <span>
                          {r.report_revision
                            ? `Verification evidence r${r.report_revision}`
                            : 'Awaiting verification evidence'}{' '}
                          · {r.requested_by}
                        </span>
                      </button>
                    ))}
                  </div>
                  {!reviews?.reviews.length && (
                    <p className="muted">
                      Create a new review request and pass it to the verification tool.
                    </p>
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
                      Load more review requests
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
                            ? `Verification evidence r${detail.verification.revision}`
                            : 'Awaiting verification evidence'}
                        </h3>
                      </div>
                      <span className="pill">
                        {detail.is_latest ? 'Latest review' : 'Historical review · read only'}
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
                        Export verification request
                      </button>
                      <button
                        onClick={() =>
                          downloadJson(detail, `rx-review-material-${detail.job.request.id}.json`)
                        }
                      >
                        Download review evidence
                      </button>
                    </div>
                    <p className="muted">
                      Place the evidence from the verification tool and its detached signature in
                      the server intake directory, then register them.
                    </p>
                    <details className="package-report">
                      <summary>Register signed verification evidence</summary>
                      <fieldset disabled={!canWrite || !detail.is_latest}>
                        <label>
                          Verification evidence relative path
                          <input
                            value={buffer.reportPath}
                            onChange={(e) => patch({ reportPath: e.target.value })}
                            placeholder="reviews/tending-v1"
                          />
                        </label>
                        <label>
                          Verification report identifier
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
                              `${selected.title} Register verification evidence`,
                            )
                          }
                          disabled={
                            !canAct ||
                            !detail.context_current ||
                            !buffer.reportPath.trim() ||
                            !digest.safeParse(buffer.reportDigest.trim()).success
                          }
                        >
                          Request verification evidence registration
                        </button>
                      </fieldset>
                    </details>
                    {detail.latest_report_revision && (
                      <div className="review-history">
                        <label>
                          Verification version to view
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
                          View historical version
                        </button>
                        <button
                          onClick={() => {
                            invalidate();
                            setHistory('');
                            setHistoryInput('');
                          }}
                        >
                          View latest review
                        </button>
                      </div>
                    )}
                    {!detail.context_current && (
                      <div className="notice error">
                        The current context differs from the configuration or verification policy
                        used for the review. A new review is required.
                      </div>
                    )}
                    {detail.verification && (
                      <>
                        <div
                          className={`review-verdict ${detail.verification.ready_for_software_approval ? 'ready' : 'blocked'}`}
                        >
                          <b>
                            {detail.verification.ready_for_software_approval
                              ? 'Software review evidence ready'
                              : 'Verification results need review'}
                          </b>
                          <span>
                            Signer {detail.verification.signature.key} · review identifier{' '}
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
                            <h4>Workflow source · {source.data.process}</h4>
                            {source.data.flows.map((f, i) => (
                              <div key={f.id}>
                                <b>{f.id}</b>
                                {previewRows(source.data, i).map((r, j) => (
                                  <div
                                    className={`review-flow-row review-depth-${Math.max(0, Math.min(r.depth, 8))}`}
                                    key={`${f.id}-${j}`}
                                  >
                                    <span>{String(j + 1).padStart(2, '0')}</span>
                                    <strong>{r.nodeId || 'Display limit'}</strong>
                                    <small>
                                      {r.problem ??
                                        nodeLabels[String(f.nodes[r.nodeIndex]?.body.kind)] ??
                                        'Check definition'}
                                    </small>
                                  </div>
                                ))}
                              </div>
                            ))}
                          </div>
                        )}
                        <details>
                          <summary>View full source and conditions</summary>
                          <pre>{JSON.stringify(detail.source, null, 2)}</pre>
                        </details>
                        <details>
                          <summary>View full compiled result</summary>
                          <pre>{JSON.stringify(detail.resolved, null, 2)}</pre>
                        </details>
                        {detail.job.device_context && (
                          <p>
                            Reviewing a device change candidate. The current cell configuration is
                            unchanged. Device bindings and operating conditions must be verified
                            before application.
                          </p>
                        )}
                        <details>
                          <summary>
                            View all cell operation bindings and verification evidence
                          </summary>
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
                      <h3>Record a decision for this review version</h3>
                      <p>
                        This is a software review decision. Physical device operation and activation
                        require separate checks.
                      </p>
                      {detail.decision && (
                        <div className="inset">
                          <b>
                            {detail.approval_matches_current_review
                              ? 'Software approval is recorded for this version'
                              : 'Previous review decision record'}
                          </b>
                          <p>
                            {detail.decision.choice === 'APPROVE' ? 'Approve' : 'Reject'} ·
                            verification evidence r{detail.decision.report_revision} ·{' '}
                            {detail.decision.decided_by}
                          </p>
                          <p>{detail.decision.note}</p>
                        </div>
                      )}
                      {!roles.includes('VERIFIER') ? (
                        <p className="muted">
                          An account with verifier authority can approve or reject this review.
                        </p>
                      ) : (
                        <>
                          <fieldset disabled={!canWrite || !detail.is_latest}>
                            <label>
                              Review note
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
                              I have reviewed the source, compiled result, cell bindings, and this
                              review version.
                            </label>
                          </fieldset>
                          {principal === detail.job.submitted_by && (
                            <p className="muted">
                              The submitter cannot approve their own package. Another verifier
                              account is required.
                            </p>
                          )}
                          <div className="package-actions">
                            <button
                              className="primary"
                              disabled={!approved || !acknowledged || !buffer.note.trim()}
                              onClick={() => target('APPROVE')}
                            >
                              Approve software review
                            </button>
                            <button
                              disabled={
                                !canAct || !detail.is_latest || !acknowledged || !buffer.note.trim()
                              }
                              onClick={() => target('REJECT')}
                            >
                              Reject review
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
        <h2>{dialog?.choice === 'APPROVE' ? 'Approve software review' : 'Reject review'}</h2>
        <p>
          {selected?.title} · verification evidence r{dialog?.revision}
        </p>
        <p className="mono">{dialog?.digest}</p>
        <p>
          Record this decision for the displayed review evidence. This does not start device motion.
        </p>
        <div className="package-actions">
          <button onClick={invalidate}>Back</button>
          <button
            className="primary"
            onClick={() => void decide()}
            disabled={!canAct || !acknowledged}
          >
            {dialog?.choice === 'APPROVE'
              ? 'Record approval for this version'
              : 'Record rejection for this version'}
          </button>
        </div>
      </dialog>
    </section>
  );
}
