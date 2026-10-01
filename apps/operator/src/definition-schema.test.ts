import { describe, expect, it } from 'vitest';
import {
  definitionInput,
  definitionSaveSchema,
  definitionViewSchema,
  validateDefinitionReceipt,
  type DefinitionView,
} from './definition-schema';
import { readPending, savePending } from './pending';
import type { Pending } from './schema';
const id = 'a0000000-0000-4000-8000-000000000001';
const catalog = 'a0000000-0000-4000-8000-000000000002';
const reference = { id, catalog, revision: '1', digest: 'a'.repeat(64) };
const spec = {
  value_type: 'BOOLEAN' as const,
  category: 'RESOURCE' as const,
  constraint_scope: null,
  unit: 'unitless',
  minimum: null,
  maximum: null,
  choices: [],
  vector_length: null,
  overridable: false,
  parameter_mapping: {},
};
const view: DefinitionView = {
  version: {
    definition: {
      reference,
      label: 'Tray occupied',
      body: { kind: 'PROPERTY', specification: spec },
    },
    archived: false,
    created_by: 'author',
    updated_by: 'author',
    updated_at: { clock_id: 'clock', ticks_ns: '1' },
  },
  effective: { fields: {}, values: {}, shadowed: {}, missing: [] },
};
const request: Pending = {
  principal: 'author',
  installation: catalog,
  store_generation: id,
  request_key: id,
  route: '/api/v1/definitions',
  label: 'Save property',
  command: { ...definitionInput(view), expected: null },
};

describe('definition authoring receipts', () => {
  it('recovers the same immutable request through browser storage', () => {
    let stored = '';
    savePending(
      {
        setItem: (_key, value) => {
          stored = value;
        },
      },
      request,
    );
    const recovered = readPending({ getItem: () => stored });
    expect(recovered).toEqual(request);
    expect(validateDefinitionReceipt(recovered!, view).value).toEqual(view);
  });
  it.each(['catalog', 'id', 'revision', 'body', 'actor', 'archive'])(
    'retains an unknown outcome when %s differs',
    (field) => {
      const wrong = structuredClone(view);
      if (field === 'catalog') wrong.version.definition.reference.catalog = id;
      if (field === 'id') wrong.version.definition.reference.id = catalog;
      if (field === 'revision') wrong.version.definition.reference.revision = '2';
      if (field === 'body' && wrong.version.definition.body.kind === 'PROPERTY')
        wrong.version.definition.body.specification.overridable = true;
      if (field === 'actor') wrong.version.updated_by = 'other';
      if (field === 'archive') wrong.version.archived = true;
      expect(() => validateDefinitionReceipt(request, wrong)).toThrow();
    },
  );
  it('does not normalize changed server content into a matching receipt', () => {
    const wrong = structuredClone(view);
    wrong.version.definition.label = ` ${wrong.version.definition.label} `;
    expect(() => validateDefinitionReceipt(request, wrong)).toThrow();
  });
  it('keeps missing, zero and false distinct and rejects unknown value tags', () => {
    const withValues = structuredClone(view);
    withValues.effective.values = {
      occupied: { value: { boolean: false }, declared_by: reference },
      height: { value: { number: 0 }, declared_by: reference },
    };
    withValues.effective.missing = ['pose'];
    expect(definitionViewSchema.parse(withValues).effective).toEqual(withValues.effective);
    expect(
      definitionViewSchema.safeParse({
        ...withValues,
        effective: {
          ...withValues.effective,
          values: { pose: { value: { pose: [] }, declared_by: reference } },
        },
      }).success,
    ).toBe(false);
  });
  it('requires a fully pinned reference and explicit archive state for saves', () => {
    const command = {
      catalog,
      id,
      expected: null,
      label: 'Slot',
      archived: false,
      body: { kind: 'RESOURCE_INSTANCE', base: reference, values: {} },
    };
    expect(definitionSaveSchema.safeParse(command).success).toBe(true);
    expect(
      definitionSaveSchema.safeParse({
        ...command,
        body: { ...command.body, base: { ...reference, digest: '' } },
      }).success,
    ).toBe(false);
    const { archived: _archive, ...withoutArchive } = command;
    expect(definitionSaveSchema.safeParse(withoutArchive).success).toBe(false);
  });
  it('checks catalog ACL receipts without depending on terminal ordering', () => {
    const command = {
      id: catalog,
      expected: '2',
      title: 'Laser',
      members: { reviewer: 'READ' },
      terminals: ['b', 'a'],
      archived: false,
    };
    const catalogRequest: Pending = { ...request, route: '/api/v1/definition-catalogs', command };
    const result = {
      id: catalog,
      revision: '3',
      title: 'Laser',
      owner: 'author',
      members: command.members,
      terminals: ['a', 'b'],
      archived: false,
    };
    expect(validateDefinitionReceipt(catalogRequest, result).route).toBe(
      '/api/v1/definition-catalogs',
    );
    expect(() =>
      validateDefinitionReceipt(catalogRequest, { ...result, members: { reviewer: 'EDIT' } }),
    ).toThrow();
    expect(() =>
      validateDefinitionReceipt(catalogRequest, { ...result, owner: 'other' }),
    ).toThrow();
  });
});
