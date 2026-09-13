import { describe, it, expect } from 'vitest';
import { intakeSchema } from './package-schema';
import { correlateDeviceCatalog, deviceCatalogSchema } from './device-catalog';
const id = '00000000-0000-4000-8000-000000000001',
  hash = 'a'.repeat(64);
const reference = { sha256: hash, schema_id: 'rx.device-operation-catalog.v1', size_bytes: '1' };
const object = { manifest: hash, signature: hash };
const intake = {
  id,
  cell: 'cell/a',
  title: 'Device',
  object,
  device_catalog: reference,
  configuration_digest: hash,
  registration: {
    generation: id,
    store_owner: id,
    policy_fingerprint: hash,
    policy_file_digest: hash,
  },
  submitted_by: 'author',
  terminal: null,
  submitted_at: { clock_id: 'test', ticks_ns: '1' },
  state: 'AWAITING_REVIEW',
  manifest: {
    package: 'device/a',
    version: '1.0.0',
    publisher: 'test',
    entry: { kind: 'DEVICE_REFERENCE' },
    permissions: [],
  },
};
const detail = {
  cell: 'cell/a',
  intake: id,
  object,
  reference,
  review_context_current: true,
  content_reverification_required: true,
  manufacturer_validation_required: true,
  activation_authorized: false,
  catalog: {
    schema: 'rx.device-operation-catalog.v1',
    installation: id,
    cell: 'cell/a',
    target: 'robot/arm',
    environment: 'SIMULATION',
    profile_digest: hash,
    condition_ids: ['ready'],
    operations: {
      load: {
        kind: 'FINITE_ACTION',
        target: 'robot/arm',
        profile_digest: hash,
        resource_set: ['robot/controller'],
        execution_timeout_ms: '1000',
        completion_rule: 'terminal',
      },
    },
    outcomes: {
      schema: 'rx.native-outcome-table.v1',
      profile_digest: hash,
      completion_rule: 'terminal',
      cases: [{ status_schema: 'native/done', statuses: ['0'], conclusion: 'SUCCEEDED' }],
    },
    documents: {},
  },
};
describe('device declarations remain scoped review data', () => {
  it('recognizes device references and never requires new fields on historical receipts', () => {
    const current = intakeSchema.parse(intake);
    expect(correlateDeviceCatalog(detail, current).activation_authorized).toBe(false);
    const { device_catalog: _, ...old } = intake;
    expect(intakeSchema.parse(old).device_catalog).toBeUndefined();
  });
  it('rejects another receipt, object, reference or declared cell', () => {
    const current = intakeSchema.parse(intake);
    for (const changed of [
      { ...detail, cell: 'cell/b' },
      { ...detail, intake: '00000000-0000-4000-8000-000000000002' },
      { ...detail, object: { ...object, manifest: 'b'.repeat(64) } },
      { ...detail, reference: { ...reference, size_bytes: '2' } },
      { ...detail, catalog: { ...detail.catalog, cell: 'cell/b' } },
      { ...detail, catalog: null },
    ])
      expect(() => correlateDeviceCatalog(changed, current)).toThrow();
  });
  it('cannot present a declaration as activated or device validated', () => {
    for (const changed of [
      { ...detail, activation_authorized: true },
      { ...detail, manufacturer_validation_required: false },
      { ...detail, content_reverification_required: false },
    ])
      expect(deviceCatalogSchema.safeParse(changed).success).toBe(false);
    expect(
      deviceCatalogSchema.parse({ ...detail, review_context_current: false })
        .review_context_current,
    ).toBe(false);
  });
});
