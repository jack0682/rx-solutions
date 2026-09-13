import { diagnosticsSchema } from './diagnostics-schema';
import { z } from './schema-runtime';
export const counter = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((s) => /^(0|[1-9][0-9]{0,19})$/.test(s) && BigInt(s) <= 18446744073709551615n);
const id = z.uuid();
export const timeSchema = z.object({ clock_id: z.string(), ticks_ns: counter });
const time = timeSchema;
export const artifactSchema = z.object({
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
  schema_id: z.string(),
  size_bytes: counter,
});
const artifact = artifactSchema;
export const installationSchema = z.object({
  id,
  store_generation: id,
  runtime_boot: id,
  clock_id: z.string(),
});
const environment = z.enum(['SIMULATION', 'PHYSICAL']);
export const profileSchema = z.object({
  principal: z.string(),
  terminal: z.string().nullable().optional(),
  roles: z.array(
    z.enum([
      'OBSERVER',
      'OPERATOR',
      'RECOVERY_LEAD',
      'ENGINEER',
      'VERIFIER',
      'RELEASE_MANAGER',
      'ACCOUNT_ADMIN',
      'EXECUTOR',
      'HOST',
      'OPERATOR_API',
    ]),
  ),
  cells: z.array(z.string()),
  expires_at: time,
});
const block = z.object({
  id,
  reason: z.enum([
    'CONFIGURATION_CHANGE',
    'RUNTIME_RESTART',
    'OPERATOR_HOLD',
    'EXECUTOR_PAUSE',
    'CASE_DIAGNOSTIC',
    'CASE_INTERVENTION',
    'PROCEDURE_REPORTED',
    'OUT_OF_SERVICE',
    'AUTHORITY_REVOKED',
    'CONDITION_LOST',
    'INTEGRITY_CONFLICT',
    'DEVICE_RESTART',
  ]),
  latched: z.boolean(),
  scopes: z.array(z.string()),
});
const cell = z.object({
  id: z.string(),
  mode: z.enum(['SETUP', 'AUTOMATIC', 'RECOVERY', 'MAINTENANCE']).nullable().optional(),
  commissioning: z
    .enum(['NOT_COMMISSIONED', 'COMMISSIONED', 'REVALIDATION_REQUIRED'])
    .nullable()
    .optional(),
  environment,
  epoch: counter,
  definition: artifact,
  envelope: artifact,
  recipe: artifact,
  site_config_digest: z.string(),
  hosts: z.array(z.string()),
  blocks: z.array(block),
  open_cases: z.array(id),
  qualification: z
    .object({ id, revision: counter, environment, envelope_digest: z.string() })
    .nullable(),
});
const budget = z.object({
  unit: z.enum(['PART_ATTEMPT', 'OPERATION_COUNT']),
  limit: counter,
  revision: counter,
  consumptions: z.array(z.union([z.object({ PART_ATTEMPT: id }), z.object({ OPERATION: id })])),
});
export const runSchema = z.object({
  id,
  cell: z.string(),
  state: z.enum(['PREPARED', 'EXECUTING', 'PAUSED', 'RECOVERY_REQUIRED', 'COMPLETED', 'ABANDONED']),
  purpose: z.enum(['PRODUCTION', 'SETUP']).nullable(),
  budget: budget.nullable(),
  recipe_digest: z.string(),
  envelope_digest: z.string(),
  part_ids: z.array(id),
  pending_attempt: id.nullable().optional(),
});
const operation = z.object({
  operation_id: id,
  revision: counter,
  phase: z.enum(['ADMITTED', 'ACTIVE', 'RECONCILING', 'SETTLED']),
  execution_knowledge: z.enum([
    'NOT_SENT',
    'MAY_HAVE_EXECUTED',
    'ACCEPTED',
    'RUNNING',
    'ENDED',
    'UNKNOWN',
  ]),
  outcome: z.enum(['NONE', 'SUCCEEDED', 'FAILED', 'CANCELED', 'NOT_EXECUTED', 'UNRESOLVED']),
  integrity: z.enum(['VALID', 'DISPUTED']),
  disposition: z.enum(['HELD', 'QUARANTINED', 'RELEASED']),
  evidence_ids: z.array(id),
});
export const overviewSchema = z.object({
  snapshot_id: id,
  observed_at: time,
  user: profileSchema,
  installation: installationSchema,
  cells: z.array(
    z.object({
      cell: z.object({ revision: counter, value: cell }),
      diagnostics: diagnosticsSchema,
      runs: z.array(z.object({ revision: counter, value: runSchema })),
      work: z.array(
        z.object({ cell: z.string(), run: id, part: id.nullable(), host: z.string(), operation }),
      ),
      runs_truncated: z.boolean(),
      work_truncated: z.boolean(),
    }),
  ),
});
export type Overview = z.infer<typeof overviewSchema>;
export type CellOverview = Overview['cells'][number];
export type Profile = z.infer<typeof profileSchema>;
export const startCommandSchema = z
  .object({
    run: id,
    envelope_digest: z.string().regex(/^[0-9a-f]{64}$/),
    purpose: z.literal('PRODUCTION'),
    budget_unit: z.literal('PART_ATTEMPT'),
    budget_limit: counter.refine((value) => /^[0-9]+$/.test(value) && BigInt(value) > 0n),
    expected_cell: counter,
    expected_run: counter.refine(
      (value) => /^[0-9]+$/.test(value) && BigInt(value) < 18446744073709551615n,
    ),
  })
  .strict();
export const startReviewSchema = z
  .object({
    cell: z.string().min(1),
    epoch: counter,
    scope_epochs: z.record(z.string(), counter),
    recipe_digest: z.string().regex(/^[0-9a-f]{64}$/),
    runtime_boot: id,
  })
  .strict();
const recoveryDigest = z.string().regex(/^[0-9a-f]{64}$/);
export const recoveryPrepareSchema = z
  .object({
    host: z.string().min(1),
    origin: z.string().min(1),
    expected_context: recoveryDigest,
    expected_cells: z.record(z.string(), counter),
  })
  .strict();
export const recoveryApproveSchema = z
  .object({
    id,
    expected_revision: counter,
    proposal_digest: recoveryDigest,
    expected_cells: z.record(z.string(), counter),
  })
  .strict();
export const recoveryReviewSchema = z
  .object({
    host: z.string().min(1),
    origin: z.string().min(1),
    runtime_boot: id,
    context_digest: recoveryDigest,
    binding: id.optional(),
    proposal_digest: recoveryDigest.optional(),
    fence_requests: z.record(z.string(), id).optional(),
    fence_payloads: z.record(z.string(), recoveryDigest).optional(),
  })
  .strict();
export const pendingSchema = z
  .object({
    request_key: id,
    route: z.enum([
      '/api/v1/runs',
      '/api/v1/runs/start',
      '/api/v1/cells/hold',
      '/api/v1/cells',
      '/api/v1/cases/acknowledge',
      '/api/v1/process-drafts',
      '/api/v1/process-draft-bindings',
      '/api/v1/package-intakes',
      '/api/v1/process-reviews',
      '/api/v1/process-review/reports',
      '/api/v1/process-review/decisions',
      '/api/v1/device-reviews',
      '/api/v1/device-review/reports',
      '/api/v1/device-review/decisions',
      '/api/v1/host-recoveries',
      '/api/v1/host-recovery/approve',
    ]),
    command: z.record(z.string(), z.unknown()),
    label: z.string(),
    principal: z.string(),
    installation: id,
    store_generation: id,
    start_review: startReviewSchema.optional(),
    recovery_review: recoveryReviewSchema.optional(),
  })
  .superRefine((value, context) => {
    if (
      value.route === '/api/v1/host-recoveries' ||
      value.route === '/api/v1/host-recovery/approve'
    ) {
      const review = value.recovery_review;
      const command =
        value.route === '/api/v1/host-recoveries'
          ? recoveryPrepareSchema.safeParse(value.command)
          : recoveryApproveSchema.safeParse(value.command);
      if (
        !review ||
        !command.success ||
        (value.route === '/api/v1/host-recoveries' &&
          (value.command.host !== review.host ||
            value.command.origin !== review.origin ||
            value.command.expected_context !== review.context_digest)) ||
        (value.route === '/api/v1/host-recovery/approve' &&
          (value.command.id !== review.binding ||
            value.command.proposal_digest !== review.proposal_digest ||
            !review.fence_requests ||
            !review.fence_payloads))
      )
        context.addIssue({
          code: 'custom',
          message: 'Recovery request and reviewed Host context differ.',
        });
    }
    if (
      value.route === '/api/v1/runs/start' &&
      (!value.start_review || !startCommandSchema.safeParse(value.command).success)
    ) {
      context.addIssue({
        code: 'custom',
        message: 'A start request requires its exact command and reviewed cell context.',
      });
    }
  });
export type Pending = z.infer<typeof pendingSchema>;

export const caseSnapshotSchema = z.object({
  acknowledgment_count: counter.default('0'),
  procedure_record_count: counter.default('0'),
  revision: counter,
  case: z.object({
    id,
    cell: z.string(),
    kind: z.enum([
      'DIAGNOSTIC_ONLY',
      'PLANNED_ACCESS',
      'FAULT_RECOVERY',
      'MAINTENANCE',
      'CHANGE_REVIEW',
    ]),
    state: z.enum([
      'OPEN',
      'CONTAINMENT_PENDING',
      'PROCEDURE_ACTIVE',
      'REVALIDATING',
      'READY_FOR_RESTART',
      'CLOSED',
      'ESCALATED',
    ]),
    lead: z.string(),
    participants: z.array(z.string()),
    operation_ids: z.array(id),
    material_ids: z.array(id),
    record_ids: z.array(id),
    block_ids: z.array(id),
    effective_cells: z.array(z.string()),
    scope_uncertain: z.boolean(),
    procedure: artifact,
  }),
});
export const caseListSchema = z.object({
  cell: z.string(),
  cases: z.array(caseSnapshotSchema),
  truncated: z.boolean(),
});
export const caseDetailSchema = z.object({
  snapshot: caseSnapshotSchema,
  acknowledgments: z.array(
    z.object({
      id,
      actor: z.string(),
      occurred_at: z.string(),
      case_revision: counter,
      assertions: artifact,
    }),
  ),
});
export type CaseSnapshot = z.infer<typeof caseSnapshotSchema>;
