import { beforeEach, expect, it, vi } from 'vitest';
import { api } from './api';
import {
  conflictText,
  definitionConflict,
  fetchName,
  pinnedName,
  provenanceRefs,
} from './definition-names';
import type { DefinitionView } from './definition-schema';
vi.mock('./api', () => ({ api: vi.fn() }));
const ref = {
  catalog: 'a0000000-0000-4000-8000-000000000001',
  id: 'a0000000-0000-4000-8000-000000000002',
  revision: '1',
  digest: 'a'.repeat(64),
};
const view: DefinitionView = {
  version: {
    definition: {
      reference: ref,
      label: 'Original tray name',
      body: { kind: 'RESOURCE_TYPE', parent: null, fields: {} },
    },
    archived: false,
    created_by: 'author',
    updated_by: 'author',
    updated_at: { clock_id: 'clock', ticks_ns: '1' },
  },
  effective: { fields: {}, values: {}, shadowed: {}, missing: [] },
};
beforeEach(() => vi.mocked(api).mockReset());
it('reads the pinned revision name and rejects a renamed newer revision', async () => {
  vi.mocked(api).mockResolvedValue(view);
  expect(await fetchName(ref, new AbortController().signal)).toBe('Original tray name');
  expect(vi.mocked(api).mock.calls[0][0]).toContain('revision=1');
  const renamed = structuredClone(view);
  renamed.version.definition.label = 'New name';
  renamed.version.definition.reference = { ...ref, revision: '2', digest: 'b'.repeat(64) };
  expect(() => pinnedName(ref, renamed)).toThrow('reference differs');
  const wrongHash = structuredClone(view);
  wrongHash.version.definition.reference.digest = 'b'.repeat(64);
  expect(() => pinnedName(ref, wrongHash)).toThrow('reference differs');
});
it('keeps different revisions of the same source identity distinct', () => {
  const v = structuredClone(view);
  v.effective.values.height = {
    value: { number: 760 },
    declared_by: { ...ref, revision: '2', digest: 'b'.repeat(64) },
  };
  v.effective.shadowed.height = [{ value: { number: 900 }, declared_by: ref }];
  expect(provenanceRefs(v).map((r) => r.revision)).toEqual(['1', '2']);
});
it('diagnoses create-only versus revision mismatch without another write', async () => {
  vi.mocked(api).mockResolvedValue(view);
  const message = await definitionConflict({ catalog: ref.catalog, id: ref.id, expected: null });
  expect(message).toContain('expected=null is create-only');
  expect(message).toContain('Original tray name');
  expect(message).toContain('current read at r1');
  expect(vi.mocked(api).mock.calls).toEqual([[expect.stringContaining('/api/v1/definition?')]]);
  expect(conflictText('2', 'Tray', '3')).toContain('expected=r2; current read');
});
it('does not manufacture a cause when the current read is forbidden or mismatched', async () => {
  vi.mocked(api).mockRejectedValue(new Error('forbidden'));
  const request = { catalog: ref.catalog, id: ref.id, expected: null };
  expect(await definitionConflict(request)).toBeNull();
  const wrong = structuredClone(view);
  wrong.version.definition.reference.id = ref.catalog;
  vi.mocked(api).mockResolvedValue(wrong);
  expect(await definitionConflict(request)).toBeNull();
});
