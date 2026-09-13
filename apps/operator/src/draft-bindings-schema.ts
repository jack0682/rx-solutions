import { z } from './schema-runtime';
const counter = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((v) => BigInt(v) <= 18446744073709551615n);
const digest = z.string().regex(/^[0-9a-f]{64}$/);
const planRef = z.object({ id: z.uuid(), revision: counter, plan_digest: digest });
const deviceSource = z.object({
  plan: planRef,
  binding: z.string(),
  step_digest: digest,
  action_digest: digest,
});
export const bindingCatalogSchema = z.object({
  device_plans: z.array(planRef).optional().default([]),
  required_cells: z.array(z.string()).optional().default([]),
  cell: z.string(),
  definition: digest,
  envelope: digest,
  catalog_digest: digest,
  candidates: z.array(
    z.object({
      device_plan: planRef.optional(),
      step: z.string(),
      host: z.string(),
      target: z.string(),
      kind: z.string(),
      resources: z.array(z.string()),
      intent_digest: digest,
      step_digest: digest,
    }),
  ),
});
export const bindingVersionSchema = z.object({
  device_plans: z.array(planRef).optional().default([]),
  required_cells: z.array(z.string()).optional().default([]),
  device_sources: z.record(z.string(), deviceSource).optional().default({}),
  draft: z.uuid(),
  cell: z.string(),
  revision: counter,
  source_revision: counter,
  source_digest: digest,
  catalog_digest: digest,
  selections: z.record(z.string(), z.string()),
  origins: z.record(z.string(), digest),
  resolved: z.record(
    z.string(),
    z.object({ host: z.string(), intent: z.record(z.string(), z.unknown()) }),
  ),
  missing: z.array(z.string()),
  complete: z.boolean(),
  updated_by: z.string(),
  updated_at: z.object({ clock_id: z.string(), ticks_ns: counter }),
});
export const bindingViewSchema = z.object({
  cell: z.string(),
  draft: z.uuid(),
  current_source_revision: counter,
  current_catalog_digest: digest,
  binding: bindingVersionSchema.nullable(),
  stale: z.array(z.enum(['SOURCE_CHANGED', 'CATALOG_CHANGED', 'DEVICE_PLAN_CHANGED'])),
});
export const compileInputSchema = z.object({
  schema: z.enum(['rx.process-compile-input.v1', 'rx.process-compile-input.v2']),
  device_sources: z.record(z.string(), deviceSource).optional().default({}),
  draft: z.uuid(),
  cell: z.string(),
  source_revision: counter,
  binding_revision: counter,
  source_document_digest: digest,
  bindings_digest: digest,
  catalog_digest: digest,
  source: z.unknown(),
  bindings: z.record(
    z.string(),
    z.object({ host: z.string(), intent: z.record(z.string(), z.unknown()) }),
  ),
});
export type BindingCatalog = z.infer<typeof bindingCatalogSchema>;
export type BindingView = z.infer<typeof bindingViewSchema>;
export type BindingVersion = z.infer<typeof bindingVersionSchema>;
export type BindingEdit = {
  devicePlans?: z.infer<typeof planRef>[];
  sourceRevision: string;
  catalogDigest: string;
  expected: string | null;
  selections: Record<string, string>;
};
