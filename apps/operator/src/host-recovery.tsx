import { useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import { short, text } from './labels';
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
  PROPOSED: '승인 전 제안',
  FENCING: '기존 운전 권한 차단 확인 중',
  RECOVERY_ONLY: '복구 조회 연결',
  ATTENTION: '담당자 확인 필요',
};
const blockers: Record<string, string> = {
  BASELINE_MISSING: '원래 연결 근거 없음',
  REGISTRATION_MISSING: '원래 Host 등록 없음',
  IDENTITY_CHANGED: '연결 대상 식별 정보 변경',
  RESTART_ORIGIN_MISSING: '재시작 차단 근거 없음',
  LIVE_AUTHORITY: '기존 실행 또는 운전 권한 남음',
  TOO_MANY_OPERATIONS: '확인 대상 작업 수 범위 초과',
  FENCE_CANDIDATES_AMBIGUOUS: '원래 차단 요청을 하나로 확인할 수 없음',
};

function Scope({ context }: { context: RecoveryContext }) {
  return (
    <div className="table-scroll">
      <table>
        <thead>
          <tr>
            <th>영향 셀</th>
            <th>기록 버전</th>
            <th>현재 차단</th>
          </tr>
        </thead>
        <tbody>
          {Object.entries(context.cells).map(([name, cut]) => (
            <tr key={name}>
              <td>
                {name}
                {context.host_cells.includes(name) ? ' · Host 연결 셀' : ' · 공유 영향 셀'}
              </td>
              <td>r{cut.revision}</td>
              <td>
                {cut.blocks.length
                  ? cut.blocks.map((block) => text(block.reason)).join(', ')
                  : '기록 없음'}
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
      <h4>마지막 Host 관측</h4>
      {readEvidence.configuration.receipt === null ? (
        <p>이번 조회에는 적용 수신 기록이 포함되지 않았습니다.</p>
      ) : !readEvidence.configuration.context_matches_current_host ? (
        <p className="notice">
          동봉된 적용 수신 기록과 현재 Host 상태의 일치가 확인되지 않았습니다.
        </p>
      ) : null}
      {binding.phase === 'ATTENTION' && (
        <p className="notice">
          확인 필요 상태에서 보관된 관측입니다. 현재 연결의 근거로 사용하지 않습니다.
        </p>
      )}
      {Object.entries(readEvidence.cells).map(([name, read]) => (
        <div key={name}>
          <b>
            {name} · {read.snapshot.sources_available ? '저장된 관측 있음' : '관측 소스 조회 불가'}
          </b>
          <p className="muted">
            {read.snapshot.observations.length}개 관측 · 현재 운전 조건 충족을 뜻하지 않습니다.
          </p>
          {read.snapshot.observations.map((o) => (
            <p key={o.source}>
              {o.source} · {stableDocument(o.value)} ·
              {o.quality_good && o.origin_age_bounded && !o.disputed
                ? ' 저장된 품질 필드 확인'
                : ' 품질·시각·상충 확인 필요'}
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
      setReviewMessage('검토한 연결 문맥이 변경되었습니다. 현재 기록을 다시 검토하세요.');
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
          action: `${explain(error)} 결과를 확인할 수 없습니다. 실패나 미실행으로 판단하지 마세요.`,
        }));
    }
  }
  return (
    <section className="panel records" aria-label="Host 복구 조회 연결">
      <div className="section-heading">
        <div>
          <p className="eyebrow">HOST RECOVERY</p>
          <h2>Host 복구 조회 연결</h2>
        </div>
        <button onClick={() => setReload((v) => v + 1)} disabled={!readable}>
          현재 기록 다시 조회
        </button>
      </div>
      <p className="muted">
        기존 연결 근거와 영향 범위를 확인해 복구 조회 통신을 연결합니다. 운전 재개는 별도 승인이
        필요합니다.
      </p>
      {!role && <p className="notice">등록 단말에서 배포 담당자 권한으로 확인할 수 있습니다.</p>}
      <label>
        현재 셀의 Host 선택
        <select
          value={host}
          disabled={working || !!review}
          onChange={(e) => setHostChoice(e.target.value)}
        >
          <option value="">확인할 Host를 선택하세요</option>
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
              <h3>현재 연결 문맥</h3>
              {!contextCurrent && (
                <p className="notice">
                  현재 설치 또는 셀 기록과 다릅니다. 이전 기록으로 표시하며 새 요청은 차단됩니다.
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
                  이 읽기에서 기록된 차단 사유가 없습니다. 제안·승인 때 현재 조건을 다시 검사합니다.
                </p>
              )}
              <button
                className="primary"
                disabled={!canPropose}
                onClick={() => void openReview('propose')}
              >
                복구 연결 제안 검토
              </button>
            </>
          )}
          <h3>저장된 복구 기록</h3>
          <p className="muted">
            다른 배포 담당자가 남긴 기록도 대상과 내용을 확인한 뒤 검토할 수 있습니다.
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
                  <th>기록</th>
                  <th>기준 셀</th>
                  <th>제안자</th>
                  <th>저장 상태</th>
                  <th>검토</th>
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
                        기록 열기
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!items.length && !errors.list && (
            <p className="muted">아직 조회된 복구 기록이 없습니다.</p>
          )}
          {next && (
            <button disabled={paging || !readable} onClick={() => void more()}>
              다음 기록 보기
            </button>
          )}
          {errors.detail && (
            <p className="error" role="alert">
              {errors.detail} 마지막 조회 기록입니다.
            </p>
          )}
          {binding && currentDetail && (
            <article className="inset">
              <h3>{phases[binding.phase]}</h3>
              <p className="mono">
                기록 {binding.id} · r{binding.revision}
              </p>
              <p>운전 재개 승인 필요 · 이 연결에는 작업 실행 권한이 없습니다.</p>
              <p className="muted">
                {detailCurrent
                  ? '현재 셀 기록과 대조했습니다.'
                  : '현재 설치·셀 문맥과 다른 이전 기록입니다.'}{' '}
                {currentDetail.view.current
                  ? '마지막 읽기에서 복구 조회 연결을 확인했습니다.'
                  : '복구 조회 연결의 현재성을 다시 확인해야 합니다.'}
              </p>
              {!sameOrigin && (
                <p>
                  기준 셀 {binding.context.origin}에서 승인 내용을 검토하세요.
                  <button onClick={() => onSelectCell(binding.context.origin)} disabled={working}>
                    기준 셀 열기
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
                  <summary>기록된 확인 사유</summary>
                  <p className="muted">{binding.detail}</p>
                </details>
              )}
              <h4>승인에 포함되는 동작</h4>
              <p>
                기존 요청 번호로 운전 권한 차단을 확인하고, 저장된 원래 작업의 수신 기록과 관측을
                조회합니다.
              </p>
              <div className="table-scroll">
                <table>
                  <thead>
                    <tr>
                      <th>Host 연결 셀</th>
                      <th>원래 차단 요청</th>
                      <th>확인 상태</th>
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(binding.fences).map(([name, fence]) => (
                      <tr key={name}>
                        <td>{name}</td>
                        <td className="mono">{fence.task.request}</td>
                        <td>
                          {fence.phase === 'ACKNOWLEDGED'
                            ? '차단 수신 확인 기록'
                            : fence.phase === 'SEND_ENTERED'
                              ? '송신 진입 · 수신 확인 필요'
                              : '전달 전'}
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
                {binding.approved_by ? '같은 기록의 승인 다시 검토' : '복구 조회 연결 승인 검토'}
              </button>
              <button disabled={!canAdvance} onClick={() => void action()}>
                승인한 차단 요청 진행·연결 확인
              </button>
              <p className="muted">
                읽기의 유효시간이 짧아도 승인 버튼을 급히 누를 필요는 없습니다. 서버가 새 읽기와
                현재 권한·범위를 다시 검사합니다.
              </p>
              <RecoveryEvidence binding={binding} />
              <h4>원래 작업 기록 조회</h4>
              {Object.values(binding.context.operations).map((operation) => (
                <p key={operation.operation}>
                  <span className="mono">{operation.operation}</span> · {operation.cell}{' '}
                  <button
                    disabled={!canAdvance || binding.phase === 'FENCING'}
                    onClick={() => void action(operation.operation)}
                  >
                    원래 수신·관측 조회
                  </button>
                </p>
              ))}
              {!Object.keys(binding.context.operations).length && (
                <p className="muted">이 복구 기록에 연결된 원래 작업이 없습니다.</p>
              )}
              {query?.binding === binding.id && (
                <div className="notice" role="status">
                  <b>작업 조회 결과</b>
                  <p className="mono">{query.operation}</p>
                  <p>
                    {query.receipt
                      ? `저장된 수신 단계 · ${query.receipt.state}`
                      : '원래 수신 기록을 아직 확인하지 못했습니다.'}
                  </p>
                  <p>
                    관측 {query.evidence.length}개 · 전체 근거가 확인된 상태는 아닙니다. 빈 결과로
                    실패·미실행을 판단하지 않습니다.
                  </p>
                  <p>
                    {query.lookup === 'UNAVAILABLE'
                      ? '원래 기록 조회 연결을 확인할 수 없습니다.'
                      : query.lookup === 'UNSUPPORTED'
                        ? '이 연결에서 추가 조회를 지원하지 않습니다.'
                        : '원래 기록 조회 결과를 표시합니다.'}
                  </p>
                  {query.publication_required && <p>수집된 기록의 반영 확인이 필요합니다.</p>}
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
              ? '이 연결의 복구를 제안할까요?'
              : '이 범위의 복구 조회 연결을 승인할까요?'}
          </h2>
          {review && (
            <>
              <p>
                {review.scope.host} · 기준 셀 {review.scope.origin}
              </p>
              <Scope context={review.scope} />
              <p>
                기존 차단 요청과 원래 작업 기록 조회만 연결합니다. 운전 재개는 별도 승인이
                필요합니다.
              </p>
            </>
          )}
          <div className="dialog-actions">
            <button type="button" disabled={working} onClick={() => setReview(null)}>
              돌아가기
            </button>
            <button
              className="primary"
              disabled={
                !canWrite ||
                (review?.request.route === '/api/v1/host-recoveries' ? !canPropose : !canApprove)
              }
            >
              {working
                ? '요청 중…'
                : review?.request.route === '/api/v1/host-recoveries'
                  ? '검토한 연결 제안'
                  : '검토한 범위 승인'}
            </button>
          </div>
        </form>
      </dialog>
    </section>
  );
}
