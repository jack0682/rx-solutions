import { useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import {
  definitionPageSchema,
  pointPageSchema,
  refKey,
  type DefinitionSummary,
  type DefinitionView,
} from './definition-schema';
import type { z } from './schema-runtime';

export function DefinitionPoints({ view, dirty }: { view: DefinitionView; dirty: boolean }) {
  const subject = view.version.definition.reference;
  const [rules, setRules] = useState<DefinitionSummary[]>([]);
  const [selected, setSelected] = useState('');
  const [page, setPage] = useState<z.infer<typeof pointPageSchema> | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => {
    const controller = new AbortController();
    void (async () => {
      let after: string | null = null;
      const result: DefinitionSummary[] = [];
      do {
        const params = new URLSearchParams({
          catalog: subject.catalog,
          kind: 'POINT_PATTERN',
          archived: 'false',
        });
        if (after) params.set('after', after);
        const value = definitionPageSchema.parse(
          await api(`/api/v1/definitions?${params}`, { signal: controller.signal }),
        );
        if (value.catalog !== subject.catalog) throw new Error('Rule catalog differs');
        result.push(...value.definitions);
        after = value.next;
      } while (after && !controller.signal.aborted);
      if (!controller.signal.aborted) setRules(result);
    })().catch((e) => {
      if (!controller.signal.aborted) setError(explain(e));
    });
    return () => {
      controller.abort();
      generation.current++;
    };
  }, [subject.catalog]);
  async function load(offset: string) {
    const rule = rules.find((r) => refKey(r.reference) === selected)?.reference;
    if (!rule) return;
    const g = ++generation.current;
    setBusy(true);
    setError('');
    setPage(null);
    try {
      const value = pointPageSchema.parse(
        await api('/api/v1/definition-points', {
          body: { subject, rule, offset, limit: 100 },
        }),
      );
      if (
        refKey(value.subject) !== refKey(subject) ||
        refKey(value.rule) !== refKey(rule) ||
        value.offset !== offset
      )
        throw new Error('Point report differs from requested versions');
      if (g === generation.current) setPage(value);
    } catch (e) {
      if (g === generation.current) setError(explain(e));
    } finally {
      if (g === generation.current) setBusy(false);
    }
  }
  return (
    <section className="definition-effective" aria-label="Derived points">
      <h4>Derived points from the saved revision</h4>
      {dirty && <p>Unsaved edits are not included. Save before recalculating.</p>}
      <label>
        Point rule
        <select
          value={selected}
          disabled={busy}
          onChange={(e) => {
            generation.current++;
            setSelected(e.target.value);
            setPage(null);
            setError('');
          }}
        >
          <option value="">Select a saved rule</option>
          {rules.map((r) => (
            <option key={refKey(r.reference)} value={refKey(r.reference)}>
              {r.label} · r{r.reference.revision}
            </option>
          ))}
        </select>
      </label>
      <button disabled={!selected || busy || dirty} onClick={() => void load('0')}>
        Calculate points
      </button>
      {error && <p role="alert">{error}</p>}
      {page && (
        <>
          <p>
            {page.total} points · {page.unit ?? 'No unit'} · Frame: {page.frame ?? 'Missing'}
          </p>
          <p>
            Orientation [x, y, z, w]: {page.orientation_xyzw?.join(', ') ?? 'Missing'} · unitless
          </p>
          <small>
            Subject {page.subject.id} r{page.subject.revision} · Rule {page.rule.id} r
            {page.rule.revision}
          </small>
          {page.violations.map((v, i) => (
            <p role="alert" key={i}>
              {v.location}: {v.message} ({v.code})
            </p>
          ))}
          <table>
            <thead>
              <tr>
                <th>Index</th>
                <th>Axis indices</th>
                <th>Position ({page.unit})</th>
              </tr>
            </thead>
            <tbody>
              {page.points.map((p) => (
                <tr key={p.index}>
                  <td>{p.index}</td>
                  <td>{p.indices.join(', ')}</td>
                  <td>{p.position.join(', ')}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {page.next !== null && (
            <button disabled={busy || dirty} onClick={() => void load(page.next!)}>
              Next points
            </button>
          )}
        </>
      )}
    </section>
  );
}
