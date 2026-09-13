import { z } from './schema-runtime';
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
const meanings = { SUCCEEDED: 'Success', FAILED: 'Failure', CANCELED: 'Cancellation completed' };
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
        <h3>Device operation declarations</h3>
        <p>Checking the imported declarations.</p>
      </section>
    );
  const c = detail.catalog;
  return (
    <section className="panel device-catalog" aria-label="Device operation declarations">
      <div className="section-heading">
        <h3>Device operation declarations</h3>
        <span className="pill">Device verification required</span>
      </div>
      <p>
        These operations are declared by the package. Physical device verification and application
        to cell configuration are separate steps.
      </p>
      {(!fresh || !detail.review_context_current) && (
        <p role="status" className="notice">
          This archived evidence must be checked again against the current cell and intake policy.
        </p>
      )}
      {!c ? (
        <p>This package contains no common operation declarations.</p>
      ) : (
        <>
          <dl className="facts">
            <div>
              <dt>Target device</dt>
              <dd>{c.target}</dd>
            </div>
            <div>
              <dt>Declared environment</dt>
              <dd>
                {c.environment === 'SIMULATION' ? 'Simulation' : 'Physical environment declared'}
              </dd>
            </div>
            <div>
              <dt>Required conditions</dt>
              <dd>{c.condition_ids.join(', ')}</dd>
            </div>
          </dl>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Operation</th>
                  <th>Operation kind</th>
                  <th>Resources used</th>
                  <th>Time limit</th>
                </tr>
              </thead>
              <tbody>
                {Object.entries(c.operations).map(([id, i]) => (
                  <tr key={id}>
                    <td>{id}</td>
                    <td>
                      {i.kind === 'FINITE_ACTION'
                        ? 'Finite operation'
                        : i.kind === 'ENSURE_STATE'
                          ? 'State reached'
                          : i.kind}
                    </td>
                    <td>{i.resource_set.join(', ')}</td>
                    <td>{i.execution_timeout_ms} ms</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <h4>Outcome interpretation declarations</h4>
          {c.outcomes ? (
            <ul>
              {c.outcomes.cases.map((row, index) => (
                <li key={index}>
                  <code>{row.status_schema}</code> · code {row.statuses.join(', ')} →{' '}
                  {meanings[row.conclusion]}
                </li>
              ))}
            </ul>
          ) : (
            <p>No common native outcome mapping is provided.</p>
          )}
          <p className="muted">
            Unknown outcomes are not treated as completion. A completion record alone does not
            confirm material support or resource handover.
          </p>
          <details>
            <summary>View source references and full declarations</summary>
            <pre>{JSON.stringify(detail, null, 2)}</pre>
          </details>
          <button
            onClick={() => downloadJson(detail, `rx-device-declaration-${detail.intake}.json`)}
          >
            Download device declarations
          </button>
        </>
      )}
    </section>
  );
}
