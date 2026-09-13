import { useCallback, useEffect, useRef, useState } from 'react';
import { z } from './schema-runtime';
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
  CONTENT_SIGNATURE: 'Signature and package content',
  DEVICE_SOURCE_CONSISTENCY: 'Device definition and source consistency',
  CATALOG_REQUEST_BINDING: 'Review request and operation declaration binding',
};
const states = { PASSED: 'Passed', FAILED: 'Failure', NOT_PERFORMED: 'Not performed' };
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
        setNotice('The device review evidence has changed. Review this version again.');
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
      `${intake.title} Device review request`,
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
      `${intake.title} Device software ${exact.choice === 'APPROVE' ? 'Approve' : 'Reject'}`,
    );
  }
  return (
    <section className="device-review-workspace" aria-label="Device software review">
      <div className="panel">
        <div className="section-heading">
          <h3>Device software review</h3>
          <span className="pill">
            Physical verification and operating qualification are separate
          </span>
        </div>
        <p>
          Review the package signature, source consistency, and operation declaration bindings. This
          procedure does not approve physical robot operation or calibration suitability.
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
            Create device review request
          </button>
          <button onClick={() => void load()} disabled={loading}>
            Refresh device reviews
          </button>
        </div>
        {!context?.device_review_authority_digest && (
          <p className="muted">Configure a device verification signer first.</p>
        )}
        {(error || !fresh) && (
          <p role="alert" className="notice error">
            {error || 'Check the latest device review evidence before making changes.'}
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
              <b>Device review {short(r.id)}</b>
              <span>
                {r.report_revision ? `Report r${r.report_revision}` : 'Awaiting report'} ·{' '}
                {r.requested_by}
                {r.approval_matches_current_review ? ' · software approval recorded' : ''}
              </span>
            </button>
          ))}
        </div>
        {!page?.reviews.length && <p className="muted">No device review requests yet.</p>}
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
            Load more device reviews
          </button>
        )}
      </div>
      {detail && (
        <div className="panel device-review-material">
          <div className="section-heading">
            <h3>
              {detail.version
                ? `Device verification report r${detail.version.revision}`
                : 'Awaiting device verification report'}
            </h3>
            <span className="pill">
              {detail.is_latest ? 'Latest review version' : 'Historical device review · read only'}
            </span>
          </div>
          <div className="package-actions">
            <button
              onClick={() =>
                downloadJson(detail.job.request, `rx-device-review-${detail.job.request.id}.json`)
              }
            >
              Download device verification request
            </button>
            <button
              onClick={() =>
                downloadJson(detail, `rx-device-review-material-${detail.job.request.id}.json`)
              }
            >
              Download device review evidence
            </button>
          </div>
          <p className="muted">
            Place the report from the verification tool and its detached signature in the server
            intake directory, then register them.
          </p>
          <details>
            <summary>Register device verification report</summary>
            <fieldset
              disabled={!canAct || !detail.is_latest || !detail.context_current || !matches}
            >
              <label>
                Device report relative path
                <input
                  value={buffer.reportPath}
                  onChange={(e) => patch({ reportPath: e.target.value })}
                />
              </label>
              <label>
                Device report identifier
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
                    `${intake.title} Register device report`,
                  )
                }
              >
                Request device report registration
              </button>
            </fieldset>
          </details>
          {detail.version && (
            <>
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>Check</th>
                      <th>Outcome</th>
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
                  ? 'Device package software checks passed'
                  : 'Software approval requirements are not met.'}
              </p>
              {detail.approval_matches_current_review && (
                <p className="notice" role="status">
                  Software approval is recorded for this device review version.
                </p>
              )}
              {(!detail.context_current || !matches) && (
                <p className="notice error">
                  The cell configuration or verification policy has changed. This evidence cannot be
                  approved.
                </p>
              )}
              <details>
                <summary>View device review source, signatures, and decisions</summary>
                <pre>{JSON.stringify(detail, null, 2)}</pre>
              </details>
              <div className="package-history">
                <label>
                  Device report version to view
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
                  View historical device version
                </button>
                {history && (
                  <button
                    onClick={() => {
                      invalidate();
                      setHistory('');
                      setHistoryInput('');
                    }}
                  >
                    View latest device review
                  </button>
                )}
              </div>
              <fieldset disabled={!canAct || !detail.is_latest || !roles.includes('VERIFIER')}>
                <label>
                  Device review note
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
                  I have reviewed the device source, signatures, check scope, and this report
                  version.
                </label>
                <div className="package-actions">
                  <button
                    className="primary"
                    disabled={!approve || checked !== stamp || !buffer.note.trim()}
                    onClick={() => decide('APPROVE')}
                  >
                    Approve device software
                  </button>
                  <button
                    disabled={!reject || checked !== stamp || !buffer.note.trim()}
                    onClick={() => decide('REJECT')}
                  >
                    Reject device software
                  </button>
                </div>
              </fieldset>
              {principal === detail.job.submitted_by && (
                <p className="muted">
                  Approval requires a verifier other than the package submitter.
                </p>
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
            <h3>
              Record a decision for this device review version:{' '}
              {target.choice === 'APPROVE' ? 'Approve' : 'Reject'}
            </h3>
            <p>
              {intake.title} · report r{target.revision}
            </p>
            <p className="mono">{target.digest}</p>
            <p>
              This is a software package review decision. It does not establish physical
              verification or operating qualification.
            </p>
            <p>{String(target.command.note)}</p>
            <div className="package-actions">
              <button onClick={() => setTarget(null)}>Back</button>
              <button
                className="primary"
                disabled={
                  target.stamp !== stamp || !(target.choice === 'APPROVE' ? approve : reject)
                }
                onClick={() => void confirm()}
              >
                {target.choice === 'APPROVE'
                  ? 'Record approval for this device version'
                  : 'Record rejection for this device version'}
              </button>
            </div>
          </>
        )}
      </dialog>
    </section>
  );
}
