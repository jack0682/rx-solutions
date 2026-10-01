import { z } from './schema-runtime';
import { counter, timeSchema, type Pending } from './schema';
import { definitionRefSchema, refKey } from './definition-schema';
import { stableDocument } from './draft-schema';
const name = z.string().min(1);
const span = z.object({ min: z.number().finite(), max: z.number().finite() }).strict();
export const quantitySchema = z
  .object({
    unit: name,
    data: z.discriminatedUnion('kind', [
      z.object({ kind: z.literal('NUMBER'), range: span }).strict(),
      z.object({ kind: z.literal('VECTOR'), ranges: z.array(span).max(128) }).strict(),
      z.object({ kind: z.literal('BOOLEAN'), value: z.boolean() }).strict(),
      z.object({ kind: z.literal('TEXT'), value: z.string() }).strict(),
    ]),
  })
  .strict();
const source = z.object({ kind: name }).passthrough();
const usage = z
  .object({
    property: definitionRefSchema,
    sources: z.array(source),
    default: quantitySchema.nullable(),
  })
  .strict();
const slot = z
  .object({
    label: name,
    kind: z.enum(['OBJECT', 'RESOURCE']),
    accepted_types: z.array(definitionRefSchema),
    required: z.boolean(),
    multiple: z.boolean(),
  })
  .strict();
const task = z
  .object({
    label: name,
    contexts: z.array(name),
    properties: z.record(name, usage),
    timeout_property: name,
  })
  .passthrough();
export const workflowModelSchema = z
  .object({
    reference: definitionRefSchema,
    label: name,
    spec: z
      .object({
        schema: z.literal('rx.workflow-model.v1'),
        contexts: z.record(name, slot),
        defaults: z.record(name, z.array(definitionRefSchema)),
        tasks: z.record(name, task),
        steps: z.array(z.object({ id: name, task: name }).strict()),
      })
      .passthrough(),
    created_by: name,
    updated_by: name,
    updated_at: timeSchema,
  })
  .strict();
export const workflowsSchema = z
  .object({
    catalog: z.uuid(),
    workflows: z.array(z.object({ reference: definitionRefSchema, label: name }).strict()),
    next: name.nullable(),
  })
  .strict();
export const workflowRequestSchema = z
  .object({
    workflow: definitionRefSchema,
    contexts: z.record(name, z.array(definitionRefSchema)),
    property_sets: z.array(definitionRefSchema),
    overrides: z.record(name, z.record(name, quantitySchema)),
    inputs: z.record(name, quantitySchema),
    slot_index: counter,
  })
  .strict();
const reportedQuantitySchema = quantitySchema.superRefine((q, ctx) => {
  const d = q.data;
  if (
    (d.kind === 'NUMBER' && d.range.min > d.range.max) ||
    (d.kind === 'VECTOR' && (!d.ranges.length || d.ranges.some((r) => r.min > r.max))) ||
    ((d.kind === 'BOOLEAN' || d.kind === 'TEXT') && q.unit !== 'unitless')
  )
    ctx.addIssue({ code: 'custom', message: 'Invalid resolved quantity' });
});
const origin = z
  .object({
    kind: name,
    reference: definitionRefSchema.nullable(),
    path: z.string(),
    value: reportedQuantitySchema,
  })
  .strict();
const resolved = z
  .object({
    value: reportedQuantitySchema,
    frame: z.string().nullable(),
    selected_source: name,
    origins: z.array(origin).min(1),
  })
  .strict();
const done = z
  .object({ observation: name, property: definitionRefSchema, equals: quantitySchema })
  .strict();
const skill = z
  .object({
    implementation: resolved,
    version: resolved,
    primitive: name,
    parameters: z.record(name, name),
  })
  .strict();
export const workflowReceiptSchema = z
  .object({
    reference: definitionRefSchema,
    report: z
      .object({
        schema: z.literal('rx.workflow-resolution.v1'),
        resolver_digest: z.string().regex(/^[0-9a-f]{64}$/),
        request: workflowRequestSchema,
        valid: z.boolean(),
        concrete: z.boolean(),
        status: z.enum(['BLOCKED', 'RESOLVED_NOT_QUALIFIED', 'BOUNDED_INPUT_NOT_EXECUTABLE']),
        definitions: z.array(z.object({ reference: definitionRefSchema, label: name }).strict()),
        steps: z
          .array(
            z
              .object({
                node: name,
                task: name,
                label: name,
                timeout_property: name,
                properties: z.record(name, resolved),
                skills: z.array(skill),
                declared_capabilities: z.record(name, resolved),
                done,
                on_failure: z.literal('STOP'),
                on_unknown: z.literal('HOLD_AND_RECONCILE'),
              })
              .strict(),
          )
          .min(1)
          .max(128),
        violations: z.array(z.object({ location: name, code: name, message: z.string() }).strict()),
      })
      .strict(),
    created_by: name,
    created_at: timeSchema,
  })
  .strict()
  .superRefine((v, ctx) => {
    const r = v.report;
    const concrete = (q: Quantity) =>
      q.data.kind === 'NUMBER'
        ? q.data.range.min === q.data.range.max
        : q.data.kind === 'VECTOR'
          ? q.data.ranges.every((r) => r.min === r.max)
          : true;
    if (
      new Set(r.steps.map((s) => s.node)).size !== r.steps.length ||
      (r.valid &&
        r.steps.some(
          (s) =>
            !s.properties[s.timeout_property] ||
            !s.skills.length ||
            s.skills.some((k) => Object.values(k.parameters).some((p) => !s.properties[p])),
        )) ||
      (r.concrete &&
        r.steps.some(
          (s) =>
            !s.properties[s.timeout_property] ||
            !concrete(s.properties[s.timeout_property].value) ||
            s.skills.some((k) =>
              Object.values(k.parameters).some(
                (p) => !s.properties[p] || !concrete(s.properties[p].value),
              ),
            ),
        ))
    )
      ctx.addIssue({
        code: 'custom',
        message: 'Resolution nodes, parameters or concreteness differ',
      });

    if (
      v.reference.catalog !== r.request.workflow.catalog ||
      v.reference.revision !== '1' ||
      r.valid !== (r.violations.length === 0) ||
      (r.concrete && !r.valid) ||
      r.status !==
        (!r.valid
          ? 'BLOCKED'
          : r.concrete
            ? 'RESOLVED_NOT_QUALIFIED'
            : 'BOUNDED_INPUT_NOT_EXECUTABLE')
    )
      ctx.addIssue({ code: 'custom', message: 'Resolution status or scope differs' });
  });
export const workflowReportsSchema = z
  .object({
    catalog: z.uuid(),
    reports: z.array(
      z
        .object({
          reference: definitionRefSchema,
          workflow: definitionRefSchema,
          slot_index: counter,
          status: name,
        })
        .strict(),
    ),
    next: name.nullable(),
  })
  .strict();
export type WorkflowModel = z.infer<typeof workflowModelSchema>;
export type WorkflowReceipt = z.infer<typeof workflowReceiptSchema>;
export type WorkflowRequest = z.infer<typeof workflowRequestSchema>;
export type Quantity = z.infer<typeof quantitySchema>;
export function workflowRoute(route: string) {
  return route === '/api/v1/workflow-resolutions';
}
export function validateWorkflowReceipt(record: Pending, result: unknown): WorkflowReceipt {
  const request = workflowRequestSchema.parse(record.command);
  const receipt = workflowReceiptSchema.parse(result);
  if (
    !workflowRoute(record.route) ||
    stableDocument(receipt.report.request) !== stableDocument(request) ||
    receipt.created_by !== record.principal
  )
    throw new Error('Resolution receipt differs from original request');
  return receipt;
}
export function quantityText(q: Quantity): string {
  const v = q.data;
  const scalar = (s: z.infer<typeof span>) =>
    s.min === s.max ? String(s.min) : `${s.min} … ${s.max}`;
  if (v.kind === 'NUMBER') return scalar(v.range);
  if (v.kind === 'VECTOR') return `[${v.ranges.map(scalar).join(', ')}]`;
  return String(v.value);
}
export function parseQuantity(value: string, unit: string): Quantity {
  let data: Quantity['data'];
  if (value.includes('..')) {
    const parts = value.split('..');
    if (parts.length !== 2 || parts.some((p) => !p.trim())) throw new Error('Invalid range');
    data = { kind: 'NUMBER', range: { min: Number(parts[0]), max: Number(parts[1]) } };
  } else {
    const raw: unknown = JSON.parse(value);
    if (typeof raw === 'number') data = { kind: 'NUMBER', range: { min: raw, max: raw } };
    else if (typeof raw === 'boolean') data = { kind: 'BOOLEAN', value: raw };
    else if (typeof raw === 'string') data = { kind: 'TEXT', value: raw };
    else if (Array.isArray(raw) && raw.every((v) => typeof v === 'number'))
      data = { kind: 'VECTOR', ranges: raw.map((v: number) => ({ min: v, max: v })) };
    else throw new Error('Unsupported quantity');
  }
  return quantitySchema.parse({ unit, data });
}
export function violationNode(location: string): string | null {
  const parts = location.split('/');
  return parts[0] === 'nodes' && parts.length > 1
    ? parts[1].replaceAll('~1', '/').replaceAll('~0', '~')
    : null;
}
export const referenceKey = refKey;
