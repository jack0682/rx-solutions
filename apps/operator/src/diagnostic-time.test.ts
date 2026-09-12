import { describe, expect, it } from 'vitest';
import { diagnosticDeadline, diagnosticFresh } from './diagnostic-time';
import { diagnosticsSchema, valueSchema } from './diagnostics-schema';
describe('diagnostic display validity', () => {
  it('includes transport delay and never extends the server budget from response receipt', () => {
    expect(diagnosticDeadline(100, '50000000')).toBe(150);
    expect(diagnosticFresh(100, '50000000', 140, true)).toBe(true);
    expect(diagnosticFresh(100, '50000000', 150, true)).toBe(false);
    expect(diagnosticFresh(100, '50000000', 200, true)).toBe(false);
  });
  it('does not use stale, failed, reversed-clock or zero-duration snapshots', () => {
    expect(diagnosticFresh(100, '1000000000', 110, false)).toBe(false);
    expect(diagnosticFresh(100, '1000000000', 99, true)).toBe(false);
    expect(diagnosticFresh(100, '0', 100, true)).toBe(false);
  });
  it('preserves exact integers and refuses unrecognized diagnostics', () => {
    expect(valueSchema.parse({ integer: '9223372036854775807' })).toEqual({
      integer: '9223372036854775807',
    });
    expect(valueSchema.safeParse({ integer: '9223372036854775808' }).success).toBe(false);
    expect(valueSchema.safeParse({ integer: '-0' }).success).toBe(false);
    expect(diagnosticsSchema.safeParse({ schema: 'rx.cell-diagnostics.v2' }).success).toBe(false);
  });
});
