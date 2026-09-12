import { describe, it, expect } from 'vitest';
import {
  canApproveDevice,
  deviceDetailSchema,
  devicePageSchema,
  deviceReviewStamp,
  validateDeviceReceipt,
} from './device-review-schema';
import { pendingSchema } from './schema';
import { requestBody } from './pending';
const id = '00000000-0000-4000-8000-000000000001',
  h = 'a'.repeat(64),
  time = { clock_id: 'clock', ticks_ns: '1' };
const request = {
  schema: 'rx.device-review-request.v1',
  id,
  intake: id,
  installation: id,
  cell: 'cell/a',
  package_manifest: h,
  package_signature: h,
  catalog: { sha256: h, schema_id: 'rx.device-operation-catalog.v1', size_bytes: '1' },
  configuration_digest: h,
  package_policy_fingerprint: h,
  package_policy_file_digest: h,
  verification_authority_digest: h,
};
const report = {
  schema: 'rx.device-verification-report.v1',
  request,
  validator_digest: h,
  validator_policy_file_digest: h,
  scope: 'DEVICE_PACKAGE_SOFTWARE',
  checks: {
    CONTENT_SIGNATURE: 'PASSED',
    DEVICE_SOURCE_CONSISTENCY: 'PASSED',
    CATALOG_REQUEST_BINDING: 'PASSED',
  },
  issues: [],
};
const version = {
  review: id,
  cell: 'cell/a',
  revision: '1',
  review_digest: h,
  checker_digest: h,
  report_digest: h,
  report,
  signature: { key: 'verifier', signature: 'ab' },
  ready_for_software_approval: true,
  recorded_by: 'author',
  recorded_at: time,
};
const raw = {
  job: {
    request,
    registration: { generation: id, store_owner: id, policy_fingerprint: h, policy_file_digest: h },
    submitted_by: 'author',
    requested_by: 'author',
    created_at: time,
  },
  version,
  decision: null,
  latest_report_revision: '1',
  is_latest: true,
  context_current: true,
  approval_matches_current_review: false,
  activation_authorized: false,
};
describe('device review target and recovery boundaries', () => {
  it('only enables approval for an independent verifier and fresh latest material', () => {
    const d = deviceDetailSchema.parse(raw);
    expect(canApproveDevice(d, 'reviewer', ['VERIFIER'], true)).toBe(true);
    for (const [value, actor, roles, fresh] of [
      [d, 'author', ['VERIFIER'], true],
      [d, 'reviewer', ['ENGINEER'], true],
      [d, 'reviewer', ['VERIFIER'], false],
      [{ ...d, is_latest: false }, 'reviewer', ['VERIFIER'], true],
      [{ ...d, context_current: false }, 'reviewer', ['VERIFIER'], true],
    ] as const)
      expect(canApproveDevice(value, actor, [...roles], fresh)).toBe(false);
  });
  it('rejects forged scope, mixed requests and inconsistent success flags', () => {
    for (const changed of [
      { ...raw, activation_authorized: true },
      { ...raw, version: { ...version, report: { ...report, scope: 'PHYSICAL_QUALIFICATION' } } },
      {
        ...raw,
        version: {
          ...version,
          report: { ...report, request: { ...request, package_manifest: 'b'.repeat(64) } },
        },
      },
      {
        ...raw,
        version: {
          ...version,
          report: { ...report, checks: { ...report.checks, DEVICE_SOURCE_CONSISTENCY: 'FAILED' } },
        },
      },
      { ...raw, approval_matches_current_review: true },
    ])
      expect(deviceDetailSchema.safeParse(changed).success).toBe(false);
  });
  it('invalidates acknowledgement for changed report content or context, not unchanged polling', () => {
    const d = deviceDetailSchema.parse(raw),
      stamp = deviceReviewStamp(d, { policy: h });
    expect(deviceReviewStamp(structuredClone(d), { policy: h })).toBe(stamp);
    expect(
      deviceReviewStamp(
        { ...d, version: { ...d.version!, signature: { key: 'other', signature: 'ab' } } },
        { policy: h },
      ),
    ).not.toBe(stamp);
    expect(deviceReviewStamp(d, { policy: 'b'.repeat(64) })).not.toBe(stamp);
  });
  it('recovers exact decisions with large string revisions and rejects mismatched receipts', () => {
    const command = {
      review: id,
      cell: 'cell/a',
      report_revision: '9007199254740993',
      review_digest: h,
      expected: '9007199254740992',
      choice: 'APPROVE',
      note: 'checked software scope',
    };
    const decision = {
      review: command.review,
      cell: command.cell,
      report_revision: command.report_revision,
      review_digest: command.review_digest,
      choice: command.choice,
      note: command.note,
      revision: '9007199254740993',
      report_digest: h,
      decided_by: 'reviewer',
      decided_at: time,
      scope: 'DEVICE_PACKAGE_SOFTWARE',
    };
    expect(() =>
      validateDeviceReceipt('/api/v1/device-review/decisions', command, decision),
    ).not.toThrow();
    for (const v of [
      { ...decision, cell: 'cell/b' },
      { ...decision, note: 'changed' },
      { ...decision, report_revision: '1' },
      { ...decision, revision: '9007199254740992' },
      { ...decision, scope: 'PROCESS_PACKAGE_SOFTWARE' },
    ])
      expect(() => validateDeviceReceipt('/api/v1/device-review/decisions', command, v)).toThrow();
    const p = pendingSchema.parse({
      route: '/api/v1/device-review/decisions',
      request_key: id,
      command,
      label: 'device approval',
      principal: 'reviewer',
      installation: id,
      store_generation: id,
    });
    expect(requestBody(p)).toEqual({ request_key: id, command });
  });
  it('accepts compact pages and rejects report bodies in list rows', () => {
    const p = {
      cell: 'cell/a',
      intake: id,
      reviews: [
        {
          id,
          requested_by: 'author',
          created_at: time,
          report_revision: '1',
          report_digest: h,
          ready_for_software_approval: true,
          context_current: true,
          decision_revision: null,
          choice: null,
          approval_matches_current_review: false,
        },
      ],
      next: null,
    };
    expect(devicePageSchema.safeParse(p).success).toBe(true);
    expect(
      devicePageSchema.safeParse({ ...p, reviews: [{ ...p.reviews[0], report }] }).success,
    ).toBe(false);
  });
});
