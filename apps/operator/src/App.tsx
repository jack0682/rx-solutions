import { Packages } from './packages';
import {
  packageRoute,
  validatePackageReceipt,
  emptyPackageBuffer,
  type PackageBuffer,
  type ReviewReceipt,
} from './package-schema';
import { bindingVersionSchema, type BindingVersion } from './draft-bindings-schema';
import { ProcessEditor } from './process-editor';
import {
  draftDetailSchema,
  fromDetail,
  stableDocument,
  type DraftBuffer,
  type DraftDetail,
} from './draft-schema';
import { Conditions } from './conditions';
import { text, short, time } from './labels';
import { Runs, Work, Configuration } from './views';
import { RunStart } from './run-start';
import { HostRecovery } from './host-recovery';
import {
  canRecoverHostRequest,
  recoveryRoute,
  validateRecoveryReceipt,
  type RecoveryTransientRoute,
  type RecoveryView,
} from './host-recovery-schema';
import {
  attemptPath,
  readStartWatch,
  saveStartWatch,
  validateAttemptContext,
  validateStartReceipt,
  type StartWatch,
} from './run-start-schema';
import { Cases } from './cases';
import { useState, useEffect, useRef, useCallback, type FormEvent } from 'react';
import { z } from './schema-runtime';
import { api, ApiFailure, explain } from './api';
import {
  overviewSchema,
  runSchema,
  profileSchema,
  caseDetailSchema,
  type Overview,
  type CellOverview,
  type Pending,
} from './schema';
import { readPending, savePending, clearPending, canRecover, requestBody } from './pending';

export function App() {
  const [phase, setPhase] = useState<'checking' | 'signed-out' | 'active'>('checking');
  const [data, setData] = useState<Overview | null>(null);
  const [packageBuffers, setPackageBuffers] = useState<Record<string, PackageBuffer>>({});
  const [packageReceipt, setPackageReceipt] = useState<ReviewReceipt | null>(null);
  const [draftBuffers, setDraftBuffers] = useState<Record<string, DraftBuffer | null>>({});
  const [bindingReceipt, setBindingReceipt] = useState<BindingVersion | null>(null);
  const [draftReceipt, setDraftReceipt] = useState<DraftDetail | null>(null);
  const authoringContext = useRef('');
  const [lastRead, setLastRead] = useState(0);
  const [readStarted, setReadStarted] = useState(0);
  const [lastReadSteady, setLastReadSteady] = useState(0);
  const [now, setNow] = useState(performance.now());
  const [error, setError] = useState('');
  const [toast, setToast] = useState('');
  const [tab, setTab] = useState('운영');
  const [selected, setSelected] = useState('');
  const [dialog, setDialog] = useState<{ kind: 'run' | 'hold'; cell: CellOverview } | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [startWatch, setStartWatch] = useState<StartWatch | null>(null);
  const [recoveryReceipt, setRecoveryReceipt] = useState<RecoveryView | null>(null);
  const [runSelections, setRunSelections] = useState<Record<string, string>>({});
  const [storageError, setStorageError] = useState(false);
  const [working, setWorking] = useState(false);
  const busy = useRef(false);
  const request = useRef<AbortController | null>(null);
  const generation = useRef(0);
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    try {
      setPending(readPending(sessionStorage));
      setStartWatch(readStartWatch(sessionStorage));
    } catch {
      setStorageError(true);
    }
  }, []);
  const refresh = useCallback(async () => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    const current = ++generation.current;
    const started = performance.now();
    try {
      const result = overviewSchema.parse(
        await api('/api/v1/overview', {
          signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]),
        }),
      );
      if (current !== generation.current) return;
      const context = JSON.stringify([
        result.user.principal,
        result.installation.id,
        result.installation.store_generation,
      ]);
      if (authoringContext.current !== context) {
        authoringContext.current = context;
        setDraftBuffers({});
        setPackageBuffers({});
        setPackageReceipt(null);
        setDraftReceipt(null);
        setBindingReceipt(null);
        setRunSelections({});
        setRecoveryReceipt(null);
      }
      setData(result);
      setReadStarted(started);
      setLastReadSteady(performance.now());
      setPhase('active');
      setLastRead(Date.now());
      setError('');
      setSelected((previous) =>
        result.cells.some((c) => c.cell.value.id === previous)
          ? previous
          : (result.cells[0]?.cell.value.id ?? ''),
      );
    } catch (e) {
      if (controller.signal.aborted || current !== generation.current) return;
      if (e instanceof ApiFailure && (e.status === 401 || e.status === 403)) {
        setData(null);
        setPhase('signed-out');
        setDraftBuffers({});
        setPackageBuffers({});
        setPackageReceipt(null);
        setDraftReceipt(null);
        setBindingReceipt(null);
      } else setError(explain(e));
    }
  }, []);
  useEffect(() => {
    void refresh();
    return () => {
      generation.current++;
      request.current?.abort();
    };
  }, [refresh]);
  useEffect(() => {
    const timer = setInterval(() => setNow(performance.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (phase !== 'active') return;
    const timer = setInterval(
      () => {
        if (document.visibilityState === 'visible') void refresh();
      },
      tab === '운전 조건' ? 500 : 3000,
    );
    const visible = () => {
      if (document.visibilityState === 'visible') void refresh();
    };
    document.addEventListener('visibilitychange', visible);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', visible);
    };
  }, [phase, refresh, tab]);
  useEffect(() => {
    if (dialog) dialogRef.current?.showModal();
    else dialogRef.current?.close();
  }, [dialog]);

  const hasUnsavedDraft = Object.values(draftBuffers).some(
    (d) => d?.dirty || d?.sourceText != null || d?.conditionText != null || d?.bindingEdit != null,
  );
  useEffect(() => {
    if (!hasUnsavedDraft) return;
    const guard = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', guard);
    return () => window.removeEventListener('beforeunload', guard);
  }, [hasUnsavedDraft]);
  const fresh = !!data && !error && now - lastReadSteady < 10000;
  const cell = data?.cells.find((c) => c.cell.value.id === selected);
  const canOperate = !!data?.user.roles.includes('OPERATOR');
  const recoverableStart =
    data &&
    pending?.route === '/api/v1/runs/start' &&
    canRecover(
      pending,
      data.user.principal,
      data.installation.id,
      data.installation.store_generation,
    )
      ? pending
      : null;
  const canAuthor = !!data?.user.roles.includes('ENGINEER');
  const canReadDrafts = canAuthor || !!data?.user.roles.includes('VERIFIER');
  const canEditDraft = canAuthor && !pending && !working && !storageError;
  const canSaveDraft = fresh && canEditDraft;
  const canRequest = fresh && canOperate && !pending && !working && !storageError;
  const canRecoverHost =
    fresh &&
    !!data?.user.roles.includes('RELEASE_MANAGER') &&
    !!data?.user.terminal &&
    !pending &&
    !working &&
    !storageError;

  const canAcknowledge =
    fresh &&
    !!data?.user.roles.some((r) => r === 'OPERATOR' || r === 'RECOVERY_LEAD') &&
    !pending &&
    !working &&
    !storageError;

  async function login(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy.current) return;
    busy.current = true;
    setWorking(true);
    setError('');
    const form = event.currentTarget;
    const values = new FormData(form);
    try {
      profileSchema.parse(
        await api('/api/v1/session', {
          body: { principal: values.get('principal'), password: values.get('password') },
        }),
      );
      form.reset();
      await refresh();
    } catch (e) {
      setError(explain(e));
    } finally {
      busy.current = false;
      setWorking(false);
    }
  }
  async function logout() {
    if (busy.current) return;
    if (hasUnsavedDraft && !window.confirm('저장되지 않은 공정 편집을 버리고 로그아웃할까요?'))
      return;
    busy.current = true;
    setWorking(true);
    try {
      await api('/api/v1/session/end', { body: {} });
      generation.current++;
      request.current?.abort();
      setData(null);
      setPhase('signed-out');
      setDraftBuffers({});
      setPackageBuffers({});
      setPackageReceipt(null);
      setDraftReceipt(null);
      setBindingReceipt(null);
      setError('');
      setDialog(null);
      setRecoveryReceipt(null);
    } catch (e) {
      setError(explain(e));
    } finally {
      busy.current = false;
      setWorking(false);
    }
  }
  function canEditDraftRequest(route: string) {
    return (
      !pending &&
      !working &&
      !storageError &&
      (route === '/api/v1/package-intakes'
        ? canAuthor
        : route === '/api/v1/process-review/decisions' ||
            route === '/api/v1/device-review/decisions'
          ? !!data?.user.roles.includes('VERIFIER')
          : canReadDrafts)
    );
  }
  async function packageSubmit(
    route: Pending['route'],
    command: Record<string, unknown>,
    label: string,
  ) {
    if (!data) return;
    await submit(undefined, {
      route,
      command,
      label,
      request_key: crypto.randomUUID(),
      principal: data.user.principal,
      installation: data.installation.id,
      store_generation: data.installation.store_generation,
    });
  }
  async function submit(retry?: Pending, action?: Pending) {
    if (busy.current || !data || !cell || !fresh || storageError) return;
    if (retry && !canRecoverHostRequest(retry, data)) return;
    if (
      !retry &&
      !(action && recoveryRoute(action.route)
        ? canRecoverHost
        : action && packageRoute(action.route)
          ? fresh && canReadDrafts && canEditDraftRequest(action.route)
          : action?.route === '/api/v1/process-drafts' ||
              action?.route === '/api/v1/process-draft-bindings'
            ? canSaveDraft
            : action?.route === '/api/v1/cases/acknowledge'
              ? canAcknowledge
              : canRequest)
    )
      return;
    busy.current = true;
    setWorking(true);
    setToast('');
    const reviewed = dialog?.cell ?? cell;
    const config = reviewed.cell.value;
    const record: Pending = retry ??
      action ?? {
        request_key: crypto.randomUUID(),
        principal: data.user.principal,
        installation: data.installation.id,
        store_generation: data.installation.store_generation,
        route: dialog?.kind === 'hold' ? '/api/v1/cells/hold' : '/api/v1/runs',
        label: dialog?.kind === 'hold' ? `${config.id} 운전 보류` : `${config.id} 새 실행 준비`,
        command:
          dialog?.kind === 'hold'
            ? { cell: config.id }
            : {
                cell: config.id,
                recipe_digest: config.recipe.sha256,
                site_config_digest: config.site_config_digest,
                expected_cell: reviewed.cell.revision,
              },
      };
    let sent = false;
    try {
      // Persist before transmission. Reload means outcome unknown until the SAME key is recovered.
      savePending(sessionStorage, record);
      setPending(record);
      sent = true;
      const result = await api(record.route, { body: requestBody(record) });
      try {
        if (recoveryRoute(record.route)) {
          setRecoveryReceipt(await validateRecoveryReceipt(record, result));
        } else if (packageRoute(record.route)) {
          const value = validatePackageReceipt(record.route, record.command, result);
          setPackageReceipt({ route: record.route, value, key: record.request_key });
        } else if (record.route === '/api/v1/process-draft-bindings') {
          const value = bindingVersionSchema.parse(result);
          if (
            value.draft !== record.command.draft ||
            value.cell !== record.command.cell ||
            value.source_revision !== record.command.source_revision ||
            value.catalog_digest !== record.command.catalog_digest ||
            stableDocument(value.selections) !== stableDocument(record.command.selections)
          )
            throw new Error('binding correlation');
          setBindingReceipt(value);
          setDraftBuffers((old) => {
            const current = old[value.cell];
            if (
              current?.id === value.draft &&
              current.bindingEdit &&
              stableDocument(current.bindingEdit.selections) === stableDocument(value.selections) &&
              current.bindingEdit.catalogDigest === value.catalog_digest &&
              current.bindingEdit.sourceRevision === value.source_revision
            )
              return { ...old, [value.cell]: { ...current, bindingEdit: null } };
            return old;
          });
        } else if (record.route === '/api/v1/process-drafts') {
          const value = draftDetailSchema.parse(result);
          if (
            value.version.id !== record.command.id ||
            value.version.cell !== record.command.cell ||
            value.version.title !== record.command.title ||
            stableDocument(value.document) !== stableDocument(record.command.document)
          )
            throw new Error('draft correlation');
          setDraftReceipt(value);
          setDraftBuffers((old) => {
            const current = old[value.version.cell];
            if (
              current?.id === value.version.id &&
              current.title === value.version.title &&
              stableDocument(current.document) === stableDocument(value.document)
            )
              return { ...old, [value.version.cell]: fromDetail(value) };
            return old;
          });
        } else if (record.route === '/api/v1/runs') {
          const run = runSchema.parse(result);
          if (
            run.cell !== record.command.cell ||
            run.recipe_digest !== record.command.recipe_digest
          )
            throw new Error('run correlation');
          setRunSelections((old) => ({ ...old, [run.cell]: run.id }));
          setSelected(run.cell);
        } else if (record.route === '/api/v1/runs/start') {
          const receipt = validateStartReceipt(record, result);
          // The unchanged POST receipt has no budget/configuration fields. Read the same
          // attempt and Run before releasing the durable unknown-request record.
          const checked = validateAttemptContext(
            await api(attemptPath(receipt.cell, receipt.run, receipt.id)),
            data,
            receipt.cell,
            receipt.run,
            receipt.id,
            record,
          );
          const watch = { request: record, attempt: checked.attempt };
          try {
            saveStartWatch(sessionStorage, watch);
          } catch (error) {
            setStorageError(true);
            throw error;
          }
          setStartWatch(watch);
          setRunSelections((old) => ({ ...old, [receipt.cell]: receipt.run }));
          setSelected(receipt.cell);
        } else if (record.route === '/api/v1/cells/hold') {
          const held = z
            .object({
              configuration: z.object({ id: z.string() }),
              epoch: z.string().regex(/^[0-9]+$/),
            })
            .parse(result);
          if (held.configuration.id !== record.command.cell) throw new Error('cell correlation');
        } else if (record.route === '/api/v1/cases/acknowledge') {
          const value = caseDetailSchema.parse(result);
          if (value.snapshot.case.id !== record.command.case) throw new Error('case correlation');
        } else z.object({ revision: z.string().regex(/^[0-9]+$/) }).parse(result);
      } catch {
        throw new ApiFailure('INVALID_RESPONSE', true);
      }
      clearPending(sessionStorage);
      setPending(null);
      setDialog(null);
      setToast('요청 기록을 확인했습니다. 최신 상태를 조회했습니다.');
      await refresh();
    } catch (e) {
      if (!sent) setStorageError(true);
      // Once a result was uncertain, a later denial is NOT proof that the original never committed.
      if (!retry && e instanceof ApiFailure && !e.unknownOutcome) {
        try {
          clearPending(sessionStorage);
          setPending(null);
        } catch {
          setStorageError(true);
        }
      }
      setToast(
        `${explain(e)}${sent && (retry || !(e instanceof ApiFailure) || e.unknownOutcome) ? ' 같은 요청 번호로 기록을 확인해야 합니다.' : ''}`,
      );
      setDialog(null);
    } finally {
      busy.current = false;
      setWorking(false);
    }
  }

  async function recoveryAction(
    route: RecoveryTransientRoute,
    command: { id: string; operation?: string },
  ) {
    if (busy.current || !canRecoverHost) throw new ApiFailure('FORBIDDEN');
    const body = (
      route === '/api/v1/host-recovery/query'
        ? z.object({ id: z.uuid(), operation: z.uuid() }).strict()
        : z.object({ id: z.uuid() }).strict()
    ).parse(command);
    busy.current = true;
    setWorking(true);
    try {
      return await api(route, { body });
    } finally {
      busy.current = false;
      setWorking(false);
    }
  }

  if (phase !== 'active')
    return (
      <div className="entry">
        <section className="entry-brand">
          <div className="wordmark">RX</div>
          <p className="eyebrow">ROBOT OPERATIONS</p>
          <h1>
            현장을 연결하고,
            <br />
            운전을 신뢰할 수 있게.
          </h1>
          <div className="entry-line" />
          <p>
            장비와 공정의 실행 상태를 확인하고
            <br />
            다음 작업을 준비하는 RX 운영 공간입니다.
          </p>
          <footer>
            RX <span>ROBOT SYSTEMS · 0.1</span>
          </footer>
        </section>
        <section className="entry-form">
          <div className="pill">RX OPERATIONS</div>
          <h2>운영 공간에 로그인</h2>
          <p className="muted">현장 계정으로 접근 가능한 셀을 확인하세요.</p>
          {phase === 'checking' ? (
            <>
              <p role="status">서비스 연결을 확인하고 있습니다.</p>
              {error && (
                <p role="alert" className="error">
                  {error}
                </p>
              )}
              <button onClick={() => void refresh()}>다시 확인</button>
            </>
          ) : (
            <form onSubmit={login}>
              <label>
                계정
                <input
                  name="principal"
                  autoComplete="username"
                  required
                  maxLength={128}
                  autoFocus
                />
              </label>
              <label>
                비밀번호
                <input
                  type="password"
                  name="password"
                  autoComplete="current-password"
                  required
                  maxLength={1024}
                />
              </label>
              {error && (
                <p className="error" role="alert">
                  {error}
                </p>
              )}
              <button className="primary" disabled={working}>
                {working ? '확인 중…' : '로그인'}
                <span aria-hidden="true">→</span>
              </button>
            </form>
          )}
          <p className="entry-note">
            로그인 후 현재 셀과 단말 연결 상태를 확인합니다.
            <br />
            작업 시작은 등록 단말과 현재 운전 자격·조건을 검사합니다.
          </p>
        </section>
      </div>
    );

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="wordmark">RX</div>
        <p className="sidebar-label">OPERATIONS WORKSPACE</p>
        <nav aria-label="주 메뉴">
          {[
            '운영',
            '운전 조건',
            '실행 기록',
            '개입 사건',
            ...(canReadDrafts ? ['공정 설계', '패키지 검토'] : []),
            '구성',
            '내 접근 권한',
          ].map((item, i) => (
            <button
              key={item}
              className={tab === item ? 'selected' : ''}
              onClick={() => setTab(item)}
            >
              <span className="nav-index" aria-hidden="true">
                0{i + 1}
              </span>
              {item}
              <span className="nav-arrow" aria-hidden="true">
                ↗
              </span>
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          {fresh && <span className="signal" />}
          {fresh ? (data?.user.terminal ? '등록 단말 연결' : '서비스 연결') : '연결 확인 필요'}
          <p>운전 상태는 셀별로 확인</p>
          <div className="sidebar-rule" />
          <b>RX</b>
          <small>RX AUTOMATION / 0.1</small>
        </div>
      </aside>
      <main>
        <header className="topbar">
          <div className="breadcrumb">
            WORKSPACE <span>/</span> {tab}
          </div>
          <div className="account">
            <span className="avatar">{data?.user.principal.slice(0, 1).toUpperCase()}</span>
            <b>{data?.user.principal}</b>
            <button className="text-button" onClick={() => void logout()} disabled={working}>
              로그아웃
            </button>
          </div>
        </header>
        <div className="page">
          <div className="page-heading">
            <div>
              <p className="eyebrow">
                RX /{' '}
                {tab === '운영'
                  ? 'CELL OPERATIONS'
                  : tab === '패키지 검토'
                    ? 'PACKAGE REVIEW'
                    : tab === '공정 설계'
                      ? 'PROCESS AUTHORING'
                      : tab === '운전 조건'
                        ? 'CONDITIONS'
                        : tab === '실행 기록'
                          ? 'EXECUTION RECORDS'
                          : tab === '개입 사건'
                            ? 'INTERVENTIONS'
                            : tab === '구성'
                              ? 'CONFIGURATION'
                              : 'ACCESS'}
              </p>
              <h1>{tab === '운영' ? '셀 운영' : tab}</h1>
              <p className="muted">
                {tab === '운영'
                  ? '확인된 상태에서, 다음 작업을 준비합니다.'
                  : '현재 설치와 계정에 연결된 기록을 확인합니다.'}
              </p>
            </div>
            <div className="connection">
              <span className={`status-dot ${fresh ? 'online' : 'offline'}`} />
              <span>{fresh ? '조회 연결됨' : '최신 상태 확인 필요'}</span>
              <small>{lastRead ? `마지막 확인 ${time(lastRead)}` : '아직 확인하지 못함'}</small>
              <button className="text-button" onClick={() => void refresh()}>
                새로고침 ↻
              </button>
            </div>
          </div>
          {error && (
            <div className="notice error" role="alert">
              {error} 표시된 내용은 마지막으로 확인한 기록입니다. 새 요청은 차단됩니다.
            </div>
          )}
          {storageError && (
            <div className="notice error" role="alert">
              브라우저의 요청 기록을 읽거나 저장할 수 없습니다. 현재는 조회만 가능합니다.
            </div>
          )}
          {pending && (
            <div className="notice pending" role="alert">
              <div>
                <b>{working ? '요청을 처리하고 있습니다' : '요청 결과를 확인해야 합니다'}</b>
                <p>
                  {pending.label} · 요청 {short(pending.request_key)}
                </p>
                <small>새 요청을 만들지 않고, 전송했던 번호와 내용으로 확인합니다.</small>
                {data && !canRecoverHostRequest(pending, data) && (
                  <p>
                    현재 설치·계정 또는 연결 세대·단말 권한이 원래 요청과 다릅니다. 원래 요청은
                    보존했습니다. {pending.principal} 계정의 요청 기록 확인이 필요합니다.
                  </p>
                )}
              </div>
              <button
                onClick={() => void submit(pending)}
                disabled={!fresh || working || !data || !canRecoverHostRequest(pending, data)}
              >
                {working ? '확인 중…' : '같은 요청 확인'}
              </button>
            </div>
          )}
          {toast && (
            <div className="notice" role="status">
              {toast}
            </div>
          )}
          {tab === '내 접근 권한' ? (
            <section className="panel access-panel">
              <p className="eyebrow">CURRENT ACCOUNT</p>
              <h2>{data?.user.principal}</h2>
              <div className="role-list">
                {data?.user.roles.map((role) => (
                  <span className="pill" key={role}>
                    {text(role)}
                  </span>
                ))}
              </div>
              <h3>접근 가능한 셀</h3>
              <ul>
                {data?.user.cells.map((c) => (
                  <li key={c}>{c}</li>
                ))}
              </ul>
              <div className="inset">
                <b>등록 단말 인증</b>
                <p>
                  {data?.user.terminal
                    ? `${data.user.terminal}에서 인증된 세션입니다. 작업 시작에는 현재 운전 조건 확인도 필요합니다.`
                    : '현재 세션에는 등록 단말 인증이 없습니다. 로그인만으로 실장비 운전을 시작할 수 없습니다.'}
                </p>
              </div>
            </section>
          ) : !data?.cells.length ? (
            <section className="empty panel">
              <div className="empty-symbol">＋</div>
              <h2>아직 등록된 셀이 없습니다</h2>
              <p>엔지니어가 셀 구성을 등록하면 장비, 공정과 실행 기록이 여기에 표시됩니다.</p>
              <small>구성 등록과 실제 운전 자격은 별도로 확인합니다.</small>
            </section>
          ) : (
            <>
              <div className="cell-tabs" role="tablist" aria-label="셀 선택">
                {data.cells.map((c) => (
                  <button
                    role="tab"
                    aria-selected={c.cell.value.id === selected}
                    key={c.cell.value.id}
                    onClick={() => setSelected(c.cell.value.id)}
                  >
                    {c.cell.value.id}
                    <span className="tiny-dot" />
                  </button>
                ))}
              </div>
              {cell && canReadDrafts && (
                <Packages
                  key={JSON.stringify([
                    data.user.principal,
                    data.installation.id,
                    data.installation.store_generation,
                    cell.cell.value.id,
                  ])}
                  cell={cell.cell.value.id}
                  principal={data.user.principal}
                  roles={data.user.roles}
                  active={tab === '패키지 검토'}
                  canWrite={fresh && !pending && !working && !storageError}
                  buffer={packageBuffers[cell.cell.value.id] ?? emptyPackageBuffer()}
                  onBuffer={(v) =>
                    setPackageBuffers((old) => ({ ...old, [cell.cell.value.id]: v }))
                  }
                  receipt={packageReceipt}
                  onSubmit={packageSubmit}
                />
              )}
              {cell && tab === '패키지 검토' && !canReadDrafts && (
                <section className="panel empty">
                  <h2>검토 기록 접근 권한이 필요합니다</h2>
                  <p>구성 담당자 또는 검토자 계정으로 확인할 수 있습니다.</p>
                </section>
              )}
              {cell &&
                tab !== '패키지 검토' &&
                (tab === '운영' ? (
                  <>
                    <div className="cell-hero">
                      <div>
                        <div className="hero-top">
                          <span className="pill">
                            {cell.cell.value.environment === 'SIMULATION'
                              ? '모의 환경'
                              : '실장비 구성'}
                          </span>
                          <span className="mono">
                            CELL /{' '}
                            {String(data.cells.findIndex((c) => c === cell) + 1).padStart(2, '0')}
                          </span>
                        </div>
                        <h2>{cell.cell.value.id}</h2>
                        <p>
                          {cell.cell.value.commissioning === 'REVALIDATION_REQUIRED'
                            ? '이전 자격 기록을 그대로 사용할 수 없습니다. 개입·변경 사항을 재검증해야 합니다.'
                            : cell.cell.value.commissioning === 'COMMISSIONED'
                              ? '운전 자격 기록이 있습니다. 시작 시 현재 조건을 다시 확인합니다.'
                              : cell.cell.value.commissioning === 'NOT_COMMISSIONED'
                                ? '운전 자격이 아직 등록되지 않았습니다. 구성을 확인하고 검증을 준비하세요.'
                                : '운전 상태 기록이 없습니다. 현재 상태를 다시 확인해야 합니다.'}
                        </p>
                      </div>
                      <div className="hero-status">
                        <span className="status-dot caution" />
                        <strong>
                          {cell.cell.value.blocks.length
                            ? '운전 보류'
                            : cell.cell.value.qualification
                              ? '자격 기록 있음'
                              : '검증 대기'}
                        </strong>
                        <small>
                          {cell.cell.value.blocks.length
                            ? `${cell.cell.value.blocks.length}개 차단 사유 확인 필요`
                            : '현재 운전 가능 판정을 의미하지 않습니다'}
                        </small>
                      </div>
                    </div>
                    <div className="operations-grid">
                      <section className="panel">
                        <div className="section-heading">
                          <div>
                            <p className="eyebrow">NEXT ACTION</p>
                            <h3>실행 준비</h3>
                          </div>
                          <span className="section-no">01</span>
                        </div>
                        <p className="body-copy">
                          등록된 공정과 현장 구성을 연결해 새 실행 기록을 만듭니다. 장비 동작은
                          시작하지 않습니다.
                        </p>
                        <dl className="facts">
                          <div>
                            <dt>장비 연결 구성</dt>
                            <dd>{cell.cell.value.hosts.length}개 항목</dd>
                          </div>
                          <div>
                            <dt>셀 기록 버전</dt>
                            <dd>r{cell.cell.revision}</dd>
                          </div>
                          <div>
                            <dt>RX 운영 모드</dt>
                            <dd>{text(cell.cell.value.mode ?? 'UNKNOWN')}</dd>
                          </div>
                          <div>
                            <dt>운전 자격</dt>
                            <dd>{text(cell.cell.value.commissioning ?? 'UNKNOWN')}</dd>
                          </div>
                        </dl>
                        <button
                          className="primary wide"
                          disabled={!canRequest}
                          onClick={() => setDialog({ kind: 'run', cell })}
                        >
                          새 실행 준비 <span aria-hidden="true">＋</span>
                        </button>
                        {!canOperate && (
                          <small className="muted">
                            운영 권한이 있는 계정에서 요청할 수 있습니다.
                          </small>
                        )}
                      </section>
                      <section className="panel">
                        <div className="section-heading">
                          <div>
                            <p className="eyebrow">ATTENTION</p>
                            <h3>확인이 필요한 상태</h3>
                          </div>
                          <span className="section-no">02</span>
                        </div>
                        {!cell.cell.value.qualification && (
                          <div className="check-line">
                            <span className="check-mark">!</span>
                            <div>
                              <b>현장 검증·운전 자격 미등록</b>
                              <p>구성 등록이 운전 허가를 대신하지 않습니다.</p>
                            </div>
                          </div>
                        )}
                        {cell.cell.value.blocks.map((b) => (
                          <div className="check-line" key={b.id}>
                            <span className="check-mark">!</span>
                            <div>
                              <b>{text(b.reason)}</b>
                              <p>
                                {b.latched
                                  ? '원인이 해소되어도 자동 재시작하지 않습니다.'
                                  : '현재 조건을 다시 확인하세요.'}
                              </p>
                            </div>
                          </div>
                        ))}
                        <div className="check-line">
                          <span className="check-mark neutral">—</span>
                          <div>
                            <b>{data.user.terminal ? '등록 단말 인증됨' : '등록 단말 인증 없음'}</b>
                            <p>
                              {data.user.terminal
                                ? `${data.user.terminal} · 실행을 선택해 현재 시작 조건을 조회하세요.`
                                : '작업 시작 요청에는 등록 단말 인증이 필요합니다.'}
                            </p>
                          </div>
                        </div>
                        <button
                          className="outline wide"
                          disabled={!canRequest}
                          onClick={() => setDialog({ kind: 'hold', cell })}
                        >
                          운전 보류 요청
                        </button>
                        <small className="muted">
                          소프트웨어 권한 철회 요청입니다. 물리적 정지 확인은 별도입니다.
                        </small>
                      </section>
                    </div>
                    <button className="conditions-link" onClick={() => setTab('운전 조건')}>
                      운전 조건과 관측 근거 확인 <span aria-hidden="true">↗</span>
                    </button>
                    <Runs cell={cell} />
                  </>
                ) : tab === '운전 조건' ? (
                  <Conditions cell={cell} requestStarted={readStarted} queryFresh={fresh} />
                ) : tab === '공정 설계' && canReadDrafts ? (
                  <ProcessEditor
                    key={cell.cell.value.id}
                    cell={cell.cell.value.id}
                    buffer={draftBuffers[cell.cell.value.id] ?? null}
                    onBuffer={(value) =>
                      setDraftBuffers((old) => ({ ...old, [cell.cell.value.id]: value }))
                    }
                    canEdit={canEditDraft}
                    canSave={canSaveDraft}
                    receipt={draftReceipt}
                    bindingReceipt={bindingReceipt}
                    onSaveBindings={async (edit) => {
                      const buffer = draftBuffers[cell.cell.value.id];
                      if (!buffer) return;
                      return submit(undefined, {
                        request_key: crypto.randomUUID(),
                        principal: data.user.principal,
                        installation: data.installation.id,
                        store_generation: data.installation.store_generation,
                        route: '/api/v1/process-draft-bindings',
                        label: `${buffer.title} 바인딩 저장`,
                        command: {
                          draft: buffer.id,
                          cell: buffer.cell,
                          source_revision: edit.sourceRevision,
                          expected: edit.expected,
                          catalog_digest: edit.catalogDigest,
                          selections: edit.selections,
                          device_plans: edit.devicePlans ?? [],
                        },
                      });
                    }}
                    onSave={async (buffer) =>
                      submit(undefined, {
                        request_key: crypto.randomUUID(),
                        principal: data.user.principal,
                        installation: data.installation.id,
                        store_generation: data.installation.store_generation,
                        route: '/api/v1/process-drafts',
                        label: `${buffer.title} 초안 저장`,
                        command: {
                          id: buffer.id,
                          cell: buffer.cell,
                          expected: buffer.expected,
                          title: buffer.title,
                          document: buffer.document,
                        },
                      })
                    }
                  />
                ) : tab === '개입 사건' ? (
                  <Cases
                    cell={cell.cell.value.id}
                    refreshToken={data.snapshot_id}
                    canAcknowledge={canAcknowledge}
                    onAcknowledge={(item) =>
                      void submit(undefined, {
                        request_key: crypto.randomUUID(),
                        principal: data.user.principal,
                        installation: data.installation.id,
                        store_generation: data.installation.store_generation,
                        route: '/api/v1/cases/acknowledge',
                        label: `개입 사건 ${short(item.case.id)} 알림 확인`,
                        command: {
                          cell: cell.cell.value.id,
                          case: item.case.id,
                          expected_case: item.revision,
                          occurred_at: new Date().toISOString(),
                        },
                      })
                    }
                  />
                ) : tab === '실행 기록' ? (
                  <>
                    <Runs cell={cell} />
                    <Work cell={cell} />
                  </>
                ) : (
                  <Configuration cell={cell} />
                ))}
              {cell && tab === '구성' && data.user.roles.includes('RELEASE_MANAGER') && (
                <HostRecovery
                  key={JSON.stringify([
                    data.user.principal,
                    data.installation.id,
                    data.installation.store_generation,
                    data.installation.runtime_boot,
                    cell.cell.value.id,
                  ])}
                  data={data}
                  cell={cell}
                  fresh={fresh}
                  canWrite={canRecoverHost}
                  working={working}
                  pending={pending}
                  receipt={recoveryReceipt}
                  onSubmit={(record) => submit(undefined, record)}
                  onAction={recoveryAction}
                  onSelectCell={setSelected}
                />
              )}
              {cell && (tab === '운영' || tab === '실행 기록') && (
                <RunStart
                  key={JSON.stringify([
                    data.user.principal,
                    data.installation.id,
                    data.installation.store_generation,
                    cell.cell.value.id,
                  ])}
                  data={data}
                  cell={cell}
                  selectedRun={
                    runSelections[cell.cell.value.id] ??
                    (recoverableStart?.start_review?.cell === cell.cell.value.id
                      ? String(recoverableStart.command.run)
                      : undefined) ??
                    (startWatch &&
                    canRecover(
                      startWatch.request,
                      data.user.principal,
                      data.installation.id,
                      data.installation.store_generation,
                    ) &&
                    startWatch.attempt.cell === cell.cell.value.id
                      ? startWatch.attempt.run
                      : '')
                  }
                  onSelectRun={(run) =>
                    setRunSelections((old) => ({ ...old, [cell.cell.value.id]: run }))
                  }
                  canRequest={canRequest}
                  fresh={fresh}
                  working={working}
                  watch={startWatch}
                  onSubmit={(record) => submit(undefined, record)}
                />
              )}
            </>
          )}
          <footer className="page-footer">
            <span>RX · 확인된 사실을 기준으로</span>
            <span>설치 {short(data?.installation.id ?? '')} / 최근 조회 결과 표시</span>
          </footer>
        </div>
      </main>
      <dialog
        ref={dialogRef}
        onCancel={(event) => {
          if (working) event.preventDefault();
          else setDialog(null);
        }}
        aria-labelledby="action-title"
      >
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          <p className="eyebrow">REVIEW REQUEST</p>
          <h2 id="action-title">
            {dialog?.kind === 'hold' ? '운전을 보류할까요?' : '새 실행을 준비할까요?'}
          </h2>
          <p>
            <b>{dialog?.cell.cell.value.id}</b> · 셀 기록 r{dialog?.cell.cell.revision}
          </p>
          <p>
            {dialog?.kind === 'hold'
              ? '관련 운전 권한을 철회하고 차단 사유를 기록합니다. 실제 장비 정지는 이 응답만으로 확인되지 않습니다.'
              : '현재 등록된 공정과 구성으로 실행 기록을 생성합니다. 장비를 움직이거나 운전 자격을 부여하지 않습니다.'}
          </p>
          <div className="dialog-actions">
            <button type="button" onClick={() => setDialog(null)} disabled={working}>
              돌아가기
            </button>
            <button type="submit" className="primary" disabled={!canRequest}>
              {working ? '요청 중…' : dialog?.kind === 'hold' ? '보류 요청' : '실행 기록 만들기'}
            </button>
          </div>
        </form>
      </dialog>
    </div>
  );
}
