import { serviceHealthSchema } from './service-health-schema';
import { z } from './schema-runtime';
const count = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((s) => BigInt(s) <= 18446744073709551615n);
const clock = z.object({ clock_id: z.string(), ticks_ns: count });
export const valueSchema = z.union([
  z.strictObject({ boolean: z.boolean() }),
  z.strictObject({
    integer: z
      .string()
      .regex(/^(0|[1-9][0-9]*|-[1-9][0-9]*)$/)
      .refine((s) => BigInt(s) >= -9223372036854775808n && BigInt(s) <= 9223372036854775807n),
  }),
  z.strictObject({ real: z.number().finite() }),
  z.strictObject({ symbol: z.string() }),
  z.strictObject({ reals: z.object({ values: z.array(z.number().finite()) }) }),
]);
export type Value = z.infer<typeof valueSchema>;
export type Expression =
  | { op: 'ALL'; children: Expression[] }
  | { op: 'ANY'; children: Expression[] }
  | { op: 'EQ'; fact: string; schema: string; unit: string; expected: Value }
  | { op: 'RANGE'; fact: string; schema: string; unit: string; min: number; max: number }
  | { op: 'SET_CONTAINS'; fact: string; schema: string; unit: string; expected: string };
const expression: z.ZodType<Expression> = z.lazy(() =>
  z.discriminatedUnion('op', [
    z.object({ op: z.literal('ALL'), children: z.array(expression) }),
    z.object({ op: z.literal('ANY'), children: z.array(expression) }),
    z.object({
      op: z.literal('EQ'),
      fact: z.string(),
      schema: z.string(),
      unit: z.string(),
      expected: valueSchema,
    }),
    z.object({
      op: z.literal('RANGE'),
      fact: z.string(),
      schema: z.string(),
      unit: z.string(),
      min: z.number().finite(),
      max: z.number().finite(),
    }),
    z.object({
      op: z.literal('SET_CONTAINS'),
      fact: z.string(),
      schema: z.string(),
      unit: z.string(),
      expected: z.string(),
    }),
  ]),
);
const issue = z.enum([
  'MISSING_OBSERVATION',
  'HOST_UNREGISTERED',
  'GENERATION_UNREGISTERED',
  'GENERATION_MISMATCH',
  'WRONG_SOURCE_HOST',
  'SCHEMA_MISMATCH',
  'UNIT_MISMATCH',
  'BAD_QUALITY',
  'ORIGIN_AGE_UNBOUNDED',
  'DISPUTED',
  'CLOCK_MISMATCH',
  'FUTURE_TIMESTAMP',
  'UNCERTAINTY_EXCEEDED',
  'AGE_EXCEEDED',
  'AGE_OVERFLOW',
]);
const observation = z.object({
  cell: z.string(),
  id: z.string(),
  source_host: z.string(),
  source_generation: z.uuid(),
  schema: z.string(),
  unit: z.string(),
  acquired_at: clock,
  maximum_age_ns: count,
  acquisition_uncertainty_ns: count,
  quality_good: z.boolean(),
  origin_age_bounded: z.boolean(),
  disputed: z.boolean(),
  value: valueSchema,
  evidence_id: z.uuid(),
});
export const diagnosticsSchema = z.object({
  schema: z.literal('rx.cell-diagnostics.v1'),
  display_valid_for_ns: count.refine((s) => BigInt(s) <= 3000000000n),
  sources: z.array(
    z.object({
      source: z.string(),
      host: z.string(),
      expected_generation: z.uuid().nullable(),
      maximum_age_ns: count,
      maximum_uncertainty_ns: count,
      age_ns: count.nullable(),
      usable: z.boolean(),
      issues: z.array(issue),
      observation: observation.nullable(),
    }),
  ),
  hosts: z.array(
    z.object({
      host: z.string(),
      context: z.enum([
        'UNREGISTERED',
        'IDENTITY_UNAVAILABLE',
        'CONTEXT_STALE',
        'CLOCK_MISMATCH',
        'LEASE_EXPIRED',
        'CURRENT',
      ]),
      grant_valid_until: clock.nullable(),
      runtime: serviceHealthSchema.nullable().optional(),
    }),
  ),
  conditions: z.array(
    z.object({
      path: z.string(),
      group: z.enum(['START', 'MAINTAINED', 'OPERATION']),
      step: z.string().nullable(),
      expression,
      verdict: z.enum(['PASS', 'FAIL', 'UNKNOWN']),
      reason: z.enum(['SATISFIED', 'NOT_SATISFIED', 'SOURCE_UNAVAILABLE', 'EXPRESSION_MISMATCH']),
      evidence_ids: z.array(z.uuid()),
      valid_until: clock.nullable(),
    }),
  ),
});
export type Diagnostics = z.infer<typeof diagnosticsSchema>;
