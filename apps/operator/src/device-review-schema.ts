import { z } from 'zod';
import { stableDocument } from './draft-schema';
const id = z.uuid(),
  digest = z.string().regex(/^[0-9a-f]{64}$/);
const count = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((v) => BigInt(v) <= 18446744073709551615n);
const time = z.object({ clock_id: z.string(), ticks_ns: count });
const artifact = z
  .object({
    sha256: digest,
    schema_id: z.literal('rx.device-operation-catalog.v1'),
    size_bytes: count,
  })
  .strict();
const registration = z
  .object({
    generation: id,
    store_owner: id,
    policy_fingerprint: digest,
    policy_file_digest: digest,
  })
  .strict();
const request = z
  .object({
    schema: z.literal('rx.device-review-request.v1'),
    id,
    intake: id,
    installation: id,
    cell: z.string(),
    package_manifest: digest,
    package_signature: digest,
    catalog: artifact,
    configuration_digest: digest,
    package_policy_fingerprint: digest,
    package_policy_file_digest: digest,
    verification_authority_digest: digest,
  })
  .strict();
const result = z.enum(['PASSED', 'FAILED', 'NOT_PERFORMED']);
const report = z
  .object({
    schema: z.literal('rx.device-verification-report.v1'),
    request,
    validator_digest: digest,
    validator_policy_file_digest: digest,
    scope: z.literal('DEVICE_PACKAGE_SOFTWARE'),
    checks: z
      .object({
        CONTENT_SIGNATURE: result,
        DEVICE_SOURCE_CONSISTENCY: result,
        CATALOG_REQUEST_BINDING: result,
      })
      .strict(),
    issues: z
      .array(z.object({ code: z.string(), location: z.string(), detail: z.string() }).strict())
      .max(32),
  })
  .strict();
export const deviceJobSchema = z
  .object({
    request,
    registration,
    submitted_by: z.string(),
    requested_by: z.string(),
    created_at: time,
  })
  .strict();
export const deviceVersionSchema = z
  .object({
    review: id,
    cell: z.string(),
    revision: count,
    review_digest: digest,
    checker_digest: digest,
    report_digest: digest,
    report,
    signature: z.object({ key: z.string(), signature: z.string() }),
    ready_for_software_approval: z.boolean(),
    recorded_by: z.string(),
    recorded_at: time,
  })
  .strict()
  .superRefine((v, c) => {
    if (
      v.review !== v.report.request.id ||
      v.cell !== v.report.request.cell ||
      v.ready_for_software_approval !==
        (v.report.issues.length === 0 &&
          Object.values(v.report.checks).every((x) => x === 'PASSED'))
    )
      c.addIssue({ code: 'custom', message: 'device report correlation' });
  });
export const deviceDecisionSchema = z
  .object({
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
    scope: z.literal('DEVICE_PACKAGE_SOFTWARE'),
  })
  .strict();
export const deviceDetailSchema = z
  .object({
    job: deviceJobSchema,
    version: deviceVersionSchema.nullable(),
    decision: deviceDecisionSchema.nullable(),
    latest_report_revision: count.nullable(),
    is_latest: z.boolean(),
    context_current: z.boolean(),
    approval_matches_current_review: z.boolean(),
    activation_authorized: z.literal(false),
  })
  .strict()
  .superRefine((v, c) => {
    const r = v.version,
      d = v.decision,
      j = v.job.request;
    if (
      (r &&
        (r.review !== j.id ||
          r.cell !== j.cell ||
          stableDocument(r.report.request) !== stableDocument(j))) ||
      (d && (d.review !== j.id || d.cell !== j.cell)) ||
      v.is_latest !== ((r?.revision ?? null) === v.latest_report_revision) ||
      (v.approval_matches_current_review &&
        (!r ||
          !d ||
          !v.context_current ||
          !v.is_latest ||
          !r.ready_for_software_approval ||
          d.choice !== 'APPROVE' ||
          d.review_digest !== r.review_digest ||
          d.report_revision !== r.revision ||
          d.report_digest !== r.report_digest))
    )
      c.addIssue({ code: 'custom', message: 'device review detail correlation' });
  });
export const devicePageSchema = z
  .object({
    cell: z.string(),
    intake: id,
    reviews: z
      .array(
        z
          .object({
            id,
            requested_by: z.string(),
            created_at: time,
            report_revision: count.nullable(),
            report_digest: digest.nullable(),
            ready_for_software_approval: z.boolean(),
            context_current: z.boolean(),
            decision_revision: count.nullable(),
            choice: z.enum(['APPROVE', 'REJECT']).nullable(),
            approval_matches_current_review: z.boolean(),
          })
          .strict(),
      )
      .max(50),
    next: id.nullable(),
  })
  .strict();
export type DeviceReviewDetail = z.infer<typeof deviceDetailSchema>;
export type DeviceReviewPage = z.infer<typeof devicePageSchema>;
export const deviceReviewRoutes = [
  '/api/v1/device-reviews',
  '/api/v1/device-review/reports',
  '/api/v1/device-review/decisions',
] as const;
export function validateDeviceReceipt(
  route: string,
  command: Record<string, unknown>,
  raw: unknown,
): unknown {
  if (route === deviceReviewRoutes[0]) {
    const v = deviceJobSchema.parse(raw);
    if (
      v.request.id !== command.id ||
      v.request.intake !== command.intake ||
      v.request.cell !== command.cell ||
      v.request.configuration_digest !== command.configuration_digest ||
      v.registration.generation !== command.policy_generation
    )
      throw Error('device job receipt differs');
    return v;
  }
  if (route === deviceReviewRoutes[1]) {
    const v = deviceVersionSchema.parse(raw);
    if (
      v.review !== command.review ||
      v.cell !== command.cell ||
      v.report_digest !== command.report_digest ||
      BigInt(v.revision) !== BigInt((command.expected as string | null) ?? '0') + 1n
    )
      throw Error('device report receipt differs');
    return v;
  }
  if (route === deviceReviewRoutes[2]) {
    const v = deviceDecisionSchema.parse(raw);
    if (
      v.review !== command.review ||
      v.cell !== command.cell ||
      v.report_revision !== command.report_revision ||
      v.review_digest !== command.review_digest ||
      v.choice !== command.choice ||
      v.note !== command.note ||
      BigInt(v.revision) !== BigInt((command.expected as string | null) ?? '0') + 1n
    )
      throw Error('device decision receipt differs');
    return v;
  }
  throw Error('unsupported device route');
}
export function deviceReviewStamp(detail: DeviceReviewDetail, context: unknown): string {
  return stableDocument({ detail, context });
}
export function canApproveDevice(
  detail: DeviceReviewDetail,
  principal: string,
  roles: string[],
  fresh: boolean,
): boolean {
  return (
    fresh &&
    roles.includes('VERIFIER') &&
    principal !== detail.job.submitted_by &&
    detail.is_latest &&
    detail.context_current &&
    !!detail.version?.ready_for_software_approval &&
    !detail.activation_authorized
  );
}
export type DeviceReviewBuffer = {
  selectedReview: string;
  reportPath: string;
  reportDigest: string;
  note: string;
};
export const emptyDeviceReviewBuffer = (): DeviceReviewBuffer => ({
  selectedReview: '',
  reportPath: '',
  reportDigest: '',
  note: '',
});
