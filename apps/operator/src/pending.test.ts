import { describe, it, expect } from 'vitest';
import { readPending, savePending, requestBody, canRecover } from './pending';
import { overviewSchema, type Pending } from './schema';
const pending: Pending = {
  request_key: 'c39d227e-2f3b-439f-b90f-12e3e987985a',
  route: '/api/v1/runs',
  command: { cell: 'cell/demo', expected_cell: '9007199254740993' },
  label: 'Prepare new run',
  principal: 'admin',
  installation: '6a59a0c0-1e86-4274-87e5-79d086bb328d',
  store_generation: '825492b5-dae2-40b1-a778-656e06ca200d',
};
describe('unknown request recovery', () => {
  it('retains exact counter strings and request identity over a reload', () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (k: string) => values.get(k) ?? null,
      setItem: (k: string, v: string) => {
        values.set(k, v);
      },
    };
    savePending(storage, pending);
    expect(requestBody(readPending(storage)!)).toEqual({
      request_key: pending.request_key,
      command: pending.command,
    });
    expect(canRecover(pending, 'admin', pending.installation, 'different-store-generation')).toBe(
      false,
    );
    expect(
      canRecover(pending, 'another-account', pending.installation, pending.store_generation),
    ).toBe(false);
    expect(canRecover(pending, 'admin', 'a-different-installation', pending.store_generation)).toBe(
      false,
    );
  });
  it('does not silently discard an unreadable pending request', () => {
    expect(() => readPending({ getItem: () => '{bad-json' } as unknown as Storage)).toThrow();
    expect(() =>
      readPending({
        getItem: () => JSON.stringify({ ...pending, route: '/device/execute' }),
      } as unknown as Storage),
    ).toThrow();
  });
  it('never accepts an invalid overview as current state', () => {
    expect(overviewSchema.safeParse({ cells: [], ready: true }).success).toBe(false);
  });
});
