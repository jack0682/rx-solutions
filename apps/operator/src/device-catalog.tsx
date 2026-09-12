import { z } from 'zod';
import { count, digest, downloadJson, type Intake } from './package-schema';
const artifact = z.object({ sha256: digest, schema_id: z.string(), size_bytes: count });
const signed = z
  .string()
  .regex(/^(0|-?[1-9][0-9]*)$/)
  .refine((v) => BigInt(v) >= -9223372036854775808n && BigInt(v) <= 9223372036854775807n);
const catalogSchema = z.object({
  schema: z.literal('rx.device-operation-catalog.v1'),
  installation: z.uuid(),
  cell: z.string(),
  target: z.string(),
  environment: z.enum(['SIMULATION', 'PHYSICAL']),
  profile_digest: digest,
  condition_ids: z.array(z.string()),
  operations: z.record(
    z.string(),
    z
      .object({
        target: z.string(),
        kind: z.string(),
        profile_digest: digest,
        resource_set: z.array(z.string()),
        execution_timeout_ms: count,
        completion_rule: z.string(),
      })
      .passthrough(),
  ),
  outcomes: z
    .object({
      schema: z.literal('rx.native-outcome-table.v1'),
      profile_digest: digest,
      completion_rule: z.string(),
      cases: z.array(
        z.object({
          status_schema: z.string(),
          statuses: z.array(signed),
          conclusion: z.enum(['SUCCEEDED', 'FAILED', 'CANCELED']),
        }),
      ),
    })
    .nullable(),
  documents: z.record(z.string(), z.object({ path: z.string(), artifact })),
});
export const deviceCatalogSchema = z
  .object({
    cell: z.string(),
    intake: z.uuid(),
    object: z.object({ manifest: digest, signature: digest }),
    reference: artifact.nullable(),
    catalog: catalogSchema.nullable(),
    review_context_current: z.boolean(),
    content_reverification_required: z.literal(true),
    manufacturer_validation_required: z.literal(true),
    activation_authorized: z.literal(false),
  })
  .superRefine((v, ctx) => {
    const c = v.catalog;
    if (
      !!c !== !!v.reference ||
      (c &&
        (c.cell !== v.cell ||
          v.reference?.schema_id !== 'rx.device-operation-catalog.v1' ||
          Object.values(c.operations).some(
            (i) => i.profile_digest !== c.profile_digest || i.target !== c.target,
          ) ||
          (c.outcomes &&
            (c.outcomes.profile_digest !== c.profile_digest ||
              Object.values(c.operations).some(
                (i) => i.completion_rule !== c.outcomes?.completion_rule,
              )))))
    )
      ctx.addIssue({ code: 'custom', message: 'catalog correlation' });
  });
export type DeviceCatalogDetail = z.infer<typeof deviceCatalogSchema>;
export function correlateDeviceCatalog(value: unknown, intake: Intake): DeviceCatalogDetail {
  const d = deviceCatalogSchema.parse(value);
  if (
    d.cell !== intake.cell ||
    d.intake !== intake.id ||
    d.object.manifest !== intake.object.manifest ||
    d.object.signature !== intake.object.signature ||
    (d.reference?.sha256 ?? null) !== (intake.device_catalog?.sha256 ?? null) ||
    (d.reference?.schema_id ?? null) !== (intake.device_catalog?.schema_id ?? null) ||
    (d.reference?.size_bytes ?? null) !== (intake.device_catalog?.size_bytes ?? null)
  )
    throw Error('device intake correlation');
  return d;
}
const meanings = { SUCCEEDED: '성공', FAILED: '실패', CANCELED: '취소 완료' };
export function DeviceCatalogPanel({
  detail,
  intake,
  fresh,
}: {
  detail: DeviceCatalogDetail | null;
  intake: Intake;
  fresh: boolean;
}) {
  if (!detail || detail.intake !== intake.id)
    return (
      <section className="panel">
        <h3>장비 작업 선언</h3>
        <p>반입된 선언 내용을 확인하고 있습니다.</p>
      </section>
    );
  const c = detail.catalog;
  return (
    <section className="panel device-catalog" aria-label="장비 작업 선언">
      <div className="section-heading">
        <h3>장비 작업 선언</h3>
        <span className="pill">장비 검증 필요</span>
      </div>
      <p>패키지가 선언한 작업입니다. 실제 장비 검증과 셀 구성 적용은 별도로 진행합니다.</p>
      {(!fresh || !detail.review_context_current) && (
        <p role="status" className="notice">
          현재 셀·반입 정책과 다시 대조해야 하는 보관 자료입니다.
        </p>
      )}
      {!c ? (
        <p>이 패키지에는 공통 작업 선언 자료가 없습니다.</p>
      ) : (
        <>
          <dl className="facts">
            <div>
              <dt>대상 장비</dt>
              <dd>{c.target}</dd>
            </div>
            <div>
              <dt>선언 환경</dt>
              <dd>{c.environment === 'SIMULATION' ? '모의 환경' : '실물 환경 선언'}</dd>
            </div>
            <div>
              <dt>필요 조건</dt>
              <dd>{c.condition_ids.join(', ')}</dd>
            </div>
          </dl>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>작업</th>
                  <th>작업 종류</th>
                  <th>사용 자원</th>
                  <th>시간 제한</th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(c.operations).map(([id, i]) => (
                  <tr key={id}>
                    <td>{id}</td>
                    <td>
                      {i.kind === 'FINITE_ACTION'
                        ? '유한 작업'
                        : i.kind === 'ENSURE_STATE'
                          ? '상태 도달'
                          : i.kind}
                    </td>
                    <td>{i.resource_set.join(', ')}</td>
                    <td>{i.execution_timeout_ms} ms</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <h4>결과 해석 선언</h4>
          {c.outcomes ? (
            <ul>
              {c.outcomes.cases.map((row, index) => (
                <li key={index}>
                  <code>{row.status_schema}</code> · 코드 {row.statuses.join(', ')} →{' '}
                  {meanings[row.conclusion]}
                </li>
              ))}
            </ul>
          ) : (
            <p>공통 native 결과 대응표가 없습니다.</p>
          )}
          <p className="muted">
            알 수 없는 결과를 완료로 간주하지 않습니다. 완료 기록만으로 소재 지지나 자원 인계가
            확인되지는 않습니다.
          </p>
          <details>
            <summary>원본 연결과 선언 전체 보기</summary>
            <pre>{JSON.stringify(detail, null, 2)}</pre>
          </details>
          <button
            onClick={() => downloadJson(detail, `rx-device-declaration-${detail.intake}.json`)}
          >
            장비 선언 자료 내려받기
          </button>
        </>
      )}
    </section>
  );
}
