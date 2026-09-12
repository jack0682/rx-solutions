import { describe, expect, it } from 'vitest';
import { previewRows } from './process-preview';
import { editableSourceSchema, stableDocument } from './draft-schema';
describe('process draft preview and identity', () => {
  it('bounds a branching invalid DAG and reports cyclic references without recursing forever', () => {
    const nodes = Array.from({ length: 30 }, (_, i) => ({
      id: `n${i}`,
      body: { kind: 'SEQUENCE', children: [`n${i + 1}`, `n${i + 1}`] },
    }));
    const source = editableSourceSchema.parse({
      schema: 'rx.process-source.v1',
      process: 'demo',
      entry: 'main',
      conditions: {},
      flows: [{ id: 'main', root: 'n0', nodes }],
    });
    expect(previewRows(source, 0, 32).length).toBeLessThanOrEqual(33);
    source.flows[0].nodes[0].body.children = ['n0'];
    expect(previewRows(source, 0)[1].problem).toContain('순환');
  });
  it('keeps array order while ignoring JSON object key order for receipt correlation', () => {
    expect(stableDocument({ b: 2, a: [1, 2] })).toBe(stableDocument({ a: [1, 2], b: 2 }));
    expect(stableDocument({ a: [1, 2] })).not.toBe(stableDocument({ a: [2, 1] }));
  });
  it('preserves unknown source fields for inspection instead of silently deleting them', () => {
    const source = editableSourceSchema.parse({
      schema: 'rx.process-source.v1',
      process: 'demo',
      entry: 'main',
      conditions: {},
      unknown: 'preserve',
      flows: [],
    });
    expect(source.unknown).toBe('preserve');
  });
});
