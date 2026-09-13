import { describe, expect, it, vi } from 'vitest';

describe('strict CSP schema runtime', () => {
  it('loads and validates production schemas without probing dynamic code', async () => {
    vi.resetModules();
    const forbidden = vi.fn(() => {
      throw new Error('dynamic code forbidden');
    });
    vi.stubGlobal('Function', forbidden);
    try {
      const { installationSchema } = await import('./schema');
      const { z } = await import('./schema-runtime');
      const object = z.strictObject({ value: z.string() });
      expect(object.parse({ value: 'ok' })).toEqual({ value: 'ok' });
      expect(object.safeParse({ value: 3 }).success).toBe(false);
      expect(object.safeParse({ value: 'ok', extra: true }).success).toBe(false);
      expect(installationSchema.safeParse({}).success).toBe(false);
      expect(forbidden).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
