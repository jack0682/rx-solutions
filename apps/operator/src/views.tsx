import type { CellOverview } from './schema';
import { short, text } from './labels';

export function Runs({ cell }: { cell: CellOverview }) {
  return (
    <section className="panel records">
      <div className="section-heading">
        <div>
          <p className="eyebrow">RUN RECORDS</p>
          <h3>실행 기록</h3>
        </div>
        <span className="count">
          {cell.runs.length}
          {cell.runs_truncated ? '+' : ''}
        </span>
      </div>
      {cell.runs.length ? (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>실행 번호</th>
                <th>상태</th>
                <th>용도</th>
                <th>허용 시도 사용량</th>
                <th>기록 버전</th>
              </tr>
            </thead>
            <tbody>
              {cell.runs.map(({ revision, value: r }) => (
                <tr key={r.id}>
                  <td className="mono" title={r.id}>
                    {short(r.id)}
                  </td>
                  <td>
                    <span className={`badge ${r.state === 'RECOVERY_REQUIRED' ? 'warning' : ''}`}>
                      {text(r.state)}
                    </span>
                  </td>
                  <td>
                    {r.purpose === 'PRODUCTION'
                      ? '생산'
                      : r.purpose === 'SETUP'
                        ? '설정'
                        : '시작 시 지정'}
                  </td>
                  <td>
                    {r.budget
                      ? `${r.budget.unit === 'PART_ATTEMPT' ? '소재' : '작업'} ${r.budget.consumptions.length} / ${r.budget.limit}`
                      : '미지정'}
                  </td>
                  <td>r{revision}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="empty-inline">
          <span>↗</span>
          <div>
            <b>아직 실행 기록이 없습니다</b>
            <p>실행을 준비하면 요청과 현재 상태를 여기에서 확인할 수 있습니다.</p>
          </div>
        </div>
      )}
      {cell.runs_truncated && (
        <p className="muted">최대 50개 기록을 표시합니다. 전체 이력 조회는 후속 연결 대상입니다.</p>
      )}
    </section>
  );
}
export function Work({ cell }: { cell: CellOverview }) {
  return (
    <section className="panel records">
      <div className="section-heading">
        <div>
          <p className="eyebrow">OPERATION EVIDENCE</p>
          <h3>작업의 결과와 자원 인계</h3>
        </div>
      </div>
      <p className="muted">동작 관측, 결과 판정, 자원 인계를 각각 확인합니다.</p>
      {!cell.work.length ? (
        <p className="empty-inline">기록된 장비 작업이 없습니다.</p>
      ) : (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                <th>작업</th>
                <th>동작 관측</th>
                <th>결과</th>
                <th>기록 무결성</th>
                <th>자원 처분</th>
              </tr>
            </thead>
            <tbody>
              {cell.work.map((w) => (
                <tr key={w.operation.operation_id}>
                  <td className="mono" title={w.operation.operation_id}>
                    {short(w.operation.operation_id)}
                  </td>
                  <td>{text(w.operation.execution_knowledge)}</td>
                  <td>{text(w.operation.outcome)}</td>
                  <td>{text(w.operation.integrity)}</td>
                  <td>{text(w.operation.disposition)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {cell.work_truncated && (
        <p className="muted">
          최대 100개 작업을 표시합니다. 이 목록으로 전체 작업 수를 추정하지 마세요.
        </p>
      )}
    </section>
  );
}
export function Configuration({ cell }: { cell: CellOverview }) {
  const c = cell.cell.value;
  return (
    <section className="panel config-panel">
      <p className="eyebrow">BOUND CONFIGURATION</p>
      <h2>현재 등록된 구성</h2>
      <p className="muted">
        실행에 연결된 문서의 식별값입니다. 변경은 새 구성과 검증 절차로 진행합니다.
      </p>
      <dl className="config-list">
        {[
          ['셀', c.id],
          ['환경', c.environment === 'SIMULATION' ? '모의 환경' : '실장비'],
          ['RX 운영 모드', text(c.mode ?? 'UNKNOWN')],
          ['운전 자격 상태', text(c.commissioning ?? 'UNKNOWN')],
          ['셀 정의', c.definition.sha256],
          ['운전 범위', c.envelope.sha256],
          ['공정', c.recipe.sha256],
          ['현장 구성', c.site_config_digest],
          ['연결 Host', c.hosts.join(', ')],
        ].map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="inset">
        <b>구성 등록과 검증은 분리됩니다</b>
        <p>
          이 화면은 등록된 구성을 조회합니다. 공정 편집, 검증 보고서와 배포 화면은 다음 단계에서
          연결합니다.
        </p>
      </div>
    </section>
  );
}
