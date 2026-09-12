import { describe, expect, it } from 'vitest';
import { api, ApiFailure } from './api';
import { canRecover, readPending, requestBody, savePending } from './pending';
import { pendingSchema, startCommandSchema, type CellOverview, type Overview } from './schema';
import {
  contextIsCurrent,
  freezeStartRequest,
  positiveQuantity,
  readStartWatch,
  saveStartWatch,
  startAttemptSchema,
  startContextSchema,
  startStatusDisplay,
  validateAttemptContext,
  validateStartContext,
  validateStartReceipt,
  type AttemptContext,
  type StartContext,
} from './run-start-schema';

const uuid = (digit: string) =>
  `${digit.repeat(8)}-${digit.repeat(4)}-4${digit.repeat(3)}-8${digit.repeat(3)}-${digit.repeat(12)}`;
const artifact = (digit: string) => ({
  sha256: digit.repeat(64),
  schema_id: 'rx.test.v1',
  size_bytes: '1',
});
const installation = {
  id: uuid('1'),
  store_generation: uuid('2'),
  runtime_boot: uuid('3'),
  clock_id: 'clock/a',
};
const data = { installation, user: { principal: 'operator/a' } } as Overview;
const candidate: StartContext = {
  installation,
  checked_at: { clock_id: 'clock/a', ticks_ns: '100' },
  cell: 'cell/a',
  cell_revision: '9007199254740993',
  epoch: '3',
  scope_epochs: { 'zone/a': '2' },
  configuration_digest: 'd'.repeat(64),
  environment: 'SIMULATION',
  commissioning: 'COMMISSIONED',
  envelope: artifact('a'),
  recipe: artifact('b'),
  site_config_digest: 'c'.repeat(64),
  maximum_budget: '10',
  run_revision: '9007199254740994',
  run: {
    id: uuid('4'),
    cell: 'cell/a',
    state: 'PREPARED',
    purpose: null,
    budget: null,
    envelope_digest: 'a'.repeat(64),
    recipe_digest: 'b'.repeat(64),
    part_ids: [],
    pending_attempt: null,
  },
  request: {
    run: uuid('4'),
    envelope_digest: 'a'.repeat(64),
    purpose: 'PRODUCTION',
    budget_unit: 'PART_ATTEMPT',
    budget_limit: '2',
    expected_cell: '9007199254740993',
    expected_run: '9007199254740994',
  },
  can_request: true,
  blocking_reason: null,
};
const receipt = {
  id: uuid('5'),
  run: uuid('4'),
  cell: 'cell/a',
  expected_cell_revision: candidate.cell_revision,
  expected_run_revision: '9007199254740995',
  epoch: '3',
  scopes: { 'zone/a': '2' },
  host_boots: { 'host/a': uuid('6'), 'host/b': uuid('7') },
  acknowledgments: { 'host/a': uuid('6') },
  executor_session: uuid('9'),
  actor: 'operator/a',
  status: 'ARMING' as const,
  mandate: null,
  valid_until: { clock_id: 'clock/a', ticks_ns: '200' },
  terminal: null,
};
const record = freezeStartRequest(candidate, data, uuid('a'));
const attemptView: AttemptContext = {
  installation,
  checked_at: { clock_id: 'clock/a', ticks_ns: '150' },
  run_revision: receipt.expected_run_revision,
  run: {
    ...candidate.run,
    pending_attempt: receipt.id,
    purpose: 'PRODUCTION',
    budget: { unit: 'PART_ATTEMPT', limit: '2', revision: '1', consumptions: [] },
  },
  attempt: receipt,
  deadline_status: 'WITHIN_DEADLINE',
};
const cell = {
  cell: {
    revision: candidate.cell_revision,
    value: {
      id: candidate.cell,
      envelope: candidate.envelope,
      recipe: candidate.recipe,
      site_config_digest: candidate.site_config_digest,
    },
  },
  runs: [{ revision: candidate.run_revision, value: candidate.run }],
} as CellOverview;
function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

describe('operator start read boundaries', () => {
  it('requires an explicitly positive canonical quantity and preserves large revisions', () => {
    for (const value of ['', '0', '-1', '1.5', '02', ' 2', 'text', '18446744073709551616'])
      expect(positiveQuantity(value)).toBe(false);
    expect(positiveQuantity('2')).toBe(true);
    expect(startCommandSchema.safeParse({ ...candidate.request, budget_limit: 2 }).success).toBe(
      false,
    );
    expect(
      validateStartContext(candidate, data, 'cell/a', uuid('4'), '2').request.expected_run,
    ).toBe('9007199254740994');
  });
  it('rejects unknown status, invented ready flags and mismatched candidate identity', () => {
    expect(startContextSchema.safeParse({ ready: true }).success).toBe(false);
    expect(startAttemptSchema.safeParse({ ...receipt, status: 'READY' }).success).toBe(false);
    expect(
      startContextSchema.safeParse({ ...candidate, blocking_reason: 'HOST_NOT_PREPARED' }).success,
    ).toBe(false);
    const invalid = [
      { ...candidate, cell: 'cell/other' },
      { ...candidate, run: { ...candidate.run, id: uuid('b') } },
      { ...candidate, request: { ...candidate.request, budget_limit: '3' } },
      { ...candidate, request: { ...candidate.request, expected_run: '1' } },
      { ...candidate, installation: { ...installation, store_generation: uuid('b') } },
      { ...candidate, installation: { ...installation, runtime_boot: uuid('b') } },
    ];
    for (const value of invalid)
      expect(() => validateStartContext(value, data, 'cell/a', uuid('4'), '2')).toThrow();
    const blocked = { ...candidate, can_request: false, blocking_reason: 'NOT_COMMISSIONED' };
    expect(validateStartContext(blocked, data, 'cell/a', uuid('4'), '2').can_request).toBe(false);
  });
  it('freezes exact reviewed values and requires a new review after revision, run or runtime changes', () => {
    const frozen = freezeStartRequest(candidate, data, uuid('a'));
    const original = JSON.stringify(requestBody(frozen));
    expect(contextIsCurrent(candidate, data, cell, uuid('4'), '2')).toBe(true);
    expect(
      contextIsCurrent(
        candidate,
        data,
        { ...cell, cell: { ...cell.cell, revision: '9007199254740994' } },
        uuid('4'),
        '2',
      ),
    ).toBe(false);
    expect(
      contextIsCurrent(
        candidate,
        data,
        { ...cell, runs: [{ ...cell.runs[0], revision: '9007199254740995' }] },
        uuid('4'),
        '2',
      ),
    ).toBe(false);
    expect(contextIsCurrent(candidate, data, cell, uuid('b'), '2')).toBe(false);
    expect(contextIsCurrent(candidate, data, cell, uuid('4'), '3')).toBe(false);
    expect(
      contextIsCurrent(
        candidate,
        { ...data, installation: { ...installation, runtime_boot: uuid('b') } },
        cell,
        uuid('4'),
        '2',
      ),
    ).toBe(false);
    const changed = structuredClone(candidate);
    const independent = freezeStartRequest(changed, data, uuid('a'));
    changed.request.budget_limit = '9';
    changed.scope_epochs['zone/a'] = '99';
    expect(independent.command.budget_limit).toBe('2');
    expect(independent.start_review?.scope_epochs['zone/a']).toBe('2');
    expect(JSON.stringify(requestBody(frozen))).toBe(original);
  });
});

describe('start request receipt and reload recovery', () => {
  it('rejects missing or different Host boots in STARTED receipts without releasing pending', () => {
    const saved = storage();
    savePending(saved, record);
    const started = {
      ...receipt,
      status: 'STARTED',
      mandate: uuid('b'),
      acknowledgments: { ...receipt.host_boots },
    };
    expect(validateStartReceipt(record, started).status).toBe('STARTED');
    for (const acknowledgments of [
      {},
      { 'host/a': uuid('6') },
      { 'host/a': uuid('8'), 'host/b': uuid('7') },
      { 'host/a': uuid('6'), 'host/b': uuid('7'), 'host/other': uuid('8') },
    ]) {
      expect(() => validateStartReceipt(record, { ...started, acknowledgments })).toThrow();
      expect(() =>
        validateAttemptContext(
          { ...attemptView, attempt: { ...started, acknowledgments } },
          data,
          'cell/a',
          uuid('4'),
          receipt.id,
          record,
        ),
      ).toThrow();
      expect(requestBody(readPending(saved)!)).toEqual(requestBody(record));
    }
    for (const status of ['PENDING', 'ARMING', 'REJECTED'] as const) {
      expect(() =>
        validateStartReceipt(record, {
          ...receipt,
          status,
          acknowledgments: { 'host/a': uuid('8') },
        }),
      ).toThrow();
      expect(validateStartReceipt(record, { ...receipt, status }).status).toBe(status);
    }
    expect(
      validateStartReceipt(record, { ...receipt, acknowledgments: { ...receipt.host_boots } })
        .status,
    ).toBe('ARMING');
  });
  it('keeps the same key and exact body over reply loss and reload; other identities cannot recover', async () => {
    const saved = storage();
    savePending(saved, record);
    const originalFetch = globalThis.fetch;
    let transmitted = '';
    globalThis.fetch = async (_path, options) => {
      transmitted = String(options?.body);
      throw new Error('reply lost after server commit');
    };
    try {
      await expect(api(record.route, { body: requestBody(record) })).rejects.toMatchObject({
        unknownOutcome: true,
      });
    } finally {
      globalThis.fetch = originalFetch;
    }
    const recovered = readPending(saved)!;
    expect(JSON.stringify(requestBody(recovered))).toBe(transmitted);
    expect(recovered.start_review).toEqual(record.start_review);
    expect(
      canRecover(recovered, 'operator/other', installation.id, installation.store_generation),
    ).toBe(false);
    expect(
      canRecover(recovered, data.user.principal, uuid('b'), installation.store_generation),
    ).toBe(false);
    expect(canRecover(recovered, data.user.principal, installation.id, uuid('b'))).toBe(false);
    expect(pendingSchema.safeParse({ ...record, start_review: undefined }).success).toBe(false);
    expect(
      pendingSchema.safeParse({ ...record, command: { ...record.command, budget_limit: 2 } })
        .success,
    ).toBe(false);
  });
  it('rejects unrelated receipts and keeps the original pending record when validation fails', () => {
    const saved = storage();
    savePending(saved, record);
    for (const value of [
      { ...receipt, cell: 'cell/b' },
      { ...receipt, run: uuid('b') },
      { ...receipt, actor: 'operator/b' },
      { ...receipt, expected_run_revision: candidate.run_revision },
      { ...receipt, expected_cell_revision: '1' },
      { ...receipt, epoch: '4' },
      { ...receipt, scopes: { 'zone/a': '3' } },
    ])
      expect(() => validateStartReceipt(record, value)).toThrow();
    expect(() => validateStartReceipt(record, { ...receipt, id: uuid('b') }, receipt.id)).toThrow();
    expect(requestBody(readPending(saved)!)).toEqual(requestBody(record));
    expect(new ApiFailure('STALE_REVISION').unknownOutcome).toBe(false);
  });
  it('checks envelope, purpose and budget with the same attempt GET before accepting a receipt', () => {
    expect(
      validateAttemptContext(attemptView, data, 'cell/a', uuid('4'), receipt.id, record).attempt
        .status,
    ).toBe('ARMING');
    for (const value of [
      { ...attemptView, run: { ...attemptView.run, envelope_digest: 'c'.repeat(64) } },
      { ...attemptView, run: { ...attemptView.run, purpose: 'SETUP' } },
      {
        ...attemptView,
        run: { ...attemptView.run, budget: { ...attemptView.run.budget, limit: '3' } },
      },
      { ...attemptView, attempt: { ...receipt, id: uuid('b') } },
    ])
      expect(() =>
        validateAttemptContext(value, data, 'cell/a', uuid('4'), receipt.id, record),
      ).toThrow();
    const saved = storage();
    saveStartWatch(saved, { request: record, attempt: receipt });
    expect(readStartWatch(saved)?.attempt.id).toBe(receipt.id);
    expect(requestBody(readStartWatch(saved)!.request)).toEqual(requestBody(record));
    expect(() => readStartWatch({ getItem: () => '{bad-json' })).toThrow();
  });
  it('does not infer STARTED from Host counts or REJECTED from elapsed deadlines', () => {
    const elapsed = {
      ...attemptView,
      checked_at: { clock_id: 'clock/a', ticks_ns: '200' },
      deadline_status: 'ELAPSED',
      attempt: { ...receipt, acknowledgments: { ...receipt.host_boots } },
    };
    expect(
      validateAttemptContext(elapsed, data, 'cell/a', uuid('4'), receipt.id).attempt.status,
    ).toBe('ARMING');
    expect(() =>
      validateAttemptContext(
        { ...elapsed, deadline_status: 'WITHIN_DEADLINE' },
        data,
        'cell/a',
        uuid('4'),
        receipt.id,
      ),
    ).toThrow();
    for (const status of ['STARTED', 'REJECTED'] as const) {
      const value = {
        ...elapsed,
        attempt: { ...elapsed.attempt, status, mandate: status === 'STARTED' ? uuid('b') : null },
      };
      expect(
        validateAttemptContext(value, data, 'cell/a', uuid('4'), receipt.id).attempt.status,
      ).toBe(status);
    }
    const changed = {
      ...attemptView,
      attempt: { ...receipt, valid_until: { clock_id: 'clock/old', ticks_ns: '200' } },
      deadline_status: 'CLOCK_CHANGED',
    };
    expect(
      validateAttemptContext(changed, data, 'cell/a', uuid('4'), receipt.id).deadline_status,
    ).toBe('CLOCK_CHANGED');
  });
});

describe('start attempt display against the latest overview', () => {
  const started: AttemptContext = {
    ...attemptView,
    run_revision: '9007199254740996',
    run: { ...attemptView.run, state: 'EXECUTING', pending_attempt: null },
    attempt: {
      ...receipt,
      status: 'STARTED',
      mandate: uuid('b'),
      acknowledgments: { ...receipt.host_boots },
    },
  };
  const executing = { ...cell, runs: [{ revision: started.run_revision, value: started.run }] };
  const display = {
    attempt: started,
    data,
    cell: executing,
    run: started.run.id,
    id: started.attempt.id,
    queryFresh: true,
    queryFailed: false,
    receivedAt: 100,
    now: 101,
  };

  it('immediately shows a held Run from overview while the older STARTED GET remains historical', () => {
    expect(startStatusDisplay(display)).toMatchObject({
      runState: 'EXECUTING',
      attemptFresh: true,
    });
    const held = {
      ...executing,
      runs: [
        {
          revision: '9007199254740997',
          value: { ...started.run, state: 'RECOVERY_REQUIRED' as const },
        },
      ],
    };
    expect(startStatusDisplay({ ...display, cell: held })).toEqual({
      runState: 'RECOVERY_REQUIRED',
      attemptMatches: true,
      attemptCurrent: false,
      attemptFresh: false,
    });
    expect(started.attempt.status).toBe('STARTED');
    expect(started.run.state).toBe('EXECUTING');
  });

  it('downgrades the same Run and attempt IDs across runtime, installation and store changes', () => {
    for (const installationChange of [
      { ...installation, runtime_boot: uuid('c') },
      { ...installation, id: uuid('c') },
      { ...installation, store_generation: uuid('c') },
      { ...installation, clock_id: 'clock/next' },
    ]) {
      expect(
        startStatusDisplay({ ...display, data: { ...data, installation: installationChange } }),
      ).toMatchObject({
        runState: 'EXECUTING',
        attemptMatches: true,
        attemptCurrent: false,
        attemptFresh: false,
      });
    }
  });

  it('requires matching Run revision even if both reads still say EXECUTING', () => {
    for (const revision of ['9007199254740995', '9007199254740997']) {
      expect(
        startStatusDisplay({
          ...display,
          cell: { ...executing, runs: [{ ...executing.runs[0], revision }] },
        }),
      ).toMatchObject({
        runState: 'EXECUTING',
        attemptMatches: true,
        attemptCurrent: false,
        attemptFresh: false,
      });
    }
    expect(startStatusDisplay({ ...display, cell: { ...executing, runs: [] } })).toMatchObject({
      runState: 'UNKNOWN',
      attemptFresh: false,
    });
    expect(
      startStatusDisplay({
        ...display,
        cell: {
          ...executing,
          runs: [
            {
              revision: started.run_revision,
              value: { ...started.run, state: 'RECOVERY_REQUIRED' },
            },
          ],
        },
      }),
    ).toMatchObject({ runState: 'RECOVERY_REQUIRED', attemptCurrent: false, attemptFresh: false });
  });

  it('keeps the latest overview state after the follow-up attempt query fails', () => {
    const held = {
      ...executing,
      runs: [
        {
          revision: '9007199254740997',
          value: { ...started.run, state: 'RECOVERY_REQUIRED' as const },
        },
      ],
    };
    // A failed follow-up leaves the old successful GET and its recent receive time in memory.
    expect(startStatusDisplay({ ...display, cell: held, queryFailed: true })).toMatchObject({
      runState: 'RECOVERY_REQUIRED',
      attemptMatches: true,
      attemptFresh: false,
    });
    expect(startStatusDisplay({ ...display, queryFailed: true })).toMatchObject({
      runState: 'EXECUTING',
      attemptCurrent: true,
      attemptFresh: false,
    });
    expect(startStatusDisplay({ ...display, now: 10100 }).attemptFresh).toBe(false);
    expect(startStatusDisplay({ ...display, queryFresh: false }).attemptFresh).toBe(false);
  });
});
