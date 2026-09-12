import type { ServiceHealth } from './service-health-schema';
export function servicePresentation(health: ServiceHealth | null | undefined, frameFresh: boolean) {
  if (!health) return { summary: '진단 미연결', current: false };
  if (health.availability === 'NOT_CONFIGURED')
    return { summary: '실행 서비스 미구성', current: false };
  if (health.availability === 'WAITING_REPORT')
    return { summary: '첫 상태 보고 대기', current: false };
  if (health.availability === 'CONTEXT_MISMATCH')
    return { summary: '구성 변경 · 서비스 확인 필요', current: false };
  if (!frameFresh || health.availability !== 'FRESH')
    return { summary: '최근 상태 확인 필요', current: false };
  return { summary: '상태 보고 수신', current: true };
}
const connection: Record<string, string> = {
  WAITING_FOR_PEER: 'Host 인증 대기',
  CONNECTING: '연결 확인 중',
  BOUND: '연결 등록됨',
  ATTENTION: '연결·갱신 확인 필요',
  STOPPED: '서비스 종료',
};
const observation: Record<string, string> = {
  WAITING: '관측 대기',
  RECEIVED: '관측 응답 수신',
  UNAVAILABLE: '관측 조회 확인 필요',
  INTEGRITY_ATTENTION: '근거 연속성 확인 필요',
  STOPPED: '조회 서비스 종료',
};
export function ServiceHealthCard({
  host,
  health,
  frameFresh,
}: {
  host: string;
  health: ServiceHealth | null | undefined;
  frameFresh: boolean;
}) {
  const presentation = servicePresentation(health, frameFresh);
  const active = presentation.current ? health : null;
  const observing = active?.observation === 'RECEIVED' && active.observation_recent;
  const delivering = active?.delivery === 'ACTIVE' && active.delivery_recent;
  return (
    <article className="service-health" data-service-host={host}>
      <div className="source-row-heading">
        <b>{host}</b>
        <span className={`badge ${presentation.current ? '' : 'warning'}`}>
          {presentation.summary}
        </span>
      </div>
      <div className="service-states">
        <div>
          <small>연결·사용권 조정</small>
          <strong>
            {active && active.connection ? connection[active.connection] : '확인 필요'}
          </strong>
        </div>
        <div>
          <small>관측 조회</small>
          <strong>
            {active && active.observation
              ? active.observation === 'RECEIVED' && !observing
                ? '최근 관측 응답 확인 필요'
                : observation[active.observation]
              : '확인 필요'}
          </strong>
        </div>
        <div>
          <small>명령 전달 루프</small>
          <strong>
            {active
              ? active.delivery === 'STOPPED'
                ? '전달 서비스 종료'
                : active.delivery === 'NOT_STARTED'
                  ? '아직 시작하지 않음'
                  : delivering
                    ? '전달 루프 확인'
                    : '최근 루프 동작 확인 필요'
              : '확인 필요'}
          </strong>
        </div>
      </div>
      {health?.delivery_error_history && (
        <p className="service-history">
          전달 오류 이력이 있습니다. 작업의 현재 결과와 조치 사유는 실행 기록에서 확인합니다.
        </p>
      )}
      <p className="muted">
        상태 보고 수신과 장비 운전 준비는 다릅니다. 연결 등록은 최근 통신 성공을 보장하지 않습니다.
      </p>
    </article>
  );
}
