import { z } from './schema-runtime';
import {
  artifactSchema,
  counter,
  pendingSchema,
  timeSchema,
  type Overview,
  type Pending,
} from './schema';
import { stableDocument } from './draft-schema';
import { valueSchema } from './diagnostics-schema';
import { canRecover } from './pending';

const id = z.uuid();
const digest = z.string().regex(/^[0-9a-f]{64}$/);
const name = z.string().min(1);
const counters = z.record(name, counter);
const ids = z.array(id);
const transport = z
  .object({
    uri: z.string(),
    server_name: z.string(),
    server_ca_digest: digest,
    server_leaf_digest: digest,
    platform_client_leaf_digest: digest,
    platform_client_chain_digest: digest,
    release: digest,
    base_manifest: digest,
    cell_manifest: digest,
    host_read_binding: digest,
    host_configuration_binding: digest,
  })
  .strict();
const actor = z
  .object({ principal: name, session: id, terminal: z.tuple([name, digest]).nullable() })
  .strict();
const block = z
  .object({ id, reason: name, latched: z.boolean(), scopes: z.array(name) })
  .passthrough();
const configuration = z
  .object({
    id: name,
    environment: z.enum(['SIMULATION', 'PHYSICAL']),
    definition: artifactSchema,
    envelope: artifactSchema,
    recipe: artifactSchema,
    site_config_digest: digest,
    hosts: z.array(name),
  })
  .passthrough();
const cellCut = z
  .object({
    revision: counter,
    epoch: counter,
    scopes: counters,
    configuration,
    configuration_digest: digest,
    blocks: z.array(block),
    runtime_origins: z.record(id, digest),
  })
  .strict();
const operationCut = z
  .object({
    operation: id,
    cell: name,
    permit: id,
    intent_digest: digest,
    profile_digest: digest,
    host_journal: id,
    invocation: id.nullable(),
    messages: ids,
  })
  .strict();
const blocker = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.enum([
        'BASELINE_MISSING',
        'REGISTRATION_MISSING',
        'IDENTITY_CHANGED',
        'RESTART_ORIGIN_MISSING',
        'LIVE_AUTHORITY',
        'FENCE_CANDIDATES_AMBIGUOUS',
      ]),
      cell: name,
    })
    .strict(),
  z.object({ kind: z.literal('TOO_MANY_OPERATIONS') }).strict(),
]);
export const recoveryContextSchema = z
  .object({
    installation: id,
    store_generation: id,
    runtime_boot: id,
    clock_id: name,
    host: name,
    origin: name,
    transport,
    producer: z
      .object({
        principal: name,
        session: id,
        peer_boot: id,
        journal: id,
        authentication_binding: digest,
        cells: z.record(name, digest),
      })
      .strict(),
    producer_revision: counter,
    anchor: id.nullable(),
    previous_runtime_boot: id,
    host_cells: z.array(name).min(1).max(64),
    cells: z.record(name, cellCut),
    registrations: z.record(
      name,
      z
        .object({
          revision: counter,
          digest,
          plan: id,
          plan_revision: counter,
          plan_digest: digest,
          baseline_digest: digest,
        })
        .strict(),
    ),
    operations: z.record(id, operationCut),
    blockers: z.array(blocker),
  })
  .strict()
  .superRefine((value, ctx) => {
    if (
      new Set(value.host_cells).size !== value.host_cells.length ||
      !value.host_cells.includes(value.origin) ||
      value.producer.principal !== value.host ||
      value.host_cells.some(
        (cell) => !value.cells[cell]?.configuration.hosts.includes(value.host),
      ) ||
      Object.entries(value.cells).some(([cell, cut]) => cut.configuration.id !== cell) ||
      value.blockers.some((r) => 'cell' in r && !value.cells[r.cell]) ||
      Object.entries(value.operations).some(
        ([key, operation]) =>
          key !== operation.operation || !value.host_cells.includes(operation.cell),
      )
    )
      ctx.addIssue({ code: 'custom', message: 'Recovery cohort identity differs.' });
  });
const observation = z
  .object({
    source: name,
    generation: id,
    schema: name,
    unit: name,
    value: valueSchema,
    acquired_at: timeSchema,
    uncertainty_ns: counter,
    quality_good: z.boolean(),
    origin_age_bounded: z.boolean(),
    disputed: z.boolean(),
    evidence_id: id,
  })
  .strict();
const snapshot = z
  .object({
    schema: z.literal('rx.host-snapshot.v1'),
    host: name,
    host_boot: id,
    delivery_journal: id,
    evidence_journal: id,
    cell: name,
    definition: digest,
    envelope: digest,
    environment: z.enum(['SIMULATION', 'PHYSICAL']),
    epoch: counter,
    scopes: counters,
    resource_fences: counters,
    captured_at: timeSchema,
    sources_available: z.boolean(),
    observations: z.array(observation).max(128),
    block_ids: ids,
    pending_operations: ids,
    pending_permits: ids,
  })
  .strict()
  .superRefine((value, ctx) => {
    if (!value.sources_available && value.observations.length)
      ctx.addIssue({ code: 'custom', message: 'Unavailable sources cannot contain observations.' });
  });
const applied = z
  .object({
    cell: name,
    configuration: digest,
    change: id,
    request: id,
    receipt_sequence: counter,
    binding_digest: digest,
  })
  .strict();
const configurationRead = z
  .object({
    schema: z.literal('rx.host-process-configuration-observation.v1'),
    snapshot: z
      .object({
        schema: z.literal('rx.host-process-configuration-snapshot.v1'),
        host: name,
        host_boot: id,
        delivery_journal: id,
        binding_digest: digest,
        cells: z.array(
          z
            .object({
              cell: name,
              definition: digest,
              envelope: digest,
              environment: z.enum(['SIMULATION', 'PHYSICAL']),
              epoch: counter,
              scopes: counters,
              blocked: ids,
              applied: applied.nullable(),
            })
            .strict(),
        ),
      })
      .strict(),
    receipt: z.unknown().nullable(),
    context_matches_current_host: z.boolean(),
    activation_authorized: z.literal(false),
  })
  .strict();
const readEvidence = z
  .object({
    platform_session: id,
    transport,
    configuration: configurationRead,
    configuration_started: timeSchema,
    configuration_finished: timeSchema,
    cells: z.record(
      name,
      z.object({ snapshot, started: timeSchema, finished: timeSchema }).strict(),
    ),
  })
  .strict();
const fenceTask = z
  .object({
    binding: id,
    cell: name,
    request: id,
    originating_message: id.nullable(),
    epoch: counter,
    scopes: counters,
    block_ids: ids,
  })
  .strict();
const acknowledgment = z
  .object({
    cell: name,
    invalidation: id,
    epoch: counter,
    scopes: counters,
    host_boot: id,
    journal: id,
    sequence: counter,
  })
  .strict();
const fenceStep = z
  .object({
    task: fenceTask,
    payload_digest: digest,
    phase: z.enum(['PENDING', 'SEND_ENTERED', 'ACKNOWLEDGED']),
    acknowledgment: acknowledgment.nullable(),
  })
  .strict();
const binding = z
  .object({
    schema: z.literal('rx.host-recovery.v1'),
    id,
    revision: counter,
    requested_context_digest: digest,
    phase: z.enum(['PROPOSED', 'FENCING', 'RECOVERY_ONLY', 'ATTENTION']),
    context: recoveryContextSchema,
    proposed_by: actor,
    proposed_at: timeSchema,
    proposal_read: readEvidence,
    approved_by: actor.nullable(),
    approved_at: timeSchema.nullable(),
    fences: z.record(name, fenceStep),
    last_read: readEvidence.nullable(),
    detail: z.string().nullable(),
  })
  .strict();
export const recoveryViewSchema = z
  .object({
    view: z
      .object({ binding, current: z.boolean(), operation_authorized: z.literal(false) })
      .strict(),
    proposal_digest: digest,
  })
  .strict();
export const recoveryContextResponseSchema = z
  .object({ context: recoveryContextSchema, context_digest: digest, expected_cells: counters })
  .strict();
export const recoveryPageSchema = z
  .object({ items: z.array(recoveryViewSchema).max(50), next: id.nullable() })
  .strict();
const receiptSchema = z
  .object({
    operation: id,
    digest,
    invocation: id.nullable(),
    journal: id,
    sequence: counter,
    state: z.enum([
      'PREPARED',
      'SEND_ENTERED',
      'NATIVE_ACCEPTED',
      'NATIVE_REJECTED',
      'RESULT_CAPTURED',
      'VOIDED_BEFORE_SEND',
    ]),
  })
  .strict();
export const recoveryQuerySchema = z
  .object({
    binding: id,
    operation: id,
    receipt: receiptSchema.nullable(),
    lookup: z.enum(['NOT_NEEDED', 'PREFIX_OBSERVED', 'UNAVAILABLE', 'UNSUPPORTED']),
    evidence: z.array(
      z
        .object({
          id,
          operation: id,
          invocation: id,
          profile_digest: digest,
          device_session: id,
          status_schema: name,
          status: z.string().regex(/^-?[0-9]+$/),
          captured_at: timeSchema,
          native_details: z
            .object({ native_id: z.string().nullable(), native_data: artifactSchema.nullable() })
            .nullable()
            .optional(),
        })
        .strict(),
    ),
    evidence_complete: z.literal(false),
    publication_required: z.boolean(),
    operation_authorized: z.literal(false),
  })
  .strict();
export type RecoveryContext = z.infer<typeof recoveryContextSchema>;
export type RecoveryContextResponse = z.infer<typeof recoveryContextResponseSchema>;
export type RecoveryView = z.infer<typeof recoveryViewSchema>;
export type RecoveryQuery = z.infer<typeof recoveryQuerySchema>;
export type RecoveryTransientRoute =
  '/api/v1/host-recovery/progress' | '/api/v1/host-recovery/query';

export function recoveryRoute(route: string) {
  return route === '/api/v1/host-recoveries' || route === '/api/v1/host-recovery/approve';
}
function check(value: boolean) {
  if (!value) throw new Error('Host recovery response correlation');
}
const same = (a: unknown, b: unknown) => stableDocument(a) === stableDocument(b);
export const expectedRecoveryCells = (context: RecoveryContext) =>
  Object.fromEntries(Object.entries(context.cells).map(([cell, cut]) => [cell, cut.revision]));
export function recoveryInstallationMatches(context: RecoveryContext, data: Overview) {
  return (
    context.installation === data.installation.id &&
    context.store_generation === data.installation.store_generation &&
    context.runtime_boot === data.installation.runtime_boot &&
    context.clock_id === data.installation.clock_id
  );
}
export function recoveryContextCurrent(context: RecoveryContext, data: Overview) {
  return (
    recoveryInstallationMatches(context, data) &&
    Object.entries(context.cells).every(([cell, cut]) => {
      const current = data.cells.find((item) => item.cell.value.id === cell);
      return (
        current?.cell.revision === cut.revision &&
        current.cell.value.epoch === cut.epoch &&
        current.cell.value.definition.sha256 === cut.configuration.definition.sha256 &&
        current.cell.value.envelope.sha256 === cut.configuration.envelope.sha256 &&
        current.cell.value.recipe.sha256 === cut.configuration.recipe.sha256
      );
    })
  );
}
export function canRecoverHostRequest(record: Pending, data: Overview) {
  return (
    canRecover(
      record,
      data.user.principal,
      data.installation.id,
      data.installation.store_generation,
    ) &&
    (!recoveryRoute(record.route) ||
      (!!record.recovery_review &&
        record.recovery_review.runtime_boot === data.installation.runtime_boot &&
        data.user.roles.includes('RELEASE_MANAGER') &&
        !!data.user.terminal))
  );
}
export async function validateRecoveryContext(
  raw: unknown,
  data: Overview,
  host: string,
  origin: string,
) {
  const value = recoveryContextResponseSchema.parse(raw);
  check(
    value.context.host === host &&
      value.context.origin === origin &&
      recoveryInstallationMatches(value.context, data),
  );
  check(same(value.expected_cells, expectedRecoveryCells(value.context)));
  return value;
}
export async function validateRecoveryView(raw: unknown, host: string, expectedId?: string) {
  const value = recoveryViewSchema.parse(raw);
  const b = value.view.binding;
  check(b.context.host === host && (!expectedId || b.id === expectedId));
  check(
    (b.approved_by === null) === (b.approved_at === null) &&
      (!['FENCING', 'RECOVERY_ONLY'].includes(b.phase) || !!b.approved_by),
  );
  check(same(Object.keys(b.fences).sort(), [...b.context.host_cells].sort()));
  for (const [cell, step] of Object.entries(b.fences)) {
    check(
      step.task.binding === b.id &&
        step.task.cell === cell &&
        step.task.epoch === b.context.cells[cell].epoch &&
        same(step.task.scopes, b.context.cells[cell].scopes) &&
        (!step.task.originating_message || step.task.originating_message === step.task.request) &&
        (step.phase === 'ACKNOWLEDGED') === (step.acknowledgment !== null),
    );
    if (step.acknowledgment)
      check(
        step.acknowledgment.cell === cell &&
          step.acknowledgment.invalidation === step.task.request &&
          step.acknowledgment.epoch === step.task.epoch &&
          same(step.acknowledgment.scopes, step.task.scopes) &&
          step.acknowledgment.host_boot === b.context.producer.peer_boot,
      );
  }
  check(
    b.phase !== 'RECOVERY_ONLY' ||
      (!!b.last_read && Object.values(b.fences).every((f) => f.phase === 'ACKNOWLEDGED')),
  );
  check(!value.view.current || (b.phase === 'RECOVERY_ONLY' && !!b.last_read));
  for (const read of [
    b.proposal_read,
    ...(b.phase === 'RECOVERY_ONLY' && b.last_read ? [b.last_read] : []),
  ]) {
    check(
      read.configuration.snapshot.host === host &&
        same(read.transport, b.context.transport) &&
        same(Object.keys(read.cells).sort(), [...b.context.host_cells].sort()),
    );
    for (const [cell, r] of Object.entries(read.cells))
      check(
        r.snapshot.cell === cell &&
          r.snapshot.host === host &&
          r.snapshot.host_boot === b.context.producer.peer_boot,
      );
  }
  return value;
}
export async function validateRecoveryPage(raw: unknown, host: string) {
  const value = recoveryPageSchema.parse(raw);
  check(new Set(value.items.map((item) => item.view.binding.id)).size === value.items.length);
  return {
    ...value,
    items: await Promise.all(value.items.map((item) => validateRecoveryView(item, host))),
  };
}
export async function validateRecoveryProgress(raw: unknown, previous: RecoveryView) {
  const old = previous.view.binding;
  const value = await validateRecoveryView(raw, old.context.host, old.id);
  const current = value.view.binding;
  check(
    value.proposal_digest === previous.proposal_digest &&
      current.requested_context_digest === old.requested_context_digest &&
      same(current.context, old.context) &&
      BigInt(current.revision) >= BigInt(old.revision) &&
      same(
        Object.entries(current.fences).map(([cell, f]) => [cell, f.task, f.payload_digest]),
        Object.entries(old.fences).map(([cell, f]) => [cell, f.task, f.payload_digest]),
      ),
  );
  return value;
}
export async function freezeRecoveryProposal(
  context: RecoveryContextResponse,
  data: Overview,
  key: string,
): Promise<Pending> {
  check(
    data.user.roles.includes('RELEASE_MANAGER') &&
      !!data.user.terminal &&
      recoveryContextCurrent(context.context, data) &&
      same(context.expected_cells, expectedRecoveryCells(context.context)) &&
      context.context.blockers.length === 0,
  );
  return pendingSchema.parse({
    request_key: key,
    principal: data.user.principal,
    installation: data.installation.id,
    store_generation: data.installation.store_generation,
    route: '/api/v1/host-recoveries',
    label: `${context.context.host} 복구 연결 제안`,
    command: {
      host: context.context.host,
      origin: context.context.origin,
      expected_context: context.context_digest,
      expected_cells: { ...context.expected_cells },
    },
    recovery_review: {
      host: context.context.host,
      origin: context.context.origin,
      runtime_boot: context.context.runtime_boot,
      context_digest: context.context_digest,
    },
  });
}
export async function freezeRecoveryApproval(
  view: RecoveryView,
  data: Overview,
  key: string,
): Promise<Pending> {
  const b = view.view.binding;
  check(
    data.user.roles.includes('RELEASE_MANAGER') &&
      !!data.user.terminal &&
      recoveryContextCurrent(b.context, data) &&
      b.context.blockers.length === 0,
  );
  return pendingSchema.parse({
    request_key: key,
    principal: data.user.principal,
    installation: data.installation.id,
    store_generation: data.installation.store_generation,
    route: '/api/v1/host-recovery/approve',
    label: `${b.context.host} 복구 조회 연결 승인`,
    command: {
      id: b.id,
      expected_revision: b.revision,
      proposal_digest: view.proposal_digest,
      expected_cells: expectedRecoveryCells(b.context),
    },
    recovery_review: {
      host: b.context.host,
      origin: b.context.origin,
      runtime_boot: b.context.runtime_boot,
      context_digest: b.requested_context_digest,
      binding: b.id,
      proposal_digest: view.proposal_digest,
      fence_requests: Object.fromEntries(
        Object.entries(b.fences).map(([cell, f]) => [cell, f.task.request]),
      ),
      fence_payloads: Object.fromEntries(
        Object.entries(b.fences).map(([cell, f]) => [cell, f.payload_digest]),
      ),
    },
  });
}
export async function validateRecoveryReceipt(record: Pending, raw: unknown) {
  const saved = pendingSchema.parse(record);
  const review = saved.recovery_review;
  check(recoveryRoute(saved.route) && !!review);
  if (!review) throw new Error('Missing reviewed Host');
  const value = await validateRecoveryView(raw, review.host, review.binding);
  const b = value.view.binding;
  check(
    b.context.origin === review.origin &&
      b.context.installation === saved.installation &&
      b.context.store_generation === saved.store_generation &&
      b.context.runtime_boot === review.runtime_boot &&
      same(expectedRecoveryCells(b.context), saved.command.expected_cells),
  );
  if (saved.route === '/api/v1/host-recoveries') {
    check(
      b.proposed_by.principal === saved.principal &&
        b.requested_context_digest === review.context_digest,
    );
  } else
    check(
      value.proposal_digest === review.proposal_digest &&
        b.requested_context_digest === review.context_digest &&
        BigInt(b.revision) > BigInt(String(saved.command.expected_revision)) &&
        same(
          Object.fromEntries(Object.entries(b.fences).map(([cell, f]) => [cell, f.task.request])),
          review.fence_requests,
        ) &&
        same(
          Object.fromEntries(Object.entries(b.fences).map(([cell, f]) => [cell, f.payload_digest])),
          review.fence_payloads,
        ),
    );
  return value;
}
export function validateRecoveryQuery(raw: unknown, view: RecoveryView, operation: string) {
  const value = recoveryQuerySchema.parse(raw);
  const b = view.view.binding;
  const expected = b.context.operations[operation];
  check(!!expected && value.binding === b.id && value.operation === operation);
  if (value.receipt)
    check(
      value.receipt.operation === operation &&
        value.receipt.digest === expected.intent_digest &&
        value.receipt.journal === expected.host_journal &&
        (!expected.invocation || value.receipt.invocation === expected.invocation),
    );
  check(
    value.evidence.every(
      (item) =>
        item.operation === operation &&
        item.profile_digest === expected.profile_digest &&
        (!expected.invocation || item.invocation === expected.invocation),
    ),
  );
  return value;
}
