import { describe, it, expect } from 'vitest';
import { canApprove, reviewStamp, validatePackageReceipt, reviewJobSchema } from './package-schema';
import { pendingSchema } from './schema';
const id = '00000000-0000-4000-8000-000000000001',
  hash = 'a'.repeat(64);
const detail = {
  job: { submitted_by: 'author' },
  is_latest: true,
  context_current: true,
  verification: { ready_for_software_approval: true },
  source: { schema: 'rx.process-source.v1', process: 'demo' },
  resolved: { schema: 'rx.resolved-process.v1' },
  activation_authorized: false,
};
describe('software review presentation guards', () => {
  it('requires current material, latest version and a different verifier account', () => {
    expect(canApprove(detail, 'reviewer', ['VERIFIER'], true)).toBe(true);
    for (const [v, actor, roles, fresh] of [
      [detail, 'author', ['VERIFIER'], true],
      [detail, 'reviewer', ['ENGINEER'], true],
      [detail, 'reviewer', ['VERIFIER'], false],
      [{ ...detail, is_latest: false }, 'reviewer', ['VERIFIER'], true],
      [{ ...detail, context_current: false }, 'reviewer', ['VERIFIER'], true],
      [{ ...detail, source: null }, 'reviewer', ['VERIFIER'], true],
    ] as const)
      expect(canApprove(v, actor, [...roles], fresh)).toBe(false);
  });
  it('invalidates acknowledgement when displayed material changes even under an unchanged server digest', () => {
    expect(reviewStamp({ ...detail, source: { process: 'changed' } })).not.toBe(
      reviewStamp(detail),
    );
  });
  it('correlates decision results exactly and preserves large integer revisions', () => {
    const command = {
      review: id,
      cell: 'cell/a',
      report_revision: '9007199254740993',
      review_digest: hash,
      expected: '9007199254740992',
      choice: 'APPROVE',
      note: 'reviewed',
    };
    const response = {
      ...command,
      revision: '9007199254740993',
      report_digest: hash,
      decided_by: 'reviewer',
      decided_at: { clock_id: 'clock', ticks_ns: '1' },
      scope: 'PROCESS_PACKAGE_SOFTWARE',
    };
    expect(() =>
      validatePackageReceipt('/api/v1/process-review/decisions', command, response),
    ).not.toThrow();
    for (const changed of [
      { ...response, cell: 'cell/b' },
      { ...response, choice: 'REJECT' },
      { ...response, review_digest: 'b'.repeat(64) },
      { ...response, revision: '9007199254740992' },
    ])
      expect(() =>
        validatePackageReceipt('/api/v1/process-review/decisions', command, changed),
      ).toThrow();
    expect(
      pendingSchema.parse({
        request_key: id,
        route: '/api/v1/process-review/decisions',
        command,
        label: 'review',
        principal: 'reviewer',
        installation: id,
        store_generation: id,
      }).command,
    ).toEqual(command);
  });
});

describe('device-backed process review request', () => {
  it('preserves the candidate context binding and rejects schema confusion', () => {
    const value = {
      request: {
        schema: 'rx.process-review-request.v2',
        id,
        intake: id,
        cell: 'cell/demo',
        package_manifest: hash,
        package_signature: hash,
        configuration_digest: hash,
        package_policy_fingerprint: hash,
        package_policy_file_digest: hash,
        verification_authority_digest: hash,
        binding_selections: { load: 'robot/supply' },
        device_context_digest: hash,
      },
      configuration: {},
      device_context: { catalog_digest: hash, dependencies: [{ plan: { id } }] },
      submitted_by: 'author',
      requested_by: 'author',
      created_at: { clock_id: 'test', ticks_ns: '1' },
    };
    expect(reviewJobSchema.parse(value).request.device_context_digest).toBe(hash);
    expect(reviewJobSchema.parse(value).device_context).toEqual(value.device_context);
    expect(reviewJobSchema.safeParse({ ...value, device_context: undefined }).success).toBe(false);
    expect(
      reviewJobSchema.safeParse({
        ...value,
        request: { ...value.request, schema: 'rx.process-review-request.v1' },
      }).success,
    ).toBe(false);
    const { device_context_digest: omitted, ...request } = value.request;
    expect(omitted).toBe(hash);
    expect(reviewJobSchema.safeParse({ ...value, request }).success).toBe(false);
  });
});
