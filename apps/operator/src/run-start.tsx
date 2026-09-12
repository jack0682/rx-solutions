import { useEffect, useRef, useState } from 'react';
import { api, ApiFailure, explain } from './api';
import { canRecover } from './pending';
import { short, text } from './labels';
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
  PENDING: '시작 대기',
  ARMING: '시작 접수 · Host 확인 중',
  STARTED: '시작 확정',
  REJECTED: '시작 거부 기록',
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
    <section className="panel records" aria-label="선택 실행 시작과 상태">
      <div className="section-heading">
        <div>
          <p className="eyebrow">RUN START</p>
          <h3>실행 시작과 상태</h3>
        </div>
        <button onClick={() => setReload((value) => value + 1)} disabled={!fresh}>
          다시 조회
        </button>
      </div>
      <label>
        실행 기록 선택
        <select
          value={selectedRun}
          onChange={(event) => onSelectRun(event.target.value)}
          disabled={!!review || working}
        >
          <option value="">시작하거나 조회할 실행을 선택하세요</option>
          {selectedRun && !selected && (
            <option value={selectedRun}>{short(selectedRun)} · 현재 목록 밖 기록</option>
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
          <p className="mono">실행 {selectedRun}</p>
          <p>
            {fresh ? '현재 실행 상태' : '마지막 조회 실행 상태'} · {text(attemptDisplay.runState)}
          </p>
          {(!selected || selected.value.state === 'PREPARED') && (
            <>
              <label>
                소재 시도 수량
                <input
                  inputMode="numeric"
                  value={requestedQuantity}
                  placeholder="양의 정수를 직접 입력하세요"
                  onChange={(event) => setQuantity(event.target.value)}
                  disabled={!!fixedBudget || !!review || working}
                  aria-describedby="start-quantity-help"
                />
              </label>
              <p id="start-quantity-help" className="muted">
                생산 · 소재 시도 예산입니다. 확인 완료 수량과는 다릅니다.
                {fixedBudget ? ` 기존 ${fixedBudget.limit}회 예산은 변경할 수 없습니다.` : ''}
                {contextCurrent
                  ? ` 이 셀의 최대 허용 수량은 ${context.maximum_budget}회입니다.`
                  : ''}
              </p>
              {!positiveQuantity(requestedQuantity) && (
                <p className="muted">수량을 입력하면 현재 시작 조건을 조회합니다.</p>
              )}
              {contextError && (
                <p role="alert" className="error">
                  {contextError} 시작 조건을 다시 조회해야 합니다.
                </p>
              )}
              {contextCurrent && (
                <>
                  <dl className="facts">
                    <div>
                      <dt>검토한 기록</dt>
                      <dd>
                        셀 r{context.cell_revision} / 실행 r{context.run_revision}
                      </dd>
                    </div>
                    <div>
                      <dt>운전 자격 기록</dt>
                      <dd>{text(context.commissioning ?? 'UNKNOWN')}</dd>
                    </div>
                    <div>
                      <dt>시작 조건 조회</dt>
                      <dd>
                        {!contextFresh
                          ? '최신 조회 필요'
                          : context.can_request
                            ? '요청 전 검사 통과'
                            : '현재 시작 요청 차단'}
                      </dd>
                    </div>
                  </dl>
                  {context.blocking_reason && (
                    <p className="notice" role="status">
                      {context.blocking_reason === 'BUSY'
                        ? '현재 시작 시도 또는 실행 연결을 확인해야 합니다. 운전 조건과 실행 기록을 확인하세요.'
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
                시작 내용 검토
              </button>
              <p className="muted">
                조회 결과는 운전 허가가 아닙니다. 시작 요청과 Host 확인 시 현재 단말·자격·조건을
                다시 검사합니다.
              </p>
            </>
          )}
          {attemptId && (
            <div className="inset" role="status">
              <b>
                {attempt && attemptDisplay.attemptMatches
                  ? `${attemptFresh ? '' : '마지막 조회 · '}${attemptLabels[attempt.attempt.status]}`
                  : '저장된 시작 시도 조회 중'}
              </b>
              <p className="mono">시작 시도 {attemptId}</p>
              {attemptError && <p className="error">{attemptError}</p>}
              {attempt && attemptDisplay.attemptMatches && (
                <>
                  <p>
                    Host 확인 {Object.keys(attempt.attempt.acknowledgments).length} /{' '}
                    {Object.keys(attempt.attempt.host_boots).length}
                    {' · '}저장 상태 {attempt.attempt.status}
                  </p>
                  <p>
                    시작 확인 기한 ·{' '}
                    {attempt.deadline_status === 'WITHIN_DEADLINE'
                      ? '기한 내'
                      : attempt.deadline_status === 'ELAPSED'
                        ? '기한 경과'
                        : '시간 기준 변경'}
                  </p>
                  {!attemptFresh && (
                    <p>
                      마지막 확인 기록입니다. 현재 실행 기록과 일치하는 최신 시작 상태를 다시
                      조회해야 합니다.
                    </p>
                  )}
                  {attempt.deadline_status !== 'WITHIN_DEADLINE' &&
                    (attempt.attempt.status === 'ARMING' ||
                      attempt.attempt.status === 'PENDING') && (
                      <p className="error">
                        {attempt.deadline_status === 'ELAPSED'
                          ? '시작 확인 기한 경과'
                          : '시작 확인 시간 기준 변경'}
                        {' · '}저장 상태는 {attempt.attempt.status}입니다. 조정이 필요하며 자동으로
                        재시작하지 않습니다.
                      </p>
                    )}
                  {attempt.attempt.status === 'ARMING' && (
                    <p>시작 요청 기록을 수신했습니다. 실제 시작 확정은 아직 확인되지 않았습니다.</p>
                  )}
                </>
              )}
            </div>
          )}
          <p className="muted">
            소재별 완료 집계는 아직 제공되지 않습니다. 실행 기록과 작업 결과·자원 인계를 각각
            확인하세요.
          </p>
        </>
      ) : (
        <p className="empty-inline">
          실행 기록을 선택하세요. 가장 최근 실행을 자동으로 시작하지 않습니다.
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
          <h2 id="start-title">이 실행을 시작할까요?</h2>
          {review && (
            <>
              <p>
                <b>{review.context.cell}</b> ·{' '}
                {review.context.environment === 'SIMULATION' ? '모의 환경' : '실장비'}
              </p>
              <dl className="config-list">
                <div>
                  <dt>정확한 실행 번호</dt>
                  <dd>{review.context.run.id}</dd>
                </div>
                <div>
                  <dt>생산 소재 시도 수량</dt>
                  <dd>{String(review.request.command.budget_limit)}회</dd>
                </div>
                <div>
                  <dt>검토한 버전</dt>
                  <dd>
                    셀 r{review.context.cell_revision} / 실행 r{review.context.run_revision}
                  </dd>
                </div>
                <div>
                  <dt>공정</dt>
                  <dd>{review.context.recipe.sha256}</dd>
                </div>
                <div>
                  <dt>운전 범위</dt>
                  <dd>{review.context.envelope.sha256}</dd>
                </div>
              </dl>
              <p>
                현재 조건을 다시 검사해 시작 시도를 기록합니다. Host 확인이 끝난 뒤 저장된 시작
                확정을 확인합니다.
              </p>
              {!reviewCurrent && (
                <p className="error" role="alert">
                  검토 후 상태가 변경되었습니다. 돌아가서 최신 내용을 다시 검토하세요.
                </p>
              )}
              {reviewCurrent && !reviewAllowed && (
                <p className="error" role="alert">
                  현재 시작 조건을 다시 확인해야 합니다. 요청 내용은 검토한 값으로 유지됩니다.
                </p>
              )}
            </>
          )}
          <div className="dialog-actions">
            <button type="button" onClick={() => setReview(null)} disabled={working}>
              돌아가기
            </button>
            <button type="submit" className="primary" disabled={!reviewAllowed}>
              {working ? '요청 중…' : '검토한 수량으로 시작 요청'}
            </button>
          </div>
        </form>
      </dialog>
    </section>
  );
}
