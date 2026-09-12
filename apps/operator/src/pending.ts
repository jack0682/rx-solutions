import { pendingSchema, type Pending } from './schema';
const KEY = 'rx.pending-browser-request.v1';
export function readPending(storage: Pick<Storage, 'getItem'>): Pending | null {
  const value = storage.getItem(KEY);
  if (!value) return null;
  return pendingSchema.parse(JSON.parse(value));
}
export function savePending(storage: Pick<Storage, 'setItem'>, value: Pending) {
  storage.setItem(KEY, JSON.stringify(pendingSchema.parse(value)));
}
export function clearPending(storage: Pick<Storage, 'removeItem'>) {
  storage.removeItem(KEY);
}
export function requestBody(value: Pending) {
  return { request_key: value.request_key, command: value.command };
}
export function canRecover(
  value: Pending,
  principal: string,
  installation: string,
  storeGeneration: string,
) {
  return (
    value.principal === principal &&
    value.installation === installation &&
    value.store_generation === storeGeneration
  );
}
