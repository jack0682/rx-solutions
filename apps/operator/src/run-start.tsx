import { useEffect, useRef, useState } from 'react';
import { api, ApiFailure, explain } from './api';
import { canRecover } from './pending';
import { count, short, text } from './labels';
import type { CellOverview, Overview, Pending } from './schema';
import {
  attemptPath,
  contextIsCurrent,
  freezeStartRequest,
  positiveQuantity,
  startStatusDisplay,
  validateAttemptContext,
  validateStartContext,
  type AttemptContext,
  type StartContext,
  type StartWatch,
} from './run-start-schema';

const attemptLabels = {
  PENDING: 'Start pending',
  ARMING: 'Start accepted · awaiting Host confirmation',
  STARTED: 'Start confirmed',
  REJECTED: 'Start rejection recorded',
};

export function RunStart({
  data,
  cell,
  selectedRun,
  onSelectRun,
  canRequest,
  fresh,
  working,
  watch,
  onSubmit,
}: {
  data: Overview;
  cell: CellOverview;
  selectedRun: string;
  onSelectRun: (id: string) => void;
  canRequest: boolean;
  fresh: boolean;
  working: boolean;
  watch: StartWatch | null;
  onSubmit: (record: Pending) => Promise<void>;
}) {
  const [quantity, setQuantity] = useState('');
  const [context, setContext] = useState<StartContext | null>(null);
  const [attempt, setAttempt] = useState<AttemptContext | null>(null);
  const [contextError, setContextError] = useState('');
  const [attemptError, setAttemptError] = useState('');
  const [contextRead, setContextRead] = useState(0);
  const [attemptRead, setAttemptRead] = useState(0);
  const [now, setNow] = useState(performance.now());
  const [reload, setReload] = useState(0);
  const [review, setReview] = useState<{ context: StartContext; request: Pending } | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const selected = cell.runs.find((item) => item.value.id === selectedRun);
  const matchingWatch =
    watch &&
    canRecover(
      watch.request,
      data.user.principal,
      data.installation.id,
      data.installation.store_generation,
    ) &&
    watch.attempt.cell === cell.cell.value.id &&
    watch.attempt.run === selectedRun
      ? watch
      : null;
  const fixedBudget =
    selected?.value.budget ?? (context?.run.id === selectedRun ? context.run.budget : null);
  const requestedQuantity = fixedBudget?.limit ?? quantity;
  const attemptId = selected?.value.pending_attempt ?? matchingWatch?.attempt.id ?? '';
  const contextCurrent =
    !!context && contextIsCurrent(context, data, cell, selectedRun, requestedQuantity);
  const contextFresh = fresh && contextCurrent && !contextError && now - contextRead < 10000;
  const attemptDisplay = startStatusDisplay({
    attempt,
    data,
    cell,
    run: selectedRun,
    id: attemptId,
    queryFresh: fresh,
    queryFailed: !!attemptError,
    receivedAt: attemptRead,
    now,
  });
  const attemptFresh = attemptDisplay.attemptFresh;
  const canReview =
    canRequest &&
    contextFresh &&
    context?.can_request === true &&
    context.run.state === 'PREPARED' &&
    !context.run.pending_attempt;
  const reviewCurrent =
    !!review && contextIsCurrent(review.context, data, cell, selectedRun, requestedQuantity);
  const reviewAllowed =
    canRequest && reviewCurrent && contextFresh && context?.can_request === true;

  useEffect(() => {
    setQuantity('');
    setContext(null);
    setAttempt(null);
    setReview(null);
    setContextError('');
    setAttemptError('');
  }, [selectedRun]);
  useEffect(() => {
    const timer = setInterval(() => setNow(performance.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (review) dialog.current?.showModal();
    else dialog.current?.close();
  }, [review]);
  useEffect(() => {
    if (!selectedRun || !positiveQuantity(requestedQuantity) || !fresh) return;
    const controller = new AbortController();
    const query = new URLSearchParams({
      cell: cell.cell.value.id,
      run: selectedRun,
      purpose: 'PRODUCTION',
      budget_limit: requestedQuantity,
    });
    void api(`/api/v1/run/start-context?${query}`, {
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
    })
      .then((raw) => {
        if (controller.signal.aborted) return;
        setContext(
          validateStartContext(raw, data, cell.cell.value.id, selectedRun, requestedQuantity),
        );
        setContextRead(performance.now());
        setContextError('');
      })
      .catch((error) => {
        if (!controller.signal.aborted) setContextError(explain(error));
      });
    return () => controller.abort();
  }, [selectedRun, requestedQuantity, data.snapshot_id, fresh, reload]);
  useEffect(() => {
    if (!selectedRun || !attemptId || !fresh) return;
    const controller = new AbortController();
    void api(attemptPath(cell.cell.value.id, selectedRun, attemptId), {
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
    })
      .then((raw) => {
        if (controller.signal.aborted) return;
        setAttempt(
          validateAttemptContext(
            raw,
            data,
            cell.cell.value.id,
            selectedRun,
            attemptId,
            matchingWatch?.attempt.id === attemptId ? matchingWatch.request : undefined,
          ),
        );
        setAttemptRead(performance.now());
        setAttemptError('');
      })
      .catch((error) => {
        if (!controller.signal.aborted) setAttemptError(explain(error));
      });
    return () => controller.abort();
  }, [selectedRun, attemptId, data.snapshot_id, fresh, reload]);

  return (
    <section className="panel records" aria-label="Selected run start and status">
      <div className="section-heading">
        <div>
          <p className="eyebrow">RUN START</p>
          <h3>Run start and status</h3>
        </div>
        <button onClick={() => setReload((value) => value + 1)} disabled={!fresh}>
          Refresh
        </button>
      </div>
      <label>
        Select run record
        <select
          value={selectedRun}
          onChange={(event) => onSelectRun(event.target.value)}
          disabled={!!review || working}
        >
          <option value="">Select a run to start or inspect</option>
          {selectedRun && !selected && (
            <option value={selectedRun}>
              {short(selectedRun)} · record outside the current list
            </option>
          )}
          {cell.runs.map((item) => (
            <option key={item.value.id} value={item.value.id}>
              {short(item.value.id)} · {text(item.value.state)} · r{item.revision}
            </option>
          ))}
        </select>
      </label>
      {selectedRun ? (
        <>
          <p className="mono">Run {selectedRun}</p>
          <p>
            {fresh ? 'Current run state' : 'Last retrieved run state'} ·{' '}
            {text(attemptDisplay.runState)}
          </p>
          {(!selected || selected.value.state === 'PREPARED') && (
            <>
              <label>
                Material attempt count
                <input
                  inputMode="numeric"
                  value={requestedQuantity}
                  placeholder="Enter a positive integer"
                  onChange={(event) => setQuantity(event.target.value)}
                  disabled={!!fixedBudget || !!review || working}
                  aria-describedby="start-quantity-help"
                />
              </label>
              <p id="start-quantity-help" className="muted">
                Production · this is a material attempt budget, not a confirmed completion count.
                {fixedBudget
                  ? ` The existing ${fixedBudget.limit}-attempt budget cannot be changed.`
                  : ''}
                {contextCurrent
                  ? ` The maximum permitted count for this cell is ${count(context.maximum_budget, 'attempt')}.`
                  : ''}
              </p>
              {!positiveQuantity(requestedQuantity) && (
                <p className="muted">Enter a count to query the current start conditions.</p>
              )}
              {contextError && (
                <p role="alert" className="error">
                  {contextError} Refresh the start conditions.
                </p>
              )}
              {contextCurrent && (
                <>
                  <dl className="facts">
                    <div>
                      <dt>Reviewed records</dt>
                      <dd>
                        Cell r{context.cell_revision} / run r{context.run_revision}
                      </dd>
                    </div>
                    <div>
                      <dt>Operating qualification record</dt>
                      <dd>{text(context.commissioning ?? 'UNKNOWN')}</dd>
                    </div>
                    <div>
                      <dt>Start condition query</dt>
                      <dd>
                        {!contextFresh
                          ? 'Fresh query required'
                          : context.can_request
                            ? 'Pre-request checks passed'
                            : 'Start request currently blocked'}
                      </dd>
                    </div>
                  </dl>
                  {context.blocking_reason && (
                    <p className="notice" role="status">
                      {context.blocking_reason === 'BUSY'
                        ? 'Check the current start attempt or execution binding. Review operating conditions and run records.'
                        : explain(new ApiFailure(context.blocking_reason))}{' '}
                      · {context.blocking_reason}
                    </p>
                  )}
                </>
              )}
              <button
                className="primary"
                disabled={!canReview}
                onClick={() => {
                  if (context && canReview)
                    setReview({
                      context: structuredClone(context),
                      request: freezeStartRequest(context, data, crypto.randomUUID()),
                    });
                }}
              >
                Review start details
              </button>
              <p className="muted">
                Query results do not grant operating permission. The current terminal,
                qualification, and conditions are rechecked at the start request and Host
                confirmation.
              </p>
            </>
          )}
          {attemptId && (
            <div className="inset" role="status">
              <b>
                {attempt && attemptDisplay.attemptMatches
                  ? `${attemptFresh ? '' : 'Last retrieved · '}${attemptLabels[attempt.attempt.status]}`
                  : 'Loading stored start attempt'}
              </b>
              <p className="mono">Start attempt {attemptId}</p>
              {attemptError && <p className="error">{attemptError}</p>}
              {attempt && attemptDisplay.attemptMatches && (
                <>
                  <p>
                    Host confirmations {Object.keys(attempt.attempt.acknowledgments).length} /{' '}
                    {Object.keys(attempt.attempt.host_boots).length}
                    {' · '}Stored state {attempt.attempt.status}
                  </p>
                  <p>
                    Start confirmation deadline ·{' '}
                    {attempt.deadline_status === 'WITHIN_DEADLINE'
                      ? 'Within deadline'
                      : attempt.deadline_status === 'ELAPSED'
                        ? 'Deadline elapsed'
                        : 'Clock basis changed'}
                  </p>
                  {!attemptFresh && (
                    <p>
                      This is the last verified record. Retrieve the latest start state that matches
                      the current run record.
                    </p>
                  )}
                  {attempt.deadline_status !== 'WITHIN_DEADLINE' &&
                    (attempt.attempt.status === 'ARMING' ||
                      attempt.attempt.status === 'PENDING') && (
                      <p className="error">
                        {attempt.deadline_status === 'ELAPSED'
                          ? 'Start confirmation deadline elapsed'
                          : 'Start confirmation clock basis changed'}
                        {' · '}The stored state is {attempt.attempt.status}. Coordination is
                        required; no automatic restart occurs.
                      </p>
                    )}
                  {attempt.attempt.status === 'ARMING' && (
                    <p>
                      The start request record was received. Actual start confirmation has not yet
                      been verified.
                    </p>
                  )}
                </>
              )}
            </div>
          )}
          <p className="muted">
            Per-material completion totals are not yet available. Review run records, operation
            outcomes, and resource handover separately.
          </p>
        </>
      ) : (
        <p className="empty-inline">
          Select a run record. The most recent run is not started automatically.
        </p>
      )}
      <dialog
        ref={dialog}
        aria-labelledby="start-title"
        onCancel={(event) => {
          if (working) event.preventDefault();
          else setReview(null);
        }}
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!review || !reviewAllowed) return;
            const record = review.request;
            void onSubmit(record).finally(() => {
              setReview(null);
              setContext(null);
              setReload((value) => value + 1);
            });
          }}
        >
          <p className="eyebrow">REVIEW START REQUEST</p>
          <h2 id="start-title">Start this run?</h2>
          {review && (
            <>
              <p>
                <b>{review.context.cell}</b> ·{' '}
                {review.context.environment === 'SIMULATION' ? 'Simulation' : 'Physical equipment'}
              </p>
              <dl className="config-list">
                <div>
                  <dt>Exact run ID</dt>
                  <dd>{review.context.run.id}</dd>
                </div>
                <div>
                  <dt>Production material attempt count</dt>
                  <dd>{count(String(review.request.command.budget_limit), 'attempt')}</dd>
                </div>
                <div>
                  <dt>Reviewed revision</dt>
                  <dd>
                    Cell r{review.context.cell_revision} / run r{review.context.run_revision}
                  </dd>
                </div>
                <div>
                  <dt>Workflow</dt>
                  <dd>{review.context.recipe.sha256}</dd>
                </div>
                <div>
                  <dt>Operating envelope</dt>
                  <dd>{review.context.envelope.sha256}</dd>
                </div>
              </dl>
              <p>
                Recheck the current conditions and record a start attempt. After Host confirmation,
                verify the stored start confirmation.
              </p>
              {!reviewCurrent && (
                <p className="error" role="alert">
                  The state has changed since review. Go back and review the latest details.
                </p>
              )}
              {reviewCurrent && !reviewAllowed && (
                <p className="error" role="alert">
                  The current start conditions need to be checked again. The request retains the
                  reviewed values.
                </p>
              )}
            </>
          )}
          <div className="dialog-actions">
            <button type="button" onClick={() => setReview(null)} disabled={working}>
              Back
            </button>
            <button type="submit" className="primary" disabled={!reviewAllowed}>
              {working ? 'Requesting…' : 'Request start with reviewed count'}
            </button>
          </div>
        </form>
      </dialog>
    </section>
  );
}
