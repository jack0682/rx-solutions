import { describe, expect, it } from 'vitest';
import { servicePresentation } from './service-health';
import type { ServiceHealth } from './service-health-schema';
const health: ServiceHealth = {
  schema: 'rx.host-service-health.v1',
  availability: 'FRESH',
  sampled_at: { clock_id: 'test', ticks_ns: '1' },
  valid_for_ns: '3000000000',
  connection: 'BOUND',
  observation: 'RECEIVED',
  delivery: 'ACTIVE',
  last_observation_at: { clock_id: 'test', ticks_ns: '1' },
  last_delivery_pass_at: { clock_id: 'test', ticks_ns: '1' },
  observation_recent: true,
  delivery_recent: true,
  delivery_error_history: false,
};
describe('service health presentation', () => {
  it('does not present stale or missing telemetry as a current report', () => {
    expect(servicePresentation(null, true).current).toBe(false);
    expect(servicePresentation({ ...health, availability: 'STALE' }, true).current).toBe(false);
    expect(servicePresentation(health, false).current).toBe(false);
  });
  it('keeps configuration and awaiting-owner states separate from fresh service reports', () => {
    expect(servicePresentation({ ...health, availability: 'WAITING_REPORT' }, true).summary).toBe(
      'Awaiting first status report',
    );
    expect(servicePresentation({ ...health, availability: 'NOT_CONFIGURED' }, true).summary).toBe(
      'Execution service not configured',
    );
    expect(servicePresentation({ ...health, availability: 'CONTEXT_MISMATCH' }, true).current).toBe(
      false,
    );
    expect(servicePresentation(health, true).summary).toBe('Status report received');
  });
});
