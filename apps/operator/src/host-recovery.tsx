import { useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import { count, short, text } from './labels';
import { stableDocument } from './draft-schema';
import type { CellOverview, Overview, Pending } from './schema';
import {
  canRecoverHostRequest,
  freezeRecoveryApproval,
  freezeRecoveryProposal,
  recoveryContextCurrent,
  validateRecoveryContext,
  validateRecoveryPage,
  validateRecoveryProgress,
  validateRecoveryQuery,
  validateRecoveryView,
  type RecoveryContext,
  type RecoveryContextResponse,
  type RecoveryQuery,
  type RecoveryTransientRoute,
  type RecoveryView,
} from './host-recovery-schema';

const phases = {
  PROPOSED: 'Proposal awaiting approval',
  FENCING: 'Verifying fencing of prior operating authority',
  RECOVERY_ONLY: 'Recovery inspection binding',
  ATTENTION: 'Responsible operator review required',
};
const blockers: Record<string, string> = {
  BASELINE_MISSING: 'Original binding evidence unavailable',
  REGISTRATION_MISSING: 'Original Host registration unavailable',
  IDENTITY_CHANGED: 'Binding target identity changed',
  RESTART_ORIGIN_MISSING: 'Restart block evidence unavailable',
  LIVE_AUTHORITY: 'Prior execution or operating authority remains',
  TOO_MANY_OPERATIONS: 'Number of operations to inspect exceeds the limit',
  FENCE_CANDIDATES_AMBIGUOUS: 'Cannot identify a unique original fence request',
};

function Scope({ context }: { context: RecoveryContext }) {
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Affected cell</th>
            <th>Record revision</th>
            <th>Current blocks</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(context.cells).map(([name, cut]) => (
            <tr key={name}>
              <td>
                {name}
                {context.host_cells.includes(name)
                  ? ' · Host-connected cell'
                  : ' · shared affected cell'}
              </td>
              <td>r{cut.revision}</td>
              <td>
                {cut.blocks.length
                  ? cut.blocks.map((block) => text(block.reason)).join(', ')
                  : 'No record'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
export function RecoveryEvidence({ binding }: { binding: RecoveryView['view']['binding'] }) {
  const readEvidence = binding.last_read ?? binding.proposal_read;
  return (
    <>
      <h4>Last Host observation</h4>
      {readEvidence.configuration.receipt === null ? (
        <p>This query did not include an application receipt.</p>
      ) : !readEvidence.configuration.context_matches_current_host ? (
        <p className="notice">
          The included application receipt has not been confirmed to match the current Host state.
        </p>
      ) : null}
      {binding.phase === 'ATTENTION' && (
        <p className="notice">
          This observation was stored in a state requiring review. It is not evidence for the
          current binding.
        </p>
      )}
      {Object.entries(readEvidence.cells).map(([name, read]) => (
        <div key={name}>
          <b>
            {name} ·{' '}
            {read.snapshot.sources_available
              ? 'Stored observation available'
              : 'Observation source unavailable'}
          </b>
          <p className="muted">
            {count(read.snapshot.observations.length, 'observation')} · this does not establish that
            current operating conditions are met.
          </p>
          {read.snapshot.observations.map((o) => (
            <p key={o.source}>
              {o.source} · {stableDocument(o.value)} ·
              {o.quality_good && o.origin_age_bounded && !o.disputed
                ? ' Review stored quality fields'
                : ' Quality, time, and conflicts need verification'}
            </p>
          ))}
        </div>
      ))}
    </>
  );
}
export function HostRecovery({
  data,
  cell,
  fresh,
  canWrite,
  working,
  pending,
  receipt,
  onSubmit,
  onAction,
  onSelectCell,
}: {
  data: Overview;
  cell: CellOverview;
  fresh: boolean;
  canWrite: boolean;
  working: boolean;
  pending: Pending | null;
  receipt: RecoveryView | null;
  onSubmit: (record: Pending) => Promise<void>;
  onAction: (
    route: RecoveryTransientRoute,
    command: { id: string; operation?: string },
  ) => Promise<unknown>;
  onSelectCell: (cell: string) => void;
}) {
  const pendingReview = pending?.recovery_review;
  const original =
    pending &&
    pendingReview &&
    pendingReview.origin === cell.cell.value.id &&
    canRecoverHostRequest(pending, data)
      ? pendingReview.host
      : '';
  const [hostChoice, setHostChoice] = useState('');
  const host = hostChoice || original;
  const [context, setContext] = useState<RecoveryContextResponse | null>(null);
  const [contextReadAt, setContextReadAt] = useState(0);
  const [detailReadAt, setDetailReadAt] = useState(0);
  const [items, setItems] = useState<RecoveryView[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [selected, setSelected] = useState('');
  const [detail, setDetail] = useState<RecoveryView | null>(null);
  const [query, setQuery] = useState<RecoveryQuery | null>(null);
  const [errors, setErrors] = useState({ context: '', list: '', detail: '', action: '' });
  const [reload, setReload] = useState(0);
  const [review, setReview] = useState<{
    request: Pending;
    fingerprint: string;
    scope: RecoveryContext;
  } | null>(null);
  const [reviewMessage, setReviewMessage] = useState('');
  const [paging, setPaging] = useState(false);
  const hostRef = useRef(host);
  hostRef.current = host;
  const dialog = useRef<HTMLDialogElement>(null);
  const currentContext =
    context?.context.host === host && context.context.origin === cell.cell.value.id
      ? context
      : null;
  const currentDetail =
    detail?.view.binding.id === selected && detail.view.binding.context.host === host
      ? detail
      : null;
  const binding = currentDetail?.view.binding;
  const role = data.user.roles.includes('RELEASE_MANAGER') && !!data.user.terminal;
  const readable = fresh && role && cell.cell.value.hosts.includes(host);
  const contextCurrent = !!currentContext && recoveryContextCurrent(currentContext.context, data);
  const detailCurrent = !!binding && recoveryContextCurrent(binding.context, data);
  const readNow = performance.now();
  const contextFresh =
    contextCurrent &&
    !errors.context &&
    readNow >= contextReadAt &&
    readNow - contextReadAt < 10000;
  const detailFresh =
    detailCurrent && !errors.detail && readNow >= detailReadAt && readNow - detailReadAt < 10000;
  const sameOrigin = binding?.context.origin === cell.cell.value.id;
  const basisMatches =
    !!binding &&
    !!currentContext &&
    sameOrigin &&
    binding.requested_context_digest === currentContext.context_digest;
  const canPropose =
    canWrite && readable && contextFresh && currentContext?.context.blockers.length === 0;
  const canApprove =
    canWrite &&
    readable &&
    detailFresh &&
    contextFresh &&
    basisMatches &&
    binding?.context.blockers.length === 0;
  const canAdvance =
    canWrite &&
    readable &&
    detailFresh &&
    contextFresh &&
    basisMatches &&
    !!binding?.approved_by &&
    binding.context.blockers.length === 0;
  const confirmationToken = (kind: Pending['route']) =>
    stableDocument([
      data.user.principal,
      data.user.terminal,
      data.installation,
      currentContext?.context_digest,
      currentContext?.expected_cells,
      kind === '/api/v1/host-recovery/approve'
        ? [
            binding?.id,
            binding?.revision,
            currentDetail?.proposal_digest,
            Object.values(binding?.fences ?? {}).map((f) => [f.task.request, f.payload_digest]),
          ]
        : null,
      Object.entries(currentContext?.context.cells ?? {}).map(([name]) => [
        name,
        data.cells.find((c) => c.cell.value.id === name)?.cell.revision,
      ]),
    ]);

  useEffect(() => {
    setContext(null);
    setItems([]);
    setNext(null);
    setSelected('');
    setDetail(null);
    setQuery(null);
    setErrors({ context: '', list: '', detail: '', action: '' });
    setReview(null);
  }, [host]);
  useEffect(() => {
    if (review && review.fingerprint !== confirmationToken(review.request.route)) {
      setReview(null);
      setReviewMessage(
        'The reviewed binding context has changed. Review the current records again.',
      );
    }
  }, [data.snapshot_id, context, detail, host, review]);
  useEffect(() => {
    if (review) dialog.current?.showModal();
    else dialog.current?.close();
  }, [review]);
  useEffect(() => {
    if (!readable) return;
    const controller = new AbortController();
    const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]);
    void api(
      `/api/v1/host-recovery-context?${new URLSearchParams({ host, origin: cell.cell.value.id })}`,
      { signal },
    )
      .then((raw) => validateRecoveryContext(raw, data, host, cell.cell.value.id))
      .then((value) => {
        if (!controller.signal.aborted) {
          setContext(value);
          setContextReadAt(performance.now());
          setErrors((old) => ({ ...old, context: '' }));
        }
      })
      .catch((error) => {
        if (!controller.signal.aborted) setErrors((old) => ({ ...old, context: explain(error) }));
      });
    return () => controller.abort();
  }, [host, readable, data.snapshot_id, reload]);
  useEffect(() => {
    if (!readable) return;
    const controller = new AbortController();
    const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]);
    void api(`/api/v1/host-recoveries?${new URLSearchParams({ host, limit: '20' })}`, { signal })
      .then((raw) => validateRecoveryPage(raw, host))
      .then((value) => {
        if (!controller.signal.aborted) {
          setItems(value.items);
          setNext(value.next);
          setErrors((old) => ({ ...old, list: '' }));
        }
      })
      .catch((error) => {
        if (!controller.signal.aborted) setErrors((old) => ({ ...old, list: explain(error) }));
      });
    return () => controller.abort();
  }, [host, readable, reload]);
  useEffect(() => {
    if (!readable || !selected) return;
    const controller = new AbortController();
    void api(`/api/v1/host-recovery?${new URLSearchParams({ id: selected })}`, {
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
    })
      .then((raw) => validateRecoveryView(raw, host, selected))
      .then((value) => {
        if (!controller.signal.aborted) {
          setDetail(value);
          setDetailReadAt(performance.now());
          setErrors((old) => ({ ...old, detail: '' }));
        }
      })
      .catch((error) => {
        if (!controller.signal.aborted) setErrors((old) => ({ ...old, detail: explain(error) }));
      });
    return () => controller.abort();
  }, [host, selected, readable, data.snapshot_id, reload]);
  useEffect(() => {
    if (
      receipt?.view.binding.context.host === host &&
      receipt.view.binding.context.installation === data.installation.id &&
      receipt.view.binding.context.store_generation === data.installation.store_generation
    ) {
      setSelected(receipt.view.binding.id);
      setDetail(receipt);
      setDetailReadAt(performance.now());
      setReload((v) => v + 1);
    }
  }, [receipt, host]);

  async function more() {
    if (!next || paging || !readable) return;
    setPaging(true);
    const requestedHost = host;
    try {
      const page = await validateRecoveryPage(
        await api(
          `/api/v1/host-recoveries?${new URLSearchParams({ host, after: next, limit: '20' })}`,
        ),
        host,
      );
      if (hostRef.current !== requestedHost) return;
      setItems((old) =>
        Array.from(new Map([...old, ...page.items].map((v) => [v.view.binding.id, v])).values()),
      );
      setNext(page.next);
    } catch (error) {
      if (hostRef.current === requestedHost) setErrors((old) => ({ ...old, list: explain(error) }));
    } finally {
      setPaging(false);
    }
  }
  async function openReview(kind: 'propose' | 'approve') {
    if (kind === 'propose' ? !canPropose || !currentContext : !canApprove || !currentDetail) return;
    const request =
      kind === 'propose'
        ? await freezeRecoveryProposal(currentContext!, data, crypto.randomUUID())
        : await freezeRecoveryApproval(currentDetail!, data, crypto.randomUUID());
    setReviewMessage('');
    setReview({
      request,
      fingerprint: confirmationToken(request.route),
      scope: structuredClone(
        kind === 'propose' ? currentContext!.context : currentDetail!.view.binding.context,
      ),
    });
  }
  async function action(operation?: string) {
    if (
      !canAdvance ||
      !currentDetail ||
      !binding ||
      (operation && !binding.context.operations[operation])
    )
      return;
    const selectedId = binding.id;
    const requestedHost = host;
    setErrors((old) => ({ ...old, action: '' }));
    try {
      const raw = await onAction(
        operation ? '/api/v1/host-recovery/query' : '/api/v1/host-recovery/progress',
        operation ? { id: selectedId, operation } : { id: selectedId },
      );
      if (hostRef.current !== requestedHost) return;
      if (operation) setQuery(validateRecoveryQuery(raw, currentDetail, operation));
      else {
        const value = await validateRecoveryProgress(raw, currentDetail);
        setDetail(value);
        setDetailReadAt(performance.now());
        setReload((v) => v + 1);
      }
    } catch (error) {
      if (hostRef.current === requestedHost)
        setErrors((old) => ({
          ...old,
          action: `${explain(error)} The outcome cannot be verified. Do not infer failure or non-execution.`,
        }));
    }
  }
  return (
    <section className="panel records" aria-label="Host recovery inspection binding">
      <div className="section-heading">
        <div>
          <p className="eyebrow">HOST RECOVERY</p>
          <h2>Host recovery inspection binding</h2>
        </div>
        <button onClick={() => setReload((v) => v + 1)} disabled={!readable}>
          Refresh current records
        </button>
      </div>
      <p className="muted">
        Check existing binding evidence and affected scope to establish recovery inspection
        communication. Resuming operation requires separate approval.
      </p>
      {!role && (
        <p className="notice">
          Use a registered terminal with release manager authority to review this binding.
        </p>
      )}
      <label>
        Select a Host in the current cell
        <select
          value={host}
          disabled={working || !!review}
          onChange={(e) => setHostChoice(e.target.value)}
        >
          <option value="">Select a Host to inspect</option>
          {cell.cell.value.hosts.map((name) => (
            <option key={name}>{name}</option>
          ))}
        </select>
      </label>
      {host && (
        <>
          {errors.context && (
            <p className="error" role="alert">
              {errors.context}
            </p>
          )}
          {currentContext && (
            <>
              <h3>Current binding context</h3>
              {!contextCurrent && (
                <p className="notice">
                  This differs from the current installation or cell records. It is shown as
                  historical, and new requests are blocked.
                </p>
              )}
              <Scope context={currentContext.context} />
              {currentContext.context.blockers.length ? (
                <ul>
                  {currentContext.context.blockers.map((reason, i) => (
                    <li key={i}>
                      {blockers[reason.kind] ?? reason.kind}
                      {'cell' in reason ? ` · ${reason.cell}` : ''}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted">
                  No blocking reasons were recorded in this read. Current conditions are checked
                  again at proposal and approval.
                </p>
              )}
              <button
                className="primary"
                disabled={!canPropose}
                onClick={() => void openReview('propose')}
              >
                Review recovery binding proposal
              </button>
            </>
          )}
          <h3>Stored recovery records</h3>
          <p className="muted">
            You can review records created by another release manager after checking their targets
            and contents.
          </p>
          {errors.list && (
            <p className="error" role="alert">
              {errors.list}
            </p>
          )}
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Record</th>
                  <th>Anchor cell</th>
                  <th>Proposed by</th>
                  <th>Stored state</th>
                  <th>Review</th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => (
                  <tr key={item.view.binding.id}>
                    <td className="mono">{short(item.view.binding.id)}</td>
                    <td>{item.view.binding.context.origin}</td>
                    <td>{item.view.binding.proposed_by.principal}</td>
                    <td>{phases[item.view.binding.phase]}</td>
                    <td>
                      <button
                        disabled={working || !!review}
                        onClick={() => {
                          setSelected(item.view.binding.id);
                          setQuery(null);
                        }}
                      >
                        Open record
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!items.length && !errors.list && (
            <p className="muted">No recovery records have been retrieved yet.</p>
          )}
          {next && (
            <button disabled={paging || !readable} onClick={() => void more()}>
              Load next records
            </button>
          )}
          {errors.detail && (
            <p className="error" role="alert">
              {errors.detail} This is the last retrieved record.
            </p>
          )}
          {binding && currentDetail && (
            <article className="inset">
              <h3>{phases[binding.phase]}</h3>
              <p className="mono">
                Record {binding.id} · r{binding.revision}
              </p>
              <p>
                Approval to resume operation is required · this binding has no task execution
                authority.
              </p>
              <p className="muted">
                {detailCurrent
                  ? 'Checked against the current cell records.'
                  : 'This historical record differs from the current installation and cell context.'}{' '}
                {currentDetail.view.current
                  ? 'The recovery inspection binding was verified in the last read.'
                  : 'The freshness of the recovery inspection binding needs to be checked again.'}
              </p>
              {!sameOrigin && (
                <p>
                  Anchor cell {binding.context.origin}: review the approval details in this cell.
                  <button onClick={() => onSelectCell(binding.context.origin)} disabled={working}>
                    Open anchor cell
                  </button>
                </p>
              )}
              <Scope context={binding.context} />
              {!!binding.context.blockers.length && (
                <ul>
                  {binding.context.blockers.map((r, i) => (
                    <li key={i}>
                      {blockers[r.kind] ?? r.kind}
                      {'cell' in r ? ` · ${r.cell}` : ''}
                    </li>
                  ))}
                </ul>
              )}
              {binding.detail && (
                <details>
                  <summary>Recorded reasons for review</summary>
                  <p className="muted">{binding.detail}</p>
                </details>
              )}
              <h4>Actions included in approval</h4>
              <p>
                Verify fencing of operating authority using the existing request key, and inspect
                receipts and observations for the stored original operations.
              </p>
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>Host-connected cells</th>
                      <th>Original fence request</th>
                      <th>Verification state</th>
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(binding.fences).map(([name, fence]) => (
                      <tr key={name}>
                        <td>{name}</td>
                        <td className="mono">{fence.task.request}</td>
                        <td>
                          {fence.phase === 'ACKNOWLEDGED'
                            ? 'Fence receipt confirmation'
                            : fence.phase === 'SEND_ENTERED'
                              ? 'Dispatch entered · receipt verification required'
                              : 'Not sent'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <button
                className="primary"
                disabled={!canApprove}
                onClick={() => void openReview('approve')}
              >
                {binding.approved_by
                  ? 'Review approval for this record again'
                  : 'Review recovery inspection binding approval'}
              </button>
              <button disabled={!canAdvance} onClick={() => void action()}>
                Advance approved fence request and check binding
              </button>
              <p className="muted">
                There is no need to rush approval before the read expires. The server obtains a
                fresh read and rechecks current authority and scope.
              </p>
              <RecoveryEvidence binding={binding} />
              <h4>Inspect original operation records</h4>
              {Object.values(binding.context.operations).map((operation) => (
                <p key={operation.operation}>
                  <span className="mono">{operation.operation}</span> · {operation.cell}{' '}
                  <button
                    disabled={!canAdvance || binding.phase === 'FENCING'}
                    onClick={() => void action(operation.operation)}
                  >
                    Inspect original receipts and observations
                  </button>
                </p>
              ))}
              {!Object.keys(binding.context.operations).length && (
                <p className="muted">
                  No original operations are associated with this recovery record.
                </p>
              )}
              {query?.binding === binding.id && (
                <div className="notice" role="status">
                  <b>Operation inspection results</b>
                  <p className="mono">{query.operation}</p>
                  <p>
                    {query.receipt
                      ? `Stored receipt stage · ${query.receipt.state}`
                      : 'The original receipt has not been verified yet.'}
                  </p>
                  <p>
                    Observations: {count(query.evidence.length, 'item')} · the complete evidence has
                    not been verified. Empty results do not establish failure or non-execution.
                  </p>
                  <p>
                    {query.lookup === 'UNAVAILABLE'
                      ? 'Cannot verify the original record inspection connection.'
                      : query.lookup === 'UNSUPPORTED'
                        ? 'This binding does not support further inspection.'
                        : 'Displaying original record inspection results.'}
                  </p>
                  {query.publication_required && (
                    <p>Verify that the collected records have been applied.</p>
                  )}
                </div>
              )}
            </article>
          )}
          {errors.action && (
            <p className="error" role="alert">
              {errors.action}
            </p>
          )}
          {reviewMessage && (
            <p role="status" className="notice">
              {reviewMessage}
            </p>
          )}
        </>
      )}
      <dialog
        ref={dialog}
        aria-labelledby="host-recovery-confirm"
        onCancel={(event) => {
          if (working) event.preventDefault();
          else setReview(null);
        }}
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (
              !review ||
              review.fingerprint !== confirmationToken(review.request.route) ||
              (review.request.route === '/api/v1/host-recoveries' ? !canPropose : !canApprove)
            )
              return;
            void onSubmit(review.request).finally(() => {
              setReview(null);
              setReload((v) => v + 1);
            });
          }}
        >
          <p className="eyebrow">REVIEW RECOVERY CONNECTION</p>
          <h2 id="host-recovery-confirm">
            {review?.request.route === '/api/v1/host-recoveries'
              ? 'Propose recovery for this binding?'
              : 'Approve the recovery inspection binding for this scope?'}
          </h2>
          {review && (
            <>
              <p>
                {review.scope.host} · anchor cell {review.scope.origin}
              </p>
              <Scope context={review.scope} />
              <p>
                Connect only the existing fence request and original operation record inspection.
                Resuming operation requires separate approval.
              </p>
            </>
          )}
          <div className="dialog-actions">
            <button type="button" disabled={working} onClick={() => setReview(null)}>
              Back
            </button>
            <button
              className="primary"
              disabled={
                !canWrite ||
                (review?.request.route === '/api/v1/host-recoveries' ? !canPropose : !canApprove)
              }
            >
              {working
                ? 'Requesting…'
                : review?.request.route === '/api/v1/host-recoveries'
                  ? 'Submit reviewed binding proposal'
                  : 'Approve reviewed scope'}
            </button>
          </div>
        </form>
      </dialog>
    </section>
  );
}
