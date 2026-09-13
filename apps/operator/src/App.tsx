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
import { count, text, short, time } from './labels';
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
  const [tab, setTab] = useState('Operations');
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
      tab === 'Operating conditions' ? 500 : 3000,
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
    if (hasUnsavedDraft && !window.confirm('Discard unsaved workflow edits and sign out?')) return;
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
        label:
          dialog?.kind === 'hold' ? `${config.id} Operation hold` : `${config.id} Prepare new run`,
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
      setToast('The request record was verified and the latest state was retrieved.');
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
        `${explain(e)}${sent && (retry || !(e instanceof ApiFailure) || e.unknownOutcome) ? ' Verify the record using the same request key.' : ''}`,
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
            Connect your site.
            <br />
            Operate with confidence.
          </h1>
          <div className="entry-line" />
          <p>
            View device and workflow execution states
            <br />
            and prepare the next task in the RX operations workspace.
          </p>
          <footer>
            RX <span>ROBOT SYSTEMS · 0.1</span>
          </footer>
        </section>
        <section className="entry-form">
          <div className="pill">RX OPERATIONS</div>
          <h2>Sign in to the operations workspace</h2>
          <p className="muted">Use your site account to view accessible cells.</p>
          {phase === 'checking' ? (
            <>
              <p role="status">Checking the service connection.</p>
              {error && (
                <p role="alert" className="error">
                  {error}
                </p>
              )}
              <button onClick={() => void refresh()}>Check again</button>
            </>
          ) : (
            <form onSubmit={login}>
              <label>
                Account
                <input
                  name="principal"
                  autoComplete="username"
                  required
                  maxLength={128}
                  autoFocus
                />
              </label>
              <label>
                Password
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
                {working ? 'Checking…' : 'Sign in'}
                <span aria-hidden="true">→</span>
              </button>
            </form>
          )}
          <p className="entry-note">
            After signing in, check the current cell and terminal connection.
            <br />
            Starting a task checks the registered terminal and current operating qualification and
            conditions.
          </p>
        </section>
      </div>
    );

  return (
    <div className="shell">
      <aside className="sidebar">
        <div className="wordmark">RX</div>
        <p className="sidebar-label">OPERATIONS WORKSPACE</p>
        <nav aria-label="Main menu">
          {[
            'Operations',
            'Operating conditions',
            'Run records',
            'Intervention cases',
            ...(canReadDrafts ? ['Workflow design', 'Package review'] : []),
            'Configuration',
            'My access',
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
          {fresh
            ? data?.user.terminal
              ? 'Registered terminal connected'
              : 'Service connected'
            : 'Connection needs verification'}
          <p>Check operating state for each cell</p>
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
              Sign out
            </button>
          </div>
        </header>
        <div className="page">
          <div className="page-heading">
            <div>
              <p className="eyebrow">
                RX /{' '}
                {tab === 'Operations'
                  ? 'CELL OPERATIONS'
                  : tab === 'Package review'
                    ? 'PACKAGE REVIEW'
                    : tab === 'Workflow design'
                      ? 'PROCESS AUTHORING'
                      : tab === 'Operating conditions'
                        ? 'CONDITIONS'
                        : tab === 'Run records'
                          ? 'EXECUTION RECORDS'
                          : tab === 'Intervention cases'
                            ? 'INTERVENTIONS'
                            : tab === 'Configuration'
                              ? 'CONFIGURATION'
                              : 'ACCESS'}
              </p>
              <h1>{tab === 'Operations' ? 'Cell operations' : tab}</h1>
              <p className="muted">
                {tab === 'Operations'
                  ? 'Prepare the next task using verified state.'
                  : 'View records associated with the current installation and account.'}
              </p>
            </div>
            <div className="connection">
              <span className={`status-dot ${fresh ? 'online' : 'offline'}`} />
              <span>{fresh ? 'Read connection active' : 'Latest state needs verification'}</span>
              <small>{lastRead ? `Last checked ${time(lastRead)}` : 'Not checked yet'}</small>
              <button className="text-button" onClick={() => void refresh()}>
                Refresh ↻
              </button>
            </div>
          </div>
          {error && (
            <div className="notice error" role="alert">
              {error} These are the last verified records. New requests are blocked.
            </div>
          )}
          {storageError && (
            <div className="notice error" role="alert">
              Cannot read or save request records in this browser. Only read access is currently
              available.
            </div>
          )}
          {pending && (
            <div className="notice pending" role="alert">
              <div>
                <b>
                  {working ? 'Processing the request' : 'The request outcome needs verification'}
                </b>
                <p>
                  {pending.label} · request {short(pending.request_key)}
                </p>
                <small>
                  Verify using the original request key and content without creating a new request.
                </small>
                {data && !canRecoverHostRequest(pending, data) && (
                  <p>
                    The current installation, account, connection generation, or terminal authority
                    differs from the original request. The original request has been preserved.{' '}
                    {pending.principal} account request records need verification.
                  </p>
                )}
              </div>
              <button
                onClick={() => void submit(pending)}
                disabled={!fresh || working || !data || !canRecoverHostRequest(pending, data)}
              >
                {working ? 'Checking…' : 'Check original request'}
              </button>
            </div>
          )}
          {toast && (
            <div className="notice" role="status">
              {toast}
            </div>
          )}
          {tab === 'My access' ? (
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
              <h3>Accessible cells</h3>
              <ul>
                {data?.user.cells.map((c) => (
                  <li key={c}>{c}</li>
                ))}
              </ul>
              <div className="inset">
                <b>Registered terminal authentication</b>
                <p>
                  {data?.user.terminal
                    ? `${data.user.terminal} authenticated this session. Starting a task also requires checking current operating conditions.`
                    : 'This session has no registered terminal authentication. Signing in alone cannot start physical equipment.'}
                </p>
              </div>
            </section>
          ) : !data?.cells.length ? (
            <section className="empty panel">
              <div className="empty-symbol">＋</div>
              <h2>No cells have been registered yet</h2>
              <p>
                Devices, workflows, and run records appear here after an engineer registers the cell
                configuration.
              </p>
              <small>
                Configuration registration and physical operating qualification are checked
                separately.
              </small>
            </section>
          ) : (
            <>
              <div className="cell-tabs" role="tablist" aria-label="Select cell">
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
                  active={tab === 'Package review'}
                  canWrite={fresh && !pending && !working && !storageError}
                  buffer={packageBuffers[cell.cell.value.id] ?? emptyPackageBuffer()}
                  onBuffer={(v) =>
                    setPackageBuffers((old) => ({ ...old, [cell.cell.value.id]: v }))
                  }
                  receipt={packageReceipt}
                  onSubmit={packageSubmit}
                />
              )}
              {cell && tab === 'Package review' && !canReadDrafts && (
                <section className="panel empty">
                  <h2>Access to review records is required</h2>
                  <p>Use an engineer or verifier account to view these records.</p>
                </section>
              )}
              {cell &&
                tab !== 'Package review' &&
                (tab === 'Operations' ? (
                  <>
                    <div className="cell-hero">
                      <div>
                        <div className="hero-top">
                          <span className="pill">
                            {cell.cell.value.environment === 'SIMULATION'
                              ? 'Simulation'
                              : 'Physical device configuration'}
                          </span>
                          <span className="mono">
                            CELL /{' '}
                            {String(data.cells.findIndex((c) => c === cell) + 1).padStart(2, '0')}
                          </span>
                        </div>
                        <h2>{cell.cell.value.id}</h2>
                        <p>
                          {cell.cell.value.commissioning === 'REVALIDATION_REQUIRED'
                            ? 'The previous qualification record cannot be reused. Revalidate interventions and changes.'
                            : cell.cell.value.commissioning === 'COMMISSIONED'
                              ? 'Operating qualification is recorded. Current conditions are checked again at start.'
                              : cell.cell.value.commissioning === 'NOT_COMMISSIONED'
                                ? 'Operating qualification has not been registered. Check the configuration and prepare for verification.'
                                : 'No operating state is recorded. Check the current state again.'}
                        </p>
                      </div>
                      <div className="hero-status">
                        <span className="status-dot caution" />
                        <strong>
                          {cell.cell.value.blocks.length
                            ? 'Operation hold'
                            : cell.cell.value.qualification
                              ? 'Qualification recorded'
                              : 'Awaiting verification'}
                        </strong>
                        <small>
                          {cell.cell.value.blocks.length
                            ? `${cell.cell.value.blocks.length} blocking reasons need review`
                            : 'This does not establish current operating readiness'}
                        </small>
                      </div>
                    </div>
                    <div className="operations-grid">
                      <section className="panel">
                        <div className="section-heading">
                          <div>
                            <p className="eyebrow">NEXT ACTION</p>
                            <h3>Run preparation</h3>
                          </div>
                          <span className="section-no">01</span>
                        </div>
                        <p className="body-copy">
                          Create a new run record from the registered workflow and site
                          configuration. This does not start device motion.
                        </p>
                        <dl className="facts">
                          <div>
                            <dt>Device connection configuration</dt>
                            <dd>{count(cell.cell.value.hosts.length, 'item')}</dd>
                          </div>
                          <div>
                            <dt>Cell record revision</dt>
                            <dd>r{cell.cell.revision}</dd>
                          </div>
                          <div>
                            <dt>RX operating mode</dt>
                            <dd>{text(cell.cell.value.mode ?? 'UNKNOWN')}</dd>
                          </div>
                          <div>
                            <dt>Operating qualification</dt>
                            <dd>{text(cell.cell.value.commissioning ?? 'UNKNOWN')}</dd>
                          </div>
                        </dl>
                        <button
                          className="primary wide"
                          disabled={!canRequest}
                          onClick={() => setDialog({ kind: 'run', cell })}
                        >
                          Prepare new run <span aria-hidden="true">＋</span>
                        </button>
                        {!canOperate && (
                          <small className="muted">
                            An account with operator authority can submit this request.
                          </small>
                        )}
                      </section>
                      <section className="panel">
                        <div className="section-heading">
                          <div>
                            <p className="eyebrow">ATTENTION</p>
                            <h3>States requiring attention</h3>
                          </div>
                          <span className="section-no">02</span>
                        </div>
                        {!cell.cell.value.qualification && (
                          <div className="check-line">
                            <span className="check-mark">!</span>
                            <div>
                              <b>Site verification and operating qualification not registered</b>
                              <p>Configuration registration does not grant operating permission.</p>
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
                                  ? 'Resolving the cause does not restart operation automatically.'
                                  : 'Check the current conditions again.'}
                              </p>
                            </div>
                          </div>
                        ))}
                        <div className="check-line">
                          <span className="check-mark neutral">—</span>
                          <div>
                            <b>
                              {data.user.terminal
                                ? 'Registered terminal authenticated'
                                : 'No registered terminal authentication'}
                            </b>
                            <p>
                              {data.user.terminal
                                ? `${data.user.terminal} · Select a run to query its current start conditions.`
                                : 'Task start requests require registered terminal authentication.'}
                            </p>
                          </div>
                        </div>
                        <button
                          className="outline wide"
                          disabled={!canRequest}
                          onClick={() => setDialog({ kind: 'hold', cell })}
                        >
                          Request operation hold
                        </button>
                        <small className="muted">
                          This requests revocation of software authority. Physical stop confirmation
                          is separate.
                        </small>
                      </section>
                    </div>
                    <button
                      className="conditions-link"
                      onClick={() => setTab('Operating conditions')}
                    >
                      View operating conditions and observation evidence{' '}
                      <span aria-hidden="true">↗</span>
                    </button>
                    <Runs cell={cell} />
                  </>
                ) : tab === 'Operating conditions' ? (
                  <Conditions cell={cell} requestStarted={readStarted} queryFresh={fresh} />
                ) : tab === 'Workflow design' && canReadDrafts ? (
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
                        label: `${buffer.title} Save bindings`,
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
                        label: `${buffer.title} Save draft`,
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
                ) : tab === 'Intervention cases' ? (
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
                        label: `Intervention cases ${short(item.case.id)} Acknowledge notification`,
                        command: {
                          cell: cell.cell.value.id,
                          case: item.case.id,
                          expected_case: item.revision,
                          occurred_at: new Date().toISOString(),
                        },
                      })
                    }
                  />
                ) : tab === 'Run records' ? (
                  <>
                    <Runs cell={cell} />
                    <Work cell={cell} />
                  </>
                ) : (
                  <Configuration cell={cell} />
                ))}
              {cell && tab === 'Configuration' && data.user.roles.includes('RELEASE_MANAGER') && (
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
              {cell && (tab === 'Operations' || tab === 'Run records') && (
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
            <span>RX · Grounded in verified facts</span>
            <span>
              Installation {short(data?.installation.id ?? '')} / displaying the latest retrieved
              records
            </span>
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
            {dialog?.kind === 'hold' ? 'Place operation on hold?' : 'Prepare a new run?'}
          </h2>
          <p>
            <b>{dialog?.cell.cell.value.id}</b> · cell record r{dialog?.cell.cell.revision}
          </p>
          <p>
            {dialog?.kind === 'hold'
              ? 'Revoke the related operating authority and record a blocking reason. This response alone does not confirm a physical device stop.'
              : 'Create a run record using the currently registered workflow and configuration. This does not move devices or grant operating qualification.'}
          </p>
          <div className="dialog-actions">
            <button type="button" onClick={() => setDialog(null)} disabled={working}>
              Back
            </button>
            <button type="submit" className="primary" disabled={!canRequest}>
              {working
                ? 'Requesting…'
                : dialog?.kind === 'hold'
                  ? 'Request hold'
                  : 'Create run record'}
            </button>
          </div>
        </form>
      </dialog>
    </div>
  );
}
