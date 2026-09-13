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
  UNAUTHENTICATED: '로그인이 필요하거나 세션이 만료되었습니다.',
  FORBIDDEN: '현재 계정 또는 단말에 이 요청의 권한이 없습니다.',
  CONNECTION_LOST: '서버와 연결할 수 없습니다.',
  INVALID_RESPONSE: '응답 형식을 확인할 수 없습니다.',
  STALE_REVISION: '상태나 구성이 변경되었습니다. 최신 상태를 확인한 뒤 다시 요청하세요.',
  KEY_CONFLICT: '같은 요청 번호에 다른 내용이 연결되어 있습니다. 담당자의 확인이 필요합니다.',
  NOT_COMMISSIONED: '현장 검증과 운전 자격 등록이 필요합니다.',
  BUSY: '요청이 많습니다. 잠시 후 다시 시도하세요.',
  VERIFICATION_REPORT_REJECTED: '검증 자료의 서명·내용·대상 정보를 확인할 수 없습니다.',
  REVIEW_REVERIFICATION_FAILED:
    '승인 전 재검증을 통과하지 못했습니다. 파일과 검증 정책을 확인해 주세요.',
  DEVICE_REVIEW_NOT_CONFIGURED: '장비 검증 서명자 설정이 필요합니다.',
  DEVICE_REPORT_VERIFICATION_FAILED: '장비 보고서의 서명·원본·검토 대상을 확인해 주세요.',
  DEVICE_APPROVAL_REVERIFICATION_FAILED:
    '승인 전 장비 원본 재검증을 통과하지 못했습니다. 정책과 자료를 확인해 주세요.',
  PACKAGE_VERIFICATION_FAILED:
    '반입 파일의 내용과 식별자를 확인해 주세요. 검증 중이면 잠시 후 다시 시도하세요.',
  QUALIFICATION_REQUIRED: '이 자료는 아직 소프트웨어 승인 조건을 충족하지 못했습니다.',
  EXPIRED: '요청 확인 시간이 지났습니다. 최신 자료를 다시 확인해 주세요.',
  INVALID_INPUT: '입력 형식이나 구성 내용을 확인하세요.',
  BLOCKED_BY_CASE: '개입 사유를 해결해야 합니다.',
  CONDITION_FAILED: '현재 시작 조건 중 충족되지 않은 항목이 있습니다.',
  CONDITION_UNKNOWN: '현재 시작 조건을 확인할 수 없는 항목이 있습니다.',
  HOST_NOT_PREPARED: '현재 Host 연결과 사용권을 확인해야 합니다.',
  CONTINUITY_UNPROVEN: '현재 실행 연결의 연속성을 확인해야 합니다.',
  MANDATE_REVOKED: '현재 실행 상태에서는 새 시작을 요청할 수 없습니다.',
  STALE_EPOCH: '운전 세대가 변경되었습니다. 현재 상태를 다시 조회하세요.',
  BUDGET_EXHAUSTED: '허용된 시도 예산을 모두 사용했습니다.',
  HOST_RECOVERY_NOT_CONFIGURED: '이 설치의 Host 복구 조회 연결 설정이 필요합니다.',
  HOST_RECOVERY_UNAVAILABLE: 'Host 복구 연결의 응답을 확인할 수 없습니다.',
  HOST_RECOVERY_INVALID_READ:
    'Host의 현재 연결 근거를 확인할 수 없습니다. 현재 기록을 다시 조회하세요.',
  HOST_RECOVERY_BUSY: 'Host 복구 확인 요청이 진행 중입니다. 현재 기록을 다시 조회하세요.',
};
export function explain(error: unknown) {
  if (error instanceof ApiFailure && error.code === 'CONNECTION_LOST' && error.unknownOutcome)
    return '요청 처리 결과를 수신하지 못했습니다.';
  return error instanceof ApiFailure
    ? (messages[error.code] ?? `요청 처리 확인 필요 · ${error.code}`)
    : '표시할 상태를 검증할 수 없습니다.';
}
