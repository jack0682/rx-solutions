import { z } from 'zod';
const counter = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((s) => BigInt(s) <= 18446744073709551615n);
const time = z.object({ clock_id: z.string(), ticks_ns: counter });
export const serviceHealthSchema = z.object({
  schema: z.literal('rx.host-service-health.v1'),
  availability: z.enum(['NOT_CONFIGURED', 'WAITING_REPORT', 'FRESH', 'STALE', 'CONTEXT_MISMATCH']),
  sampled_at: time.nullable(),
  valid_for_ns: counter,
  connection: z
    .enum(['WAITING_FOR_PEER', 'CONNECTING', 'BOUND', 'ATTENTION', 'STOPPED'])
    .nullable(),
  observation: z
    .enum(['WAITING', 'RECEIVED', 'UNAVAILABLE', 'INTEGRITY_ATTENTION', 'STOPPED'])
    .nullable(),
  delivery: z.enum(['NOT_STARTED', 'ACTIVE', 'STOPPED']).nullable(),
  last_observation_at: time.nullable(),
  last_delivery_pass_at: time.nullable(),
  observation_recent: z.boolean(),
  delivery_recent: z.boolean(),
  delivery_error_history: z.boolean(),
});
export type ServiceHealth = z.infer<typeof serviceHealthSchema>;
