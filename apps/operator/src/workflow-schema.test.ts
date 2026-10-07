import { expect, it } from 'vitest';
import {
  parseQuantity,
  readableViolation,
  reportIdentityTime,
  violationProperty,
  workflowReportIndexSchema,
  quantityText,
  validateWorkflowReceipt,
  violationNode,
  workflowReceiptSchema,
  type WorkflowReceipt,
} from './workflow-schema';
import type { Pending } from './schema';
const ref = {
  catalog: 'a0000000-0000-4000-8000-000000000001',
  id: 'a0000000-0000-4000-8000-000000000002',
  revision: '1',
  digest: 'a'.repeat(64),
};
const request = {
  workflow: ref,
  contexts: {},
  property_sets: [],
  overrides: {},
  inputs: {},
  slot_index: '0',
};
const resolved = (q: ReturnType<typeof parseQuantity>) => ({
  value: q,
  frame: null,
  selected_source: 'DEFAULT',
  origins: [{ kind: 'DEFAULT', reference: ref, path: 'tasks/act/default', value: q }],
});
const receipt: WorkflowReceipt = {
  reference: { ...ref, id: 'a0000000-0000-4000-8000-000000000003' },
  created_by: 'author',
  created_at: { clock_id: 'clock', ticks_ns: '1' },
  report: {
    schema: 'rx.workflow-resolution.v1',
    resolver_digest: 'b'.repeat(64),
    request,
    valid: true,
    concrete: true,
    status: 'RESOLVED_NOT_QUALIFIED',
    definitions: [],
    steps: [
      {
        node: 'first',
        task: 'act',
        label: 'Act',
        timeout_property: 'timeout',
        properties: {
          value: resolved(parseQuantity('1', 'mm')),
          timeout: resolved(parseQuantity('10', 's')),
        },
        declared_capabilities: { act: resolved(parseQuantity('true', 'unitless')) },
        skills: [
          {
            implementation: resolved(parseQuantity('"simulator"', 'unitless')),
            version: resolved(parseQuantity('"1"', 'unitless')),
            primitive: 'set',
            parameters: { value: 'value' },
          },
        ],
        done: { observation: 'done', property: ref, equals: parseQuantity('true', 'unitless') },
        on_failure: 'STOP',
        on_unknown: 'HOLD_AND_RECONCILE',
      },
    ],
    violations: [],
  },
};
const pending: Pending = {
  request_key: ref.id,
  route: '/api/v1/workflow-resolutions',
  principal: 'author',
  installation: ref.catalog,
  store_generation: ref.id,
  label: 'Resolve',
  command: request,
};
it('correlates the complete original resolution request and actor', () => {
  expect(validateWorkflowReceipt(pending, receipt).reference.id).toBe(receipt.reference.id);
  for (const change of ['workflow', 'contexts', 'override', 'slot', 'actor']) {
    const changed = structuredClone(receipt);
    if (change === 'workflow') changed.report.request.workflow.revision = '2';
    if (change === 'contexts') changed.report.request.contexts.part = [ref];
    if (change === 'override')
      changed.report.request.overrides.pick = { force: parseQuantity('60', 'N') };
    if (change === 'slot') changed.report.request.slot_index = '1';
    if (change === 'actor') changed.created_by = 'another';
    expect(() => validateWorkflowReceipt(pending, changed)).toThrow();
  }
});
it('rejects inconsistent blocked/concrete status and catalog scope', () => {
  const bad = structuredClone(receipt);
  bad.report.violations = [
    { location: 'nodes/pick/properties/force', code: 'LIMIT', message: 'too high' },
  ];
  expect(() => workflowReceiptSchema.parse(bad)).toThrow();
  bad.report.valid = false;
  bad.report.status = 'BLOCKED';
  expect(() => workflowReceiptSchema.parse(bad)).toThrow();
  bad.report.concrete = false;
  expect(workflowReceiptSchema.parse(bad).report.valid).toBe(false);
  bad.reference.catalog = ref.id;
  expect(() => workflowReceiptSchema.parse(bad)).toThrow();
});
it('encodes explicit units/ranges without calculating execution values', () => {
  expect(parseQuantity('20..60', 'N').data).toEqual({
    kind: 'NUMBER',
    range: { min: 20, max: 60 },
  });
  expect(quantityText(parseQuantity('false', 'unitless'))).toBe('false');
  expect(quantityText(parseQuantity('0', 'mm'))).toBe('0');
  expect(() => parseQuantity('60', '')).toThrow();
  expect(() => parseQuantity('NaN', 'N')).toThrow();
  expect(() => parseQuantity('..60', 'N')).toThrow();
});
it('locates node names without confusing JSON pointer escapes', () => {
  expect(violationNode('nodes/load~1station/properties/force')).toBe('load/station');
  expect(violationNode('contexts/part')).toBeNull();
});

it('rejects missing command parameters, empty provenance and false concrete claims', () => {
  const missing = structuredClone(receipt);
  delete missing.report.steps[0].properties.value;
  expect(() => workflowReceiptSchema.parse(missing)).toThrow();
  const ranged = structuredClone(receipt);
  ranged.report.steps[0].properties.value = resolved(parseQuantity('1..2', 'mm'));
  expect(() => workflowReceiptSchema.parse(ranged)).toThrow();
  const noSource = structuredClone(receipt);
  noSource.report.steps[0].properties.value.origins = [];
  expect(() => workflowReceiptSchema.parse(noSource)).toThrow();
});

it('formats worst-case quantities without changing unknown messages', () => {
  const message = `Force limit; worst-case left=${JSON.stringify(parseQuantity('60', 'N'))} right=${JSON.stringify(parseQuantity('35', 'N'))}`;
  expect(readableViolation(message)).toBe('Force limit; worst-case left=60 N, right=35 N');
  expect(readableViolation('Unknown original diagnostic')).toBe('Unknown original diagnostic');
  expect(readableViolation('bad; worst-case left={} right={}')).toBe(
    'bad; worst-case left={} right={}',
  );
  expect(violationProperty('nodes/pick/properties/grip_force')).toEqual({
    node: 'pick',
    property: 'grip_force',
  });
  expect(violationProperty('contexts/part')).toBeNull();
});
it('reads indexed saved contexts and keeps the timestamp clock explicit', () => {
  const entry = {
    reference: ref,
    workflow: ref,
    slot_index: '0',
    status: 'BLOCKED',
    created_at: { clock_id: 'process-clock', ticks_ns: '123' },
    created_by: 'author',
    contexts: { part: [ref] },
    overrides: { pick: { force: parseQuantity('60', 'N') } },
    definitions: [{ reference: ref, label: 'Part A' }],
  };
  const index = workflowReportIndexSchema.parse({
    schema: 'rx.workflow-resolution-index.v1',
    catalog: ref.catalog,
    reports: [entry],
    next: null,
  });
  expect(index.reports[0].contexts.part).toEqual([ref]);
  expect(reportIdentityTime('017f22e2-79b0-7cc3-98c4-dc0c0c07398f')).toBe(
    '2022-02-22T19:22:22.000Z',
  );
  expect(reportIdentityTime(ref.id)).toBeNull();
});
