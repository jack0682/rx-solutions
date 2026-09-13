export class ApiFailure extends Error {
  constructor(
    public code: string,
    public unknownOutcome = false,
    public status = 0,
  ) {
    super(code);
  }
}
export async function api(
  path: string,
  options: { body?: unknown; signal?: AbortSignal } = {},
): Promise<unknown> {
  const mutation = options.body !== undefined;
  let response: Response;
  try {
    response = await fetch(path, {
      method: mutation ? 'POST' : 'GET',
      credentials: 'same-origin',
      cache: 'no-store',
      headers: mutation ? { 'Content-Type': 'application/json', 'X-RX-Client': 'browser-v1' } : {},
      body: mutation ? JSON.stringify(options.body) : undefined,
      signal: options.signal ?? AbortSignal.timeout(15000),
      redirect: 'error',
    });
  } catch (error) {
    if (options.signal?.aborted) throw error;
    throw new ApiFailure('CONNECTION_LOST', mutation);
  }
  let result: unknown = null;
  if (response.status !== 204) {
    try {
      result = await response.json();
    } catch {
      throw new ApiFailure('INVALID_RESPONSE', mutation, response.status);
    }
  }
  if (!response.ok) {
    const value = result as { code?: unknown; outcome_unknown?: unknown } | null;
    throw new ApiFailure(
      typeof value?.code === 'string' ? value.code : 'REQUEST_FAILED',
      value?.outcome_unknown === true || (mutation && response.status >= 500),
      response.status,
    );
  }
  return result;
}
const messages: Record<string, string> = {
  UNAUTHENTICATED: 'Sign in to continue. Your session may have expired.',
  FORBIDDEN: 'The current account or terminal is not authorized for this request.',
  CONNECTION_LOST: 'Cannot connect to the server.',
  INVALID_RESPONSE: 'Cannot validate the response format.',
  STALE_REVISION:
    'The state or configuration has changed. Check the latest state before trying again.',
  KEY_CONFLICT:
    'This request key is associated with different content. Contact the responsible operator.',
  NOT_COMMISSIONED: 'Site verification and operating qualification registration are required.',
  BUSY: 'Too many requests. Try again shortly.',
  VERIFICATION_REPORT_REJECTED:
    'Cannot verify the signature, content, or target of the verification evidence.',
  REVIEW_REVERIFICATION_FAILED:
    'Pre-approval revalidation failed. Check the files and verification policy.',
  DEVICE_REVIEW_NOT_CONFIGURED: 'Configure a device verification signer first.',
  DEVICE_REPORT_VERIFICATION_FAILED:
    'Check the device report signature, source, and review target.',
  DEVICE_APPROVAL_REVERIFICATION_FAILED:
    'Device source revalidation before approval failed. Check the policy and evidence.',
  PACKAGE_VERIFICATION_FAILED:
    'Check the imported file content and identifier. If verification is in progress, try again shortly.',
  QUALIFICATION_REQUIRED: 'This evidence does not yet meet the software approval requirements.',
  EXPIRED: 'The request review period has expired. Check the latest evidence again.',
  INVALID_INPUT: 'Check the input format and configuration.',
  BLOCKED_BY_CASE: 'Resolve the intervention reasons first.',
  CONDITION_FAILED: 'Some current start conditions are not met.',
  CONDITION_UNKNOWN: 'Some current start conditions cannot be verified.',
  HOST_NOT_PREPARED: 'Check the current Host connection and lease.',
  CONTINUITY_UNPROVEN: 'Verify continuity of the current execution connection.',
  MANDATE_REVOKED: 'A new start cannot be requested in the current run state.',
  STALE_EPOCH: 'The operating epoch has changed. Query the current state again.',
  BUDGET_EXHAUSTED: 'The permitted attempt budget has been exhausted.',
  HOST_RECOVERY_NOT_CONFIGURED:
    'Configure Host recovery inspection bindings for this installation first.',
  HOST_RECOVERY_UNAVAILABLE: 'Cannot verify the Host recovery binding response.',
  HOST_RECOVERY_INVALID_READ:
    'Cannot verify evidence for the current Host binding. Query the current records again.',
  HOST_RECOVERY_BUSY:
    'A Host recovery verification request is in progress. Query the current records again.',
};
export function explain(error: unknown) {
  if (error instanceof ApiFailure && error.code === 'CONNECTION_LOST' && error.unknownOutcome)
    return 'The request outcome was not received.';
  return error instanceof ApiFailure
    ? (messages[error.code] ?? `Request outcome needs verification · ${error.code}`)
    : 'Cannot validate the state to display.';
}
