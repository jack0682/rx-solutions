import { z } from './schema-runtime';
import { counter, timeSchema, type Pending } from './schema';
import { stableDocument } from './draft-schema';

const name = z.string().min(1).max(128);
const label = z
  .string()
  .min(1)
  .max(120)
  .refine((v) => v.trim().length > 0);
export const definitionKinds = [
  'PROPERTY',
  'OBJECT_TYPE',
  'OBJECT_MODEL',
  'RESOURCE_TYPE',
  'RESOURCE_MODEL',
  'RESOURCE_INSTANCE',
  'PROPERTY_SET',
  'TASK',
  'POINT_PATTERN',
] as const;
export const definitionRefSchema = z
  .object({
    catalog: z.uuid(),
    id: z.uuid(),
    revision: counter.refine((v) => v !== '0'),
    digest: z.string().regex(/^[0-9a-f]{64}$/),
  })
  .strict();
export const definitionValueSchema = z.union([
  z.object({ number: z.number().finite() }).strict(),
  z.object({ boolean: z.boolean() }).strict(),
  z.object({ text: z.string().max(2048) }).strict(),
  z.object({ vector: z.array(z.number().finite()).min(1).max(128) }).strict(),
]);
export const propertySchema = z
  .object({
    value_type: z.enum(['NUMBER', 'BOOLEAN', 'TEXT', 'VECTOR']),
    category: z.enum(['OBJECT', 'RESOURCE', 'EXECUTION', 'CONSTRAINT']),
    constraint_scope: z.enum(['OBJECT', 'RESOURCE', 'SYSTEM']).nullable(),
    unit: name,
    minimum: z.number().finite().nullable(),
    maximum: z.number().finite().nullable(),
    choices: z.array(z.string().min(1).max(120)).max(128),
    vector_length: z.number().int().min(1).max(128).nullable(),
    overridable: z.boolean(),
    parameter_mapping: z.record(name, name),
  })
  .strict();
const field = z.object({ property: definitionRefSchema, required: z.boolean() }).strict();
const source = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('OVERRIDE') }).strict(),
  z.object({ kind: z.literal('CONTEXT'), slot: name, field: name }).strict(),
  z.object({ kind: z.literal('PROPERTY_SET') }).strict(),
  z.object({ kind: z.literal('DEFAULT') }).strict(),
  z.object({ kind: z.literal('INPUT'), key: name }).strict(),
]);
const slot = z
  .object({
    label,
    kind: z.enum(['OBJECT', 'RESOURCE']),
    accepted_types: z.array(definitionRefSchema).min(1).max(32),
    required: z.boolean(),
    multiple: z.boolean(),
  })
  .strict();
const taskProperty = field
  .extend({ sources: z.array(source).min(1).max(16), default: definitionValueSchema.nullable() })
  .strict();
const values = z.record(name, definitionValueSchema);
export const definitionBodySchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('POINT_PATTERN'),
      resource_type: definitionRefSchema,
      origin: name,
      orientation: name.optional(),
      frame: name,
      axes: z
        .array(
          z
            .object({ count: name, pitch: name, direction: z.array(z.number().finite()).length(3) })
            .strict(),
        )
        .min(1)
        .max(3),
    })
    .strict(),
  z.object({ kind: z.literal('PROPERTY'), specification: propertySchema }).strict(),
  z
    .object({
      kind: z.literal('OBJECT_TYPE'),
      parent: definitionRefSchema.nullable(),
      fields: z.record(name, field),
    })
    .strict(),
  z
    .object({
      kind: z.literal('RESOURCE_TYPE'),
      parent: definitionRefSchema.nullable(),
      fields: z.record(name, field),
    })
    .strict(),
  z.object({ kind: z.literal('OBJECT_MODEL'), object_type: definitionRefSchema, values }).strict(),
  z
    .object({ kind: z.literal('RESOURCE_MODEL'), resource_type: definitionRefSchema, values })
    .strict(),
  z.object({ kind: z.literal('RESOURCE_INSTANCE'), base: definitionRefSchema, values }).strict(),
  z
    .object({
      kind: z.literal('PROPERTY_SET'),
      values: z.record(
        name,
        z.object({ property: definitionRefSchema, value: definitionValueSchema }).strict(),
      ),
    })
    .strict(),
  z
    .object({
      kind: z.literal('TASK'),
      metadata: z
        .object({
          display_name: z.string().max(120),
          category: label,
          services: z.discriminatedUnion('kind', [
            z.object({ kind: z.literal('ALL') }).strict(),
            z.object({ kind: z.literal('ONLY'), names: z.array(label).min(1).max(64) }).strict(),
          ]),
          completion_description: z.string().max(2048),
          default_timeout_ns: counter.refine((v) => v !== '0').nullable(),
        })
        .strict(),
      slots: z.record(name, slot),
      properties: z.record(name, taskProperty),
    })
    .strict(),
]);
export const definitionSaveSchema = z
  .object({
    catalog: z.uuid(),
    id: z.uuid(),
    expected: counter.nullable(),
    label,
    body: definitionBodySchema,
    archived: z.boolean(),
  })
  .strict();
export const catalogSaveSchema = z
  .object({
    id: z.uuid(),
    expected: counter.nullable(),
    title: label,
    members: z.record(name, z.enum(['READ', 'EDIT'])),
    terminals: z.array(name),
    archived: z.boolean(),
  })
  .strict();
export const catalogSchema = catalogSaveSchema
  .omit({ expected: true })
  .extend({ revision: counter, owner: name })
  .strict();
export const catalogPageSchema = z
  .object({ catalogs: z.array(catalogSchema).max(50), next: name.nullable() })
  .strict();
const resolvedValue = z
  .object({ value: definitionValueSchema, declared_by: definitionRefSchema })
  .strict();
export const definitionViewSchema = z
  .object({
    version: z
      .object({
        definition: z
          .object({ reference: definitionRefSchema, label, body: definitionBodySchema })
          .strict(),
        archived: z.boolean(),
        created_by: name,
        updated_by: name,
        updated_at: timeSchema,
      })
      .strict(),
    effective: z
      .object({
        fields: z.record(
          name,
          field
            .extend({ specification: propertySchema, declared_by: definitionRefSchema })
            .strict(),
        ),
        values: z.record(name, resolvedValue),
        shadowed: z.record(name, z.array(resolvedValue)),
        missing: z.array(name),
      })
      .strict(),
  })
  .strict();
const summary = z
  .object({
    reference: definitionRefSchema,
    label,
    kind: z.enum(definitionKinds),
    archived: z.boolean(),
  })
  .strict();
export const definitionPageSchema = z
  .object({ catalog: z.uuid(), definitions: z.array(summary).max(50), next: name.nullable() })
  .strict();
export const definitionHistorySchema = z
  .object({
    catalog: z.uuid(),
    id: z.uuid(),
    versions: z.array(summary).max(50),
    next: counter.nullable(),
  })
  .strict();
export type DefinitionRef = z.infer<typeof definitionRefSchema>;
export type DefinitionValue = z.infer<typeof definitionValueSchema>;
export type PropertySpec = z.infer<typeof propertySchema>;
export type DefinitionBody = z.infer<typeof definitionBodySchema>;
export type DefinitionView = z.infer<typeof definitionViewSchema>;
export type DefinitionSave = z.infer<typeof definitionSaveSchema>;
export type DefinitionSummary = z.infer<typeof summary>;
export type Catalog = z.infer<typeof catalogSchema>;
export type CatalogSave = z.infer<typeof catalogSaveSchema>;
export type DefinitionReceipt =
  | { route: '/api/v1/definitions'; value: DefinitionView; command: DefinitionSave; key: string }
  | { route: '/api/v1/definition-catalogs'; value: Catalog; command: CatalogSave; key: string };
export function definitionRoute(route: string) {
  return route === '/api/v1/definitions' || route === '/api/v1/definition-catalogs';
}
export function validateDefinitionReceipt(record: Pending, result: unknown): DefinitionReceipt {
  const same = (a: unknown, b: unknown) => stableDocument(a) === stableDocument(b);
  const next = (v: string | null) => String(BigInt(v ?? '0') + 1n);
  if (record.route === '/api/v1/definition-catalogs') {
    const command = catalogSaveSchema.parse(record.command);
    const value = catalogSchema.parse(result);
    if (
      value.id !== command.id ||
      value.revision !== next(command.expected) ||
      value.title !== command.title ||
      value.archived !== command.archived ||
      !same(value.members, command.members) ||
      !same([...value.terminals].sort(), [...command.terminals].sort()) ||
      value.owner !== record.principal
    )
      throw new Error('Catalog receipt differs from the request');
    return { route: record.route, value, command, key: record.request_key };
  }
  if (record.route !== '/api/v1/definitions') throw new Error('Not a definition request');
  const command = definitionSaveSchema.parse(record.command);
  const value = definitionViewSchema.parse(result);
  const d = value.version.definition;
  if (
    d.reference.catalog !== command.catalog ||
    d.reference.id !== command.id ||
    d.reference.revision !== next(command.expected) ||
    d.label !== command.label ||
    !same(d.body, command.body) ||
    value.version.archived !== command.archived ||
    value.version.updated_by !== record.principal
  )
    throw new Error('Definition receipt differs from the request');
  return { route: record.route, value, command, key: record.request_key };
}
export function definitionInput(view: DefinitionView): DefinitionSave {
  const { reference, label, body } = view.version.definition;
  return {
    catalog: reference.catalog,
    id: reference.id,
    expected: reference.revision,
    label,
    body,
    archived: view.version.archived,
  };
}
export function refKey(ref: DefinitionRef) {
  return `${ref.catalog}/${ref.id}/${ref.revision}/${ref.digest}`;
}
export function valueText(value: DefinitionValue) {
  return Object.values(value)
    .map((v) => (Array.isArray(v) ? v.join(', ') : String(v)))
    .join('');
}

export const pointPageSchema = z
  .object({
    subject: definitionRefSchema,
    rule: definitionRefSchema,
    inputs: definitionViewSchema.shape.effective,
    unit: name.nullable(),
    frame: z.string().nullable(),
    orientation_xyzw: z.array(z.number().finite()).length(4).nullable(),
    total: counter,
    offset: counter,
    points: z
      .array(
        z
          .object({
            index: counter,
            indices: z.array(counter),
            position: z.array(z.number().finite()).length(3),
          })
          .strict(),
      )
      .max(100),
    next: counter.nullable(),
    violations: z.array(
      z.object({ location: z.string(), code: z.string(), message: z.string() }).strict(),
    ),
  })
  .strict();
