import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { api } from './api';
import { readPending, requestBody, savePending } from './pending';
import { pendingSchema, type Overview } from './schema';
import { RecoveryEvidence } from './host-recovery';
import {
  canRecoverHostRequest,
  freezeRecoveryApproval,
  freezeRecoveryProposal,
  recoveryContextCurrent,
  recoveryContextResponseSchema,
  recoveryViewSchema,
  validateRecoveryContext,
  validateRecoveryPage,
  validateRecoveryProgress,
  validateRecoveryQuery,
  validateRecoveryReceipt,
  validateRecoveryView,
} from './host-recovery-schema';

const uuid = (digit: string) =>
  `${digit.repeat(8)}-${digit.repeat(4)}-4${digit.repeat(3)}-8${digit.repeat(3)}-${digit.repeat(12)}`;
const hash = (digit: string) => digit.repeat(64);
const time = { clock_id: 'clock/a', ticks_ns: '100' };
const ref = (digit: string) => ({ sha256: hash(digit), schema_id: 'rx.test.v1', size_bytes: '1' });
function fixture() {
  const transport = {
    uri: 'https://host.internal:8443',
    server_name: 'host.internal',
    server_ca_digest: hash('a'),
    server_leaf_digest: hash('b'),
    platform_client_leaf_digest: hash('c'),
    platform_client_chain_digest: hash('d'),
    release: hash('e'),
    base_manifest: hash('f'),
    cell_manifest: hash('a'),
    host_read_binding: hash('b'),
    host_configuration_binding: hash('c'),
  };
  const configuration = {
    id: 'cell/a',
    environment: 'SIMULATION',
    definition: ref('a'),
    envelope: ref('b'),
    recipe: ref('c'),
    site_config_digest: hash('d'),
    hosts: ['host/a'],
  };
  const context = recoveryContextResponseSchema.parse({
    context: {
      installation: uuid('1'),
      store_generation: uuid('2'),
      runtime_boot: uuid('3'),
      clock_id: 'clock/a',
      host: 'host/a',
      origin: 'cell/a',
      transport,
      producer: {
        principal: 'host/a',
        session: uuid('4'),
        peer_boot: uuid('6'),
        journal: uuid('7'),
        authentication_binding: hash('a'),
        cells: { 'cell/a': hash('a') },
      },
      producer_revision: '4',
      anchor: null,
      previous_runtime_boot: uuid('5'),
      host_cells: ['cell/a'],
      cells: {
        'cell/a': {
          revision: '9007199254740993',
          epoch: '2',
          scopes: { 'scope/a': '2' },
          configuration,
          configuration_digest: hash('d'),
          blocks: [
            { id: uuid('b'), reason: 'RUNTIME_RESTART', latched: true, scopes: ['scope/a'] },
          ],
          runtime_origins: { [uuid('b')]: hash('e') },
        },
      },
      registrations: {
        'cell/a': {
          revision: '2',
          digest: hash('a'),
          plan: uuid('c'),
          plan_revision: '2',
          plan_digest: hash('b'),
          baseline_digest: hash('c'),
        },
      },
      operations: {
        [uuid('e')]: {
          operation: uuid('e'),
          cell: 'cell/a',
          permit: uuid('f'),
          intent_digest: hash('b'),
          profile_digest: hash('c'),
          host_journal: uuid('8'),
          invocation: null,
          messages: [uuid('d')],
        },
      },
      blockers: [],
    },
    context_digest: hash('a'),
    expected_cells: { 'cell/a': '9007199254740993' },
  });
  const read = {
    platform_session: uuid('9'),
    transport,
    configuration: {
      schema: 'rx.host-process-configuration-observation.v1',
      snapshot: {
        schema: 'rx.host-process-configuration-snapshot.v1',
        host: 'host/a',
        host_boot: uuid('6'),
        delivery_journal: uuid('8'),
        binding_digest: hash('d'),
        cells: [
          {
            cell: 'cell/a',
            definition: hash('a'),
            envelope: hash('b'),
            environment: 'SIMULATION',
            epoch: '2',
            scopes: { 'scope/a': '2' },
            blocked: [uuid('b')],
            applied: null,
          },
        ],
      },
      receipt: null,
      context_matches_current_host: true,
      activation_authorized: false,
    },
    configuration_started: time,
    configuration_finished: time,
    cells: {
      'cell/a': {
        started: time,
        finished: time,
        snapshot: {
          schema: 'rx.host-snapshot.v1',
          host: 'host/a',
          host_boot: uuid('6'),
          delivery_journal: uuid('8'),
          evidence_journal: uuid('7'),
          cell: 'cell/a',
          definition: hash('a'),
          envelope: hash('b'),
          environment: 'SIMULATION',
          epoch: '2',
          scopes: { 'scope/a': '2' },
          resource_fences: {},
          captured_at: time,
          sources_available: true,
          observations: [],
          block_ids: [uuid('b')],
          pending_operations: [uuid('e')],
          pending_permits: [uuid('f')],
        },
      },
    },
  };
  const view = recoveryViewSchema.parse({
    view: {
      current: false,
      operation_authorized: false,
      binding: {
        schema: 'rx.host-recovery.v1',
        id: uuid('a'),
        revision: '1',
        requested_context_digest: hash('a'),
        phase: 'PROPOSED',
        context: context.context,
        proposed_by: {
          principal: 'release/a',
          session: uuid('b'),
          terminal: ['terminal/a', hash('c')],
        },
        proposed_at: time,
        proposal_read: read,
        approved_by: null,
        approved_at: null,
        fences: {
          'cell/a': {
            task: {
              binding: uuid('a'),
              cell: 'cell/a',
              request: uuid('d'),
              originating_message: null,
              epoch: '2',
              scopes: { 'scope/a': '2' },
              block_ids: [uuid('b')],
            },
            payload_digest: hash('f'),
            phase: 'PENDING',
            acknowledgment: null,
          },
        },
        last_read: null,
        detail: null,
      },
    },
    proposal_digest: hash('b'),
  });
  const data = {
    installation: {
      id: uuid('1'),
      store_generation: uuid('2'),
      runtime_boot: uuid('3'),
      clock_id: 'clock/a',
    },
    user: {
      principal: 'release/a',
      terminal: 'terminal/a',
      roles: ['RELEASE_MANAGER'],
      cells: ['cell/a'],
    },
    cells: [{ cell: { revision: '9007199254740993', value: { ...configuration, epoch: '2' } } }],
  } as unknown as Overview;
  return { context, view, data };
}
function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
  };
}

describe('Host recovery reads and immutable request identity', () => {
  it('checks selected Host/origin, installation, Counter CAS and digest wrapper shape', async () => {
    const { context, data } = fixture();
    await expect(validateRecoveryContext(context, data, 'host/a', 'cell/a')).resolves.toEqual(
      context,
    );
    for (const value of [
      { ...context, expected_cells: { 'cell/a': '9007199254740992' } },
      { ...context, context_digest: 'not-a-digest' },
      { ...context, context: { ...context.context, runtime_boot: uuid('f') } },
      { ...context, expected_cells: { 'cell/a': 2 } },
    ])
      await expect(validateRecoveryContext(value, data, 'host/a', 'cell/a')).rejects.toThrow();
    await expect(validateRecoveryContext(context, data, 'host/other', 'cell/a')).rejects.toThrow();
    await expect(validateRecoveryContext(context, data, 'host/a', 'cell/other')).rejects.toThrow();
  });

  it('persists the original proposal key/body and Host namespace over response loss and reload', async () => {
    const { context, data } = fixture();
    const pending = await freezeRecoveryProposal(context, data, uuid('c'));
    const saved = storage();
    savePending(saved, pending);
    const expected = JSON.stringify(requestBody(pending));
    context.expected_cells['cell/a'] = '9007199254740994';
    expect(JSON.stringify(requestBody(pending))).toBe(expected);
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (_path, options) => {
      expect(options?.body).toBe(expected);
      throw new Error('response lost after commit');
    };
    try {
      await expect(api(pending.route, { body: requestBody(pending) })).rejects.toMatchObject({
        unknownOutcome: true,
      });
    } finally {
      globalThis.fetch = originalFetch;
    }
    expect(JSON.stringify(requestBody(readPending(saved)!))).toBe(expected);
    expect(readPending(saved)?.recovery_review?.host).toBe('host/a');
    for (const value of [
      { ...pending, recovery_review: undefined },
      { ...pending, command: { ...pending.command, host: 'host/other' } },
      { ...pending, command: { ...pending.command, uri: 'https://user-selected.invalid' } },
      { ...pending, command: { ...pending.command, snapshot: {} } },
    ])
      expect(pendingSchema.safeParse(value).success).toBe(false);
  });

  it('uses the requested-context echo for Attention receipts without rewriting added blockers', async () => {
    const { context, data, view } = fixture();
    const pending = await freezeRecoveryProposal(context, data, uuid('c'));
    const attention = structuredClone(view);
    attention.view.binding.phase = 'ATTENTION';
    attention.view.binding.context.blockers.push({
      kind: 'FENCE_CANDIDATES_AMBIGUOUS',
      cell: 'cell/a',
    });
    expect(
      (await validateRecoveryReceipt(pending, attention)).view.binding.context.blockers,
    ).toHaveLength(1);
    attention.view.binding.requested_context_digest = hash('f');
    await expect(validateRecoveryReceipt(pending, attention)).rejects.toThrow();
  });

  it('recovers a cached proposal after the same principal logs in again without rewriting historical actor fields', async () => {
    const { context, data, view } = fixture();
    const pending = await freezeRecoveryProposal(context, data, uuid('c'));
    const relogged = { ...data, user: { ...data.user, terminal: 'terminal/new' } };
    expect(canRecoverHostRequest(pending, relogged)).toBe(true);
    const recovered = await validateRecoveryReceipt(pending, view);
    expect(recovered.view.binding.proposed_by).toEqual(view.view.binding.proposed_by);
    expect(recovered.view.binding.proposed_by.terminal?.[0]).toBe('terminal/a');
    const other = structuredClone(view);
    other.view.binding.proposed_by.principal = 'release/other';
    await expect(validateRecoveryReceipt(pending, other)).rejects.toThrow();
  });

  it('discovers another current ReleaseManager proposal and allows approval despite expired read.current', async () => {
    const { data, view } = fixture();
    const another = { ...data, user: { ...data.user, principal: 'release/b' } };
    const page = await validateRecoveryPage({ items: [view], next: null }, 'host/a');
    expect(page.items[0].view.binding.proposed_by.principal).toBe('release/a');
    expect(page.items[0].view.current).toBe(false);
    const approval = await freezeRecoveryApproval(page.items[0], another, uuid('c'));
    expect(approval.principal).toBe('release/b');
    expect(approval.command).toEqual({
      id: uuid('a'),
      expected_revision: '1',
      proposal_digest: hash('b'),
      expected_cells: { 'cell/a': '9007199254740993' },
    });
    expect(approval.recovery_review?.fence_requests).toEqual({ 'cell/a': uuid('d') });
  });

  it('rejects approval responses with different proposal, Fence IDs, cell cut, actor scope or binding', async () => {
    const { data, view } = fixture();
    const approval = await freezeRecoveryApproval(view, data, uuid('c'));
    const returned = structuredClone(view);
    returned.view.binding.phase = 'FENCING';
    returned.view.binding.revision = '2';
    returned.view.binding.approved_by = returned.view.binding.proposed_by;
    returned.view.binding.approved_at = time;
    await expect(validateRecoveryReceipt(approval, returned)).resolves.toEqual(returned);
    for (const mutate of [
      (v: typeof returned) => {
        v.proposal_digest = hash('c');
      },
      (v: typeof returned) => {
        v.view.binding.id = uuid('c');
      },
      (v: typeof returned) => {
        v.view.binding.context.cells['cell/a'].revision = '4';
      },
      (v: typeof returned) => {
        v.view.binding.fences['cell/a'].task.request = uuid('f');
      },
      (v: typeof returned) => {
        v.view.binding.fences['cell/a'].payload_digest = hash('e');
      },
      (v: typeof returned) => {
        v.view.operation_authorized = true as false;
      },
    ]) {
      const changed = structuredClone(returned);
      mutate(changed);
      await expect(validateRecoveryReceipt(approval, changed)).rejects.toThrow();
    }
  });

  it('disables old runtime/store/principal/terminal/role and stale P cell CAS without discarding pending', async () => {
    const { context, data, view } = fixture();
    const pending = await freezeRecoveryApproval(view, data, uuid('c'));
    const saved = storage();
    savePending(saved, pending);
    for (const changed of [
      { ...data, installation: { ...data.installation, runtime_boot: uuid('f') } },
      { ...data, installation: { ...data.installation, store_generation: uuid('f') } },
      { ...data, user: { ...data.user, principal: 'release/other' } },
      { ...data, user: { ...data.user, terminal: null } },
      { ...data, user: { ...data.user, roles: [] } },
    ])
      expect(canRecoverHostRequest(pending, changed)).toBe(false);
    const changed = structuredClone(data);
    changed.cells[0].cell.revision = '9007199254740994';
    expect(recoveryContextCurrent(context.context, changed)).toBe(false);
    await expect(freezeRecoveryApproval(view, changed, uuid('d'))).rejects.toThrow();
    const shared = structuredClone(context.context);
    shared.cells['cell/shared'] = {
      ...structuredClone(shared.cells['cell/a']),
      configuration: { ...shared.cells['cell/a'].configuration, id: 'cell/shared', hosts: [] },
    };
    expect(recoveryContextCurrent(shared, data)).toBe(false);
    expect(requestBody(readPending(saved)!)).toEqual(requestBody(pending));
  });

  it('preserves proposal and Fence identities during progress and rejects contradictory RecoveryOnly', async () => {
    const { view } = fixture();
    await expect(validateRecoveryProgress(view, view)).resolves.toEqual(view);
    const changed = structuredClone(view);
    changed.view.binding.fences['cell/a'].task.request = uuid('f');
    await expect(validateRecoveryProgress(changed, view)).rejects.toThrow();
    const impossible = structuredClone(view);
    impossible.view.binding.phase = 'RECOVERY_ONLY';
    impossible.view.current = true;
    await expect(validateRecoveryView(impossible, 'host/a')).rejects.toThrow();
  });

  it('validates exact original operation query and never turns incomplete lookup into a permit or absence proof', () => {
    const { view } = fixture();
    const result = {
      binding: uuid('a'),
      operation: uuid('e'),
      receipt: null,
      lookup: 'UNAVAILABLE',
      evidence: [],
      evidence_complete: false,
      publication_required: false,
      operation_authorized: false,
    };
    expect(validateRecoveryQuery(result, view, uuid('e')).receipt).toBeNull();
    for (const value of [
      { ...result, binding: uuid('b') },
      { ...result, operation: uuid('b') },
      { ...result, evidence_complete: true },
      { ...result, operation_authorized: true },
    ])
      expect(() => validateRecoveryQuery(value, view, uuid('e'))).toThrow();
    expect(() => validateRecoveryQuery(result, view, uuid('f'))).toThrow();
  });

  it('shows missing sources and uncertain quality as observations needing review', () => {
    const { view } = fixture();
    const b = view.view.binding;
    b.proposal_read.cells['cell/a'].snapshot.sources_available = false;
    const missing = renderToStaticMarkup(createElement(RecoveryEvidence, { binding: b }));
    expect(missing).toContain('Observation source unavailable');
    expect(missing).not.toContain('Review stored quality fields');
    b.proposal_read.cells['cell/a'].snapshot.sources_available = true;
    b.proposal_read.cells['cell/a'].snapshot.observations.push({
      source: 'source/a',
      generation: uuid('a'),
      schema: 'boolean/v1',
      unit: 'unit/none',
      value: { boolean: false },
      acquired_at: time,
      uncertainty_ns: '0',
      quality_good: false,
      origin_age_bounded: false,
      disputed: false,
      evidence_id: uuid('b'),
    });
    const uncertain = renderToStaticMarkup(createElement(RecoveryEvidence, { binding: b }));
    expect(uncertain).toContain('Quality, time, and conflicts need verification');
    expect(uncertain).not.toContain('class="success"');
  });

  it('keeps an Attention read with changed Host boot historical, without asserting current identity', async () => {
    const { view } = fixture();
    const changed = structuredClone(view);
    changed.view.binding.phase = 'ATTENTION';
    changed.view.binding.last_read = structuredClone(changed.view.binding.proposal_read);
    changed.view.binding.last_read.cells['cell/a'].snapshot.host_boot = uuid('f');
    const value = await validateRecoveryView(changed, 'host/a');
    const html = renderToStaticMarkup(
      createElement(RecoveryEvidence, { binding: value.view.binding }),
    );
    expect(html).toContain('It is not evidence for the current binding.');
    expect(value.view.current).toBe(false);
  });
});
