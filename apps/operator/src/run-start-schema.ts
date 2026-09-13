import { z } from './schema-runtime';
import {
  artifactSchema,
  counter,
  installationSchema,
  pendingSchema,
  runSchema,
  startCommandSchema,
  timeSchema,
  type CellOverview,
  type Overview,
  type Pending,
} from './schema';

const digest = z.string().regex(/^[0-9a-f]{64}$/);
const rejection = z.enum([
  'FORBIDDEN',
  'UNAUTHENTICATED',
  'NOT_FOUND',
  'NOT_COMMISSIONED',
  'QUALIFICATION_REQUIRED',
  'CONDITION_FAILED',
  'CONDITION_UNKNOWN',
  'STALE_REVISION',
  'EXPIRED',
  'STALE_EPOCH',
  'MANDATE_REVOKED',
  'BUDGET_EXHAUSTED',
  'BLOCKED_BY_CASE',
  'CAPABILITY_MISSING',
  'BUSY',
  'INVALID_INPUT',
  'UNSUPPORTED_SCHEMA',
  'HOST_NOT_PREPARED',
  'CONTINUITY_UNPROVEN',
]);
const operatorRun = runSchema.extend({
  pending_attempt: z.uuid().nullable(),
  recipe_digest: digest,
  envelope_digest: digest,
});
export const startContextSchema = z
  .object({
    installation: installationSchema,
    checked_at: timeSchema,
    cell: z.string(),
    cell_revision: counter,
    epoch: counter,
    scope_epochs: z.record(z.string(), counter),
    configuration_digest: digest,
    environment: z.enum(['SIMULATION', 'PHYSICAL']),
    commissioning: z.enum(['NOT_COMMISSIONED', 'COMMISSIONED', 'REVALIDATION_REQUIRED']).nullable(),
    envelope: artifactSchema,
    recipe: artifactSchema,
    site_config_digest: digest,
    maximum_budget: counter,
    run_revision: counter,
    run: operatorRun,
    request: startCommandSchema,
    can_request: z.boolean(),
    blocking_reason: rejection.nullable(),
  })
  .superRefine((value, context) => {
    if (value.can_request !== (value.blocking_reason === null))
      context.addIssue({ code: 'custom', message: 'Candidate decision is inconsistent.' });
  });
export const startAttemptSchema = z
  .object({
    id: z.uuid(),
    run: z.uuid(),
    cell: z.string(),
    expected_cell_revision: counter,
    expected_run_revision: counter,
    epoch: counter,
    scopes: z.record(z.string(), counter),
    host_boots: z.record(z.string(), z.uuid()),
    acknowledgments: z.record(z.string(), z.uuid()),
    executor_session: z.uuid(),
    actor: z.string(),
    status: z.enum(['PENDING', 'ARMING', 'STARTED', 'REJECTED']),
    mandate: z.uuid().nullable(),
    valid_until: timeSchema,
    terminal: z.tuple([z.string(), digest]).nullable(),
  })
  .superRefine((value, context) => {
    if (
      (value.status === 'STARTED') !== (value.mandate !== null) ||
      Object.entries(value.acknowledgments).some(
        ([host, boot]) => value.host_boots[host] !== boot,
      ) ||
      (value.status === 'STARTED' &&
        Object.keys(value.acknowledgments).length !== Object.keys(value.host_boots).length)
    )
      context.addIssue({ code: 'custom', message: 'Attempt state is inconsistent.' });
  });
export const attemptContextSchema = z.object({
  installation: installationSchema,
  checked_at: timeSchema,
  run_revision: counter,
  run: operatorRun,
  attempt: startAttemptSchema,
  deadline_status: z.enum(['WITHIN_DEADLINE', 'ELAPSED', 'CLOCK_CHANGED']),
});
export type StartContext = z.infer<typeof startContextSchema>;
export type StartAttempt = z.infer<typeof startAttemptSchema>;
export type AttemptContext = z.infer<typeof attemptContextSchema>;
export type StartWatch = { request: Pending; attempt: StartAttempt };

function requireMatch(value: boolean) {
  if (!value) throw new Error('start correlation');
}
function sameMap(left: Record<string, string>, right: Record<string, string>) {
  return (
    Object.keys(left).length === Object.keys(right).length &&
    Object.entries(left).every(([key, value]) => right[key] === value)
  );
}
export function sameInstallation(left: Overview['installation'], right: Overview['installation']) {
  return (
    left.id === right.id &&
    left.store_generation === right.store_generation &&
    left.runtime_boot === right.runtime_boot &&
    left.clock_id === right.clock_id
  );
}
export function positiveQuantity(value: string) {
  return counter.safeParse(value).success && BigInt(value) > 0n;
}
export function validateStartContext(
  raw: unknown,
  data: Overview,
  cell: string,
  run: string,
  quantity: string,
) {
  const value = startContextSchema.parse(raw);
  requireMatch(
    sameInstallation(value.installation, data.installation) &&
      value.checked_at.clock_id === value.installation.clock_id &&
      value.cell === cell &&
      value.run.cell === cell &&
      value.run.id === run &&
      value.request.run === run &&
      value.request.budget_limit === quantity &&
      value.request.expected_cell === value.cell_revision &&
      value.request.expected_run === value.run_revision &&
      value.request.envelope_digest === value.envelope.sha256,
  );
  if (value.can_request)
    requireMatch(
      value.run.state === 'PREPARED' &&
        !value.run.pending_attempt &&
        value.run.envelope_digest === value.envelope.sha256 &&
        value.run.recipe_digest === value.recipe.sha256 &&
        BigInt(quantity) <= BigInt(value.maximum_budget) &&
        (!value.run.budget ||
          (value.run.purpose === 'PRODUCTION' &&
            value.run.budget.unit === 'PART_ATTEMPT' &&
            value.run.budget.limit === quantity)),
    );
  return value;
}
export function contextIsCurrent(
  context: StartContext,
  data: Overview,
  cell: CellOverview,
  run: string,
  quantity: string,
) {
  const listed = cell.runs.find((item) => item.value.id === run);
  return (
    sameInstallation(context.installation, data.installation) &&
    context.cell === cell.cell.value.id &&
    context.run.id === run &&
    context.request.budget_limit === quantity &&
    context.cell_revision === cell.cell.revision &&
    context.envelope.sha256 === cell.cell.value.envelope.sha256 &&
    context.recipe.sha256 === cell.cell.value.recipe.sha256 &&
    context.site_config_digest === cell.cell.value.site_config_digest &&
    (!listed ||
      (context.run_revision === listed.revision && listed.value.state === context.run.state))
  );
}
export function startStatusDisplay({
  attempt,
  data,
  cell,
  run,
  id,
  queryFresh,
  queryFailed,
  receivedAt,
  now,
}: {
  attempt: AttemptContext | null;
  data: Overview;
  cell: CellOverview;
  run: string;
  id: string;
  queryFresh: boolean;
  queryFailed: boolean;
  receivedAt: number;
  now: number;
}) {
  const listed = cell.runs.find((item) => item.value.id === run);
  const attemptMatches =
    !!attempt &&
    attempt.attempt.id === id &&
    attempt.attempt.run === run &&
    attempt.run.id === run &&
    attempt.attempt.cell === cell.cell.value.id &&
    attempt.run.cell === cell.cell.value.id;
  // A successful GET remains a historical read as soon as the overview changes.
  // In particular, a retained STARTED receipt cannot override a later Run hold or restart.
  const attemptCurrent =
    !!attempt &&
    attemptMatches &&
    sameInstallation(attempt.installation, data.installation) &&
    !!listed &&
    attempt.run_revision === listed.revision &&
    attempt.run.state === listed.value.state;
  return {
    runState: listed?.value.state ?? 'UNKNOWN',
    attemptMatches,
    attemptCurrent,
    attemptFresh:
      queryFresh && attemptCurrent && !queryFailed && now >= receivedAt && now - receivedAt < 10000,
  };
}
export function freezeStartRequest(
  context: StartContext,
  data: Overview,
  requestKey: string,
): Pending {
  requireMatch(context.can_request && sameInstallation(context.installation, data.installation));
  return pendingSchema.parse({
    request_key: requestKey,
    route: '/api/v1/runs/start',
    command: context.request,
    label: `${context.cell} 실행 ${context.run.id.slice(0, 8)} · 소재 시도 ${context.request.budget_limit} 시작`,
    principal: data.user.principal,
    installation: data.installation.id,
    store_generation: data.installation.store_generation,
    start_review: {
      cell: context.cell,
      epoch: context.epoch,
      scope_epochs: context.scope_epochs,
      recipe_digest: context.recipe.sha256,
      runtime_boot: context.installation.runtime_boot,
    },
  });
}
export function validateStartReceipt(record: Pending, raw: unknown, expectedId?: string) {
  const request = startCommandSchema.parse(record.command);
  const review = record.start_review;
  const value = startAttemptSchema.parse(raw);
  requireMatch(
    record.route === '/api/v1/runs/start' &&
      !!review &&
      value.cell === review.cell &&
      value.run === request.run &&
      value.actor === record.principal &&
      value.expected_cell_revision === request.expected_cell &&
      BigInt(value.expected_run_revision) === BigInt(request.expected_run) + 1n &&
      value.epoch === review.epoch &&
      sameMap(value.scopes, review.scope_epochs) &&
      (!expectedId || value.id === expectedId),
  );
  return value;
}
export function validateAttemptContext(
  raw: unknown,
  data: Overview,
  cell: string,
  run: string,
  id: string,
  record?: Pending,
) {
  const value = attemptContextSchema.parse(raw);
  requireMatch(
    sameInstallation(value.installation, data.installation) &&
      value.checked_at.clock_id === value.installation.clock_id &&
      value.run.cell === cell &&
      value.run.id === run &&
      value.attempt.cell === cell &&
      value.attempt.run === run &&
      value.attempt.id === id,
  );
  const expectedDeadline =
    value.checked_at.clock_id !== value.attempt.valid_until.clock_id
      ? 'CLOCK_CHANGED'
      : BigInt(value.checked_at.ticks_ns) >= BigInt(value.attempt.valid_until.ticks_ns)
        ? 'ELAPSED'
        : 'WITHIN_DEADLINE';
  requireMatch(value.deadline_status === expectedDeadline);
  if (record) {
    validateStartReceipt(record, value.attempt, id);
    const command = startCommandSchema.parse(record.command);
    requireMatch(
      value.run.envelope_digest === command.envelope_digest &&
        value.run.recipe_digest === record.start_review?.recipe_digest &&
        value.run.purpose === command.purpose &&
        value.run.budget?.unit === command.budget_unit &&
        value.run.budget.limit === command.budget_limit,
    );
  }
  return value;
}
const WATCH_KEY = 'rx.start-attempt-browser-watch.v1';
export function saveStartWatch(storage: Pick<Storage, 'setItem'>, watch: StartWatch) {
  validateStartReceipt(watch.request, watch.attempt);
  storage.setItem(WATCH_KEY, JSON.stringify(watch));
}
export function readStartWatch(storage: Pick<Storage, 'getItem'>): StartWatch | null {
  const raw = storage.getItem(WATCH_KEY);
  if (!raw) return null;
  const value = z
    .object({ request: pendingSchema, attempt: startAttemptSchema })
    .parse(JSON.parse(raw));
  validateStartReceipt(value.request, value.attempt);
  return value;
}
export const attemptPath = (cell: string, run: string, id: string) =>
  `/api/v1/run/start-attempt?${new URLSearchParams({ cell, run, id })}`;
