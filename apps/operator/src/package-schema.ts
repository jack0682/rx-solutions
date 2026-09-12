import { z } from 'zod';
import { stableDocument } from './draft-schema';
import {
  deviceReviewRoutes,
  validateDeviceReceipt,
  emptyDeviceReviewBuffer,
  type DeviceReviewBuffer,
} from './device-review-schema';
export const digest = z.string().regex(/^[0-9a-f]{64}$/);
export const count = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((v) => BigInt(v) <= 18446744073709551615n);
const id = z.uuid();
const time = z.object({ clock_id: z.string(), ticks_ns: count });
const artifact = z.object({ sha256: digest, schema_id: z.string(), size_bytes: count });
const object = z.object({ manifest: digest, signature: digest });
const registration = z.object({
  generation: id,
  store_owner: id,
  policy_fingerprint: digest,
  policy_file_digest: digest,
});
export const intakeContextSchema = z.object({
  device_review_authority_digest: digest.nullable().optional().default(null),
  cell: z.string(),
  configuration_digest: digest,
  registration: registration.nullable(),
  review_authority_digest: digest.nullable(),
});
export const intakeSchema = z.object({
  device_catalog: artifact.optional(),
  id,
  cell: z.string(),
  title: z.string(),
  object,
  configuration_digest: digest,
  registration,
  submitted_by: z.string(),
  terminal: z.string().nullable(),
  submitted_at: time,
  state: z.literal('AWAITING_REVIEW'),
  manifest: z
    .object({
      package: z.string(),
      version: z.string(),
      publisher: z.string(),
      entry: z
        .object({ kind: z.enum(['PROCESS', 'DEVICE', 'DEVICE_REFERENCE', 'UI']) })
        .passthrough(),
      permissions: z.array(
        z.object({ kind: z.string(), operation: z.string().optional() }).passthrough(),
      ),
    })
    .passthrough(),
});
export const intakeView = z.object({
  receipt: intakeSchema,
  review_context_current: z.boolean(),
  content_reverification_required: z.literal(true),
  activation_authorized: z.literal(false),
});
export const intakePageSchema = z.object({
  cell: z.string(),
  drafts: z.never().optional(),
  packages: z.array(intakeView),
  next: id.nullable(),
});
const reviewRequest = z
  .object({
    schema: z.enum(['rx.process-review-request.v1', 'rx.process-review-request.v2']),
    device_context_digest: digest.optional(),
    id,
    intake: id,
    cell: z.string(),
    package_manifest: digest,
    package_signature: digest,
    configuration_digest: digest,
    package_policy_fingerprint: digest,
    package_policy_file_digest: digest,
    verification_authority_digest: digest,
    binding_selections: z.record(z.string(), z.string()),
  })
  .refine(
    (v) =>
      (v.schema === 'rx.process-review-request.v2') === (v.device_context_digest !== undefined),
    'device context/schema mismatch',
  );
export const reviewJobSchema = z
  .object({
    device_context: z
      .object({
        catalog_digest: digest,
        dependencies: z.array(z.record(z.string(), z.unknown())).min(1).max(16),
      })
      .optional(),
    request: reviewRequest,
    configuration: z.record(z.string(), z.unknown()),
    submitted_by: z.string(),
    requested_by: z.string(),
    created_at: time,
  })
  .refine(
    (v) => (v.request.device_context_digest !== undefined) === (v.device_context !== undefined),
    'device review material missing or unexpected',
  );
const issue = z.object({ code: z.string(), location: z.string(), detail: z.string() });
const report = z.object({
  schema: z.literal('rx.process-verification-report.v1'),
  request: reviewRequest,
  validator_digest: digest,
  validator_policy_file_digest: digest,
  resolved: artifact.nullable(),
  issues: z.array(issue),
});
export const reviewVersionSchema = z.object({
  review: id,
  cell: z.string(),
  revision: count,
  report_digest: digest,
  review_digest: digest,
  checker_digest: digest,
  report,
  signature: z.object({ key: z.string(), signature: z.string().regex(/^[0-9a-f]{128}$/) }),
  platform_issues: z.array(issue),
  source: artifact.nullable(),
  resolved: artifact.nullable(),
  ready_for_software_approval: z.boolean(),
  recorded_by: z.string(),
  recorded_at: time,
});
export const reviewDecisionSchema = z.object({
  review: id,
  cell: z.string(),
  revision: count,
  report_revision: count,
  report_digest: digest,
  review_digest: digest,
  choice: z.enum(['APPROVE', 'REJECT']),
  note: z.string(),
  decided_by: z.string(),
  decided_at: time,
  scope: z.literal('PROCESS_PACKAGE_SOFTWARE'),
});
export const reviewDetailSchema = z.object({
  job: reviewJobSchema,
  verification: reviewVersionSchema.nullable(),
  decision: reviewDecisionSchema.nullable(),
  source: z.record(z.string(), z.unknown()).nullable(),
  resolved: z.record(z.string(), z.unknown()).nullable(),
  context_current: z.boolean(),
  activation_authorized: z.literal(false),
  approval_matches_current_review: z.boolean(),
  latest_report_revision: count.nullable(),
  is_latest: z.boolean(),
});
export const reviewPageSchema = z.object({
  cell: z.string(),
  intake: id,
  reviews: z.array(
    z.object({
      id,
      intake: id,
      requested_by: z.string(),
      created_at: time,
      report_revision: count.nullable(),
      ready_for_software_approval: z.boolean(),
      decision: reviewDecisionSchema.nullable(),
    }),
  ),
  next: id.nullable(),
});
export type Intake = z.infer<typeof intakeSchema>;
export type ReviewDetail = z.infer<typeof reviewDetailSchema>;
export type ReviewReceipt = { route: string; value: unknown; key: string };
export const packageRoutes = [
  '/api/v1/package-intakes',
  '/api/v1/process-reviews',
  '/api/v1/process-review/reports',
  '/api/v1/process-review/decisions',
  ...deviceReviewRoutes,
] as const;
export function packageRoute(route: string) {
  return (packageRoutes as readonly string[]).includes(route);
}
export function validatePackageReceipt(
  route: string,
  command: Record<string, unknown>,
  result: unknown,
): unknown {
  if (route === packageRoutes[0]) {
    const v = intakeSchema.parse(result);
    if (
      v.id !== command.id ||
      v.cell !== command.cell ||
      v.title !== command.title ||
      stableDocument(v.object) !== stableDocument(command.object) ||
      v.configuration_digest !== command.configuration_digest ||
      v.registration.generation !== command.policy_generation
    )
      throw Error('intake correlation');
    return v;
  }
  if (route === packageRoutes[1]) {
    const v = reviewJobSchema.parse(result);
    if (
      v.request.id !== command.id ||
      v.request.intake !== command.intake ||
      v.request.cell !== command.cell ||
      v.request.configuration_digest !== command.configuration_digest ||
      stableDocument(v.request.binding_selections) !== stableDocument(command.binding_selections)
    )
      throw Error('review correlation');
    return v;
  }
  if (route === packageRoutes[2]) {
    const v = reviewVersionSchema.parse(result);
    if (
      v.review !== command.review ||
      v.cell !== command.cell ||
      v.report_digest !== command.report_digest ||
      BigInt(v.revision) !== BigInt((command.expected as string | null) ?? '0') + 1n
    )
      throw Error('report correlation');
    return v;
  }
  if (route === packageRoutes[3]) {
    const v = reviewDecisionSchema.parse(result);
    if (
      v.review !== command.review ||
      v.cell !== command.cell ||
      v.report_revision !== command.report_revision ||
      v.review_digest !== command.review_digest ||
      v.choice !== command.choice ||
      v.note !== command.note ||
      BigInt(v.revision) !== BigInt((command.expected as string | null) ?? '0') + 1n
    )
      throw Error('decision correlation');
    return v;
  }
  return validateDeviceReceipt(route, command, result);
}
export function reviewStamp(v: unknown): string {
  return stableDocument(v);
}
export function canApprove(
  v: {
    job: { submitted_by: string };
    is_latest: boolean;
    context_current: boolean;
    verification: { ready_for_software_approval: boolean } | null;
    source: unknown;
    resolved: unknown;
  },
  principal: string,
  roles: string[],
  fresh: boolean,
): boolean {
  return (
    fresh &&
    roles.includes('VERIFIER') &&
    principal !== v.job.submitted_by &&
    v.is_latest &&
    v.context_current &&
    !!v.verification?.ready_for_software_approval &&
    !!v.source &&
    !!v.resolved
  );
}
export type PackageBuffer = {
  device: DeviceReviewBuffer;
  selectedIntake: string;
  selectedReview: string;
  path: string;
  title: string;
  manifest: string;
  signature: string;
  selections: Record<string, string>;
  reportPath: string;
  reportDigest: string;
  note: string;
};
export const emptyPackageBuffer = (): PackageBuffer => ({
  device: emptyDeviceReviewBuffer(),
  selectedIntake: '',
  selectedReview: '',
  path: '',
  title: '',
  manifest: '',
  signature: '',
  selections: {},
  reportPath: '',
  reportDigest: '',
  note: '',
});
export function downloadJson(value: unknown, name: string) {
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(value, null, 2)], { type: 'application/json' }),
  );
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
