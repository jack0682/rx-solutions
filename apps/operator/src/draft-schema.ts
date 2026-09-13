import type { BindingEdit } from './draft-bindings-schema';
import { z } from './schema-runtime';
const counter = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((v) => BigInt(v) <= 18446744073709551615n);
const digest = z.string().regex(/^[0-9a-f]{64}$/);
export const sourceReportSchema = z.object({
  validator: z.string(),
  validator_digest: digest,
  structurally_valid: z.boolean(),
  issues: z.array(z.object({ location: z.string(), code: z.string(), message: z.string() })),
  required_bindings: z.array(z.string()),
  procedure_references: z.array(z.unknown()),
  expanded_nodes: counter,
});
export const draftDetailSchema = z.object({
  version: z.object({
    id: z.uuid(),
    cell: z.string(),
    revision: counter,
    title: z.string(),
    document_digest: digest,
    validation: sourceReportSchema,
    created_by: z.string(),
    updated_by: z.string(),
    updated_at: z.object({ clock_id: z.string(), ticks_ns: counter }),
  }),
  document: z.unknown(),
});
export const draftPageSchema = z.object({
  cell: z.string(),
  drafts: z.array(
    z.object({
      id: z.uuid(),
      cell: z.string(),
      revision: counter,
      title: z.string(),
      document_digest: digest,
      structurally_valid: z.boolean(),
      issue_count: counter,
      updated_by: z.string(),
    }),
  ),
  next: z.uuid().nullable(),
});
export type DraftDetail = z.infer<typeof draftDetailSchema>;
export type DraftSummary = z.infer<typeof draftPageSchema>['drafts'][number];
export type DraftBuffer = {
  bindingEdit?: BindingEdit | null;
  sourceText?: string | null;
  conditionText?: string | null;
  id: string;
  cell: string;
  expected: string | null;
  title: string;
  document: unknown;
  dirty: boolean;
  validation: DraftDetail['version']['validation'] | null;
  baseline: DraftDetail | null;
};
export function stableDocument(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(stableDocument).join(',')}]`;
  if (v !== null && typeof v === 'object')
    return `{${Object.keys(v)
      .sort()
      .map((k) => `${JSON.stringify(k)}:${stableDocument((v as Record<string, unknown>)[k])}`)
      .join(',')}}`;
  return JSON.stringify(v) ?? 'null';
}
export function fromDetail(d: DraftDetail): DraftBuffer {
  return {
    id: d.version.id,
    cell: d.version.cell,
    expected: d.version.revision,
    title: d.version.title,
    document: d.document,
    dirty: false,
    validation: d.version.validation,
    baseline: d,
  };
}
export const editableSourceSchema = z
  .object({
    schema: z.literal('rx.process-source.v1'),
    process: z.string(),
    entry: z.string(),
    conditions: z.record(z.string(), z.unknown()),
    flows: z.array(
      z
        .object({
          id: z.string(),
          root: z.string(),
          nodes: z.array(
            z.object({ id: z.string(), body: z.record(z.string(), z.unknown()) }).passthrough(),
          ),
        })
        .passthrough(),
    ),
  })
  .passthrough();
export type EditableSource = z.infer<typeof editableSourceSchema>;
