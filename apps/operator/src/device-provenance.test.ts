import { it, expect } from 'vitest';
import { compileInputSchema, bindingViewSchema } from './draft-bindings-schema';
const id = '00000000-0000-4000-8000-000000000001',
  h = 'a'.repeat(64);
it('preserves v2 device provenance through UI export parsing', () => {
  const sources = {
    load: {
      plan: { id, revision: '2', plan_digest: h },
      binding: 'robot/supply',
      step_digest: h,
      action_digest: h,
    },
  };
  const value = compileInputSchema.parse({
    schema: 'rx.process-compile-input.v2',
    draft: id,
    cell: 'cell/a',
    source_revision: '1',
    binding_revision: '1',
    source_document_digest: h,
    bindings_digest: h,
    catalog_digest: h,
    source: {},
    bindings: { load: { host: 'host/a', intent: {} } },
    device_sources: sources,
  });
  expect(value.device_sources).toEqual(sources);
});
it('retains the explicit device-plan stale reason', () => {
  const view = bindingViewSchema.parse({
    cell: 'cell/a',
    draft: id,
    current_source_revision: '1',
    current_catalog_digest: h,
    binding: null,
    stale: ['DEVICE_PLAN_CHANGED'],
  });
  expect(view.stale).toEqual(['DEVICE_PLAN_CHANGED']);
});
