import { useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import {
  catalogPageSchema,
  definitionPageSchema,
  definitionViewSchema,
  refKey,
  type Catalog,
  type DefinitionRef,
  type DefinitionSummary,
} from './definition-schema';
import { DefinitionName } from './definition-name';
import {
  workflowModelSchema,
  workflowsSchema,
  workflowReportsSchema,
  workflowReceiptSchema,
  parseQuantity,
  quantityText,
  violationNode,
  type WorkflowModel,
  type WorkflowReceipt,
  type WorkflowRequest,
} from './workflow-schema';

type Props = {
  principal: string;
  terminal?: string | null;
  canEdit: boolean;
  locked: boolean;
  receipt: { key: string; value: WorkflowReceipt } | null;
  onSubmit: (command: WorkflowRequest) => Promise<void>;
};
export function WorkflowResolution({
  principal,
  terminal,
  canEdit,
  locked,
  receipt,
  onSubmit,
}: Props) {
  const [catalogs, setCatalogs] = useState<Catalog[]>([]);
  const [catalog, setCatalog] = useState('');
  const [models, setModels] = useState<Array<{ reference: DefinitionRef; label: string }>>([]);
  const [model, setModel] = useState<WorkflowModel | null>(null);
  const [definitions, setDefinitions] = useState<DefinitionSummary[]>([]);
  const [contexts, setContexts] = useState<Record<string, DefinitionRef[]>>({});
  const [slot, setSlot] = useState('0');
  const [node, setNode] = useState('');
  const [property, setProperty] = useState('');
  const [input, setInput] = useState('');
  const [unit, setUnit] = useState('');
  const [overrides, setOverrides] = useState<WorkflowRequest['overrides']>({});
  const [result, setResult] = useState<WorkflowReceipt | null>(null);
  const [selected, setSelected] = useState('');
  const [reportModel, setReportModel] = useState<{
    reference: DefinitionRef;
    label: string;
  } | null>(null);
  const [history, setHistory] = useState<
    Array<{ reference: DefinitionRef; workflow: DefinitionRef; slot_index: string; status: string }>
  >([]);
  const [historyNext, setHistoryNext] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const seen = useRef('');
  const acl = catalogs.find((c) => c.id === catalog);
  const editable =
    canEdit &&
    !!acl &&
    !acl.archived &&
    (acl.owner === principal || acl.members[principal] === 'EDIT') &&
    (!terminal || acl.terminals.includes(terminal));
  useEffect(() => {
    const control = new AbortController();
    void (async () => {
      let after: string | null = null;
      const all: Catalog[] = [];
      do {
        const p = catalogPageSchema.parse(
          await api(
            `/api/v1/definition-catalogs${after ? `?${new URLSearchParams({ after })}` : ''}`,
            { signal: control.signal },
          ),
        );
        all.push(...p.catalogs);
        after = p.next;
      } while (after && !control.signal.aborted);
      if (!control.signal.aborted) {
        setCatalogs(all);
        if (all.length === 1) setCatalog(all[0].id);
      }
    })().catch((e) => {
      if (!control.signal.aborted) setError(explain(e));
    });
    return () => control.abort();
  }, []);
  async function reports(after?: string) {
    if (!catalog) return;
    const g = generation.current;
    const params = new URLSearchParams({ catalog });
    if (after) params.set('after', after);
    const page = workflowReportsSchema.parse(await api(`/api/v1/workflow-resolutions?${params}`));
    if (page.catalog !== catalog) throw new Error('Report catalog differs');
    if (g === generation.current) {
      setHistory((old) => (after ? [...old, ...page.reports] : page.reports));
      setHistoryNext(page.next);
    }
  }
  useEffect(() => {
    const g = ++generation.current;
    const control = new AbortController();
    setModel(null);
    setResult(null);
    setModels([]);
    setDefinitions([]);
    setHistory([]);
    setError('');
    if (!catalog) return () => control.abort();
    setBusy(true);
    void (async () => {
      const allModels: Array<{ reference: DefinitionRef; label: string }> = [];
      const allDefinitions: DefinitionSummary[] = [];
      let after: string | null = null;
      do {
        const params = new URLSearchParams({ catalog });
        if (after) params.set('after', after);
        const p = workflowsSchema.parse(
          await api(`/api/v1/workflow-models?${params}`, { signal: control.signal }),
        );
        if (p.catalog !== catalog) throw new Error('Workflow catalog differs');
        allModels.push(...p.workflows);
        after = p.next;
      } while (after && !control.signal.aborted);
      after = null;
      do {
        const params = new URLSearchParams({ catalog, archived: 'false' });
        if (after) params.set('after', after);
        const p = definitionPageSchema.parse(
          await api(`/api/v1/definitions?${params}`, { signal: control.signal }),
        );
        if (p.catalog !== catalog) throw new Error('Definition catalog differs');
        allDefinitions.push(...p.definitions);
        after = p.next;
      } while (after && !control.signal.aborted);
      if (g === generation.current && !control.signal.aborted) {
        setModels(allModels);
        setDefinitions(allDefinitions);
        await reports();
      }
    })()
      .catch((e) => {
        if (g === generation.current && !control.signal.aborted) setError(explain(e));
      })
      .finally(() => {
        if (g === generation.current) setBusy(false);
      });
    return () => {
      control.abort();
      generation.current++;
    };
  }, [catalog]);
  useEffect(() => {
    if (!receipt || seen.current === receipt.key) return;
    seen.current = receipt.key;
    if (receipt.value.reference.catalog !== catalog) return;
    setResult(receipt.value);
    setSelected(receipt.value.report.steps[0]?.node ?? '');
    void reports().catch((e) => setError(explain(e)));
  }, [receipt, catalog]);
  async function openModel(reference: DefinitionRef) {
    setBusy(true);
    setError('');
    const g = ++generation.current;
    try {
      const params = new URLSearchParams({
        catalog: reference.catalog,
        id: reference.id,
        revision: reference.revision,
      });
      const value = workflowModelSchema.parse(await api(`/api/v1/workflow-model?${params}`));
      if (refKey(value.reference) !== refKey(reference))
        throw new Error('Workflow revision differs');
      if (g === generation.current) {
        setModel(value);
        setContexts(value.spec.defaults);
        setOverrides({});
        setResult(null);
        setNode(value.spec.steps[0]?.id ?? '');
        setProperty('');
        setInput('');
        setUnit('');
      }
    } catch (e) {
      if (g === generation.current) setError(explain(e));
    } finally {
      if (g === generation.current) setBusy(false);
    }
  }
  const task = model?.spec.tasks[model.spec.steps.find((s) => s.id === node)?.task ?? ''];
  useEffect(() => {
    const control = new AbortController();
    setUnit('');
    const ref = task?.properties[property]?.property;
    if (!ref) return () => control.abort();
    void (async () => {
      const params = new URLSearchParams({
        catalog: ref.catalog,
        id: ref.id,
        revision: ref.revision,
      });
      const p = definitionViewSchema.parse(
        await api(`/api/v1/definition?${params}`, { signal: control.signal }),
      ).version.definition;
      if (refKey(p.reference) !== refKey(ref) || p.body.kind !== 'PROPERTY')
        throw new Error('Property differs');
      if (!control.signal.aborted) setUnit(p.body.specification.unit);
    })().catch((e) => {
      if (!control.signal.aborted) setError(explain(e));
    });
    return () => control.abort();
  }, [node, property, model]);
  async function openReport(reference: DefinitionRef) {
    setBusy(true);
    setError('');
    const g = ++generation.current;
    try {
      const params = new URLSearchParams({ catalog: reference.catalog, id: reference.id });
      const value = workflowReceiptSchema.parse(await api(`/api/v1/workflow-resolution?${params}`));
      if (refKey(value.reference) !== refKey(reference))
        throw new Error('Resolution reference differs');
      if (g === generation.current) {
        setResult(value);
        setSelected(value.report.steps[0]?.node ?? '');
      }
    } catch (e) {
      if (g === generation.current) setError(explain(e));
    } finally {
      if (g === generation.current) setBusy(false);
    }
  }
  useEffect(() => {
    const control = new AbortController();
    setReportModel(null);
    const reference = result?.report.request.workflow;
    if (!reference) return () => control.abort();
    const params = new URLSearchParams({
      catalog: reference.catalog,
      id: reference.id,
      revision: reference.revision,
    });
    void api(`/api/v1/workflow-model?${params}`, { signal: control.signal })
      .then((response) => {
        const value = workflowModelSchema.parse(response);
        if (refKey(value.reference) !== refKey(reference))
          throw new Error('Workflow revision differs');
        if (!control.signal.aborted) setReportModel(value);
      })
      .catch((e) => {
        if (!control.signal.aborted) setError(explain(e));
      });
    return () => control.abort();
  }, [result]);
  const names = Object.fromEntries(
    (result?.report.definitions ?? []).map((d) => [refKey(d.reference), d.label]),
  );
  if (reportModel) names[refKey(reportModel.reference)] = reportModel.label;
  const step = result?.report.steps.find((s) => s.node === selected);
  return (
    <section className="panel" aria-label="Workflow resolution">
      <h2>Versioned workflow resolution</h2>
      <p>
        Resolve package-defined tasks on the server. These reports do not publish or execute device
        work.
      </p>
      {error && (
        <p role="alert" className="notice error">
          {error}
        </p>
      )}
      <label>
        Authoring catalog
        <select
          disabled={locked || busy}
          value={catalog}
          onChange={(e) => setCatalog(e.target.value)}
        >
          <option value="">Select catalog</option>
          {catalogs.map((c) => (
            <option key={c.id} value={c.id}>
              {c.title}
            </option>
          ))}
        </select>
      </label>
      <label>
        Workflow model
        <select
          disabled={locked || busy}
          value={model ? refKey(model.reference) : ''}
          onChange={(e) => {
            const v = models.find((m) => refKey(m.reference) === e.target.value);
            if (v) void openModel(v.reference);
          }}
        >
          <option value="">Select saved workflow</option>
          {models.map((m) => (
            <option key={refKey(m.reference)} value={refKey(m.reference)}>
              {m.label} · r{m.reference.revision}
            </option>
          ))}
        </select>
      </label>
      {model && (
        <fieldset disabled={locked || busy || !editable}>
          <legend>Resolution inputs · saved model r{model.reference.revision}</legend>
          {Object.entries(model.spec.contexts).map(([key, s]) => (
            <label key={key}>
              {s.label} context
              <select
                aria-label={`${key} context`}
                multiple={s.multiple}
                value={
                  s.multiple
                    ? (contexts[key] ?? []).map(refKey)
                    : contexts[key]?.[0]
                      ? refKey(contexts[key][0])
                      : ''
                }
                onChange={(e) => {
                  const keys = Array.from(e.target.selectedOptions, (option) => option.value);
                  const selected = definitions.filter((v) => keys.includes(refKey(v.reference)));
                  setContexts({ ...contexts, [key]: selected.map((d) => d.reference) });
                  setResult(null);
                }}
              >
                {!s.multiple && <option value="">Not bound</option>}
                {definitions
                  .filter((d) =>
                    s.kind === 'OBJECT'
                      ? d.kind === 'OBJECT_MODEL'
                      : ['RESOURCE_MODEL', 'RESOURCE_INSTANCE'].includes(d.kind),
                  )
                  .map((d) => (
                    <option key={refKey(d.reference)} value={refKey(d.reference)}>
                      {d.label} · r{d.reference.revision}
                    </option>
                  ))}
              </select>
            </label>
          ))}
          <label>
            Slot index
            <input
              type="number"
              min="0"
              step="1"
              value={slot}
              onChange={(e) => {
                setSlot(e.target.value);
                setResult(null);
              }}
            />
          </label>
          <details>
            <summary>Explicit overrides</summary>
            <label>
              Override node
              <select
                value={node}
                onChange={(e) => {
                  setNode(e.target.value);
                  setProperty('');
                }}
              >
                {model.spec.steps.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.id}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Override property
              <select value={property} onChange={(e) => setProperty(e.target.value)}>
                <option value="">Select property</option>
                {Object.entries(task?.properties ?? {})
                  .filter(([, p]) => p.sources.some((s) => s.kind === 'OVERRIDE'))
                  .map(([k]) => (
                    <option key={k}>{k}</option>
                  ))}
              </select>
            </label>
            <label>
              Override value
              <input
                value={input}
                placeholder="60 or 20..60"
                onChange={(e) => setInput(e.target.value)}
              />
            </label>
            <label>
              Override unit
              <input value={unit} onChange={(e) => setUnit(e.target.value)} />
            </label>
            <button
              type="button"
              disabled={!property || !input || !unit}
              onClick={() => {
                try {
                  const q = parseQuantity(input, unit);
                  setOverrides({ ...overrides, [node]: { ...overrides[node], [property]: q } });
                  setResult(null);
                  setError('');
                } catch {
                  setError('Enter a JSON value or numeric range with an explicit unit.');
                }
              }}
            >
              Add override
            </button>
            {Object.entries(overrides).flatMap(([n, props]) =>
              Object.entries(props).map(([p, q]) => (
                <p key={`${n}/${p}`}>
                  {n}.{p}: {quantityText(q)} {q.unit}{' '}
                  <button
                    type="button"
                    onClick={() => {
                      const next = structuredClone(overrides);
                      delete next[n][p];
                      if (!Object.keys(next[n]).length) delete next[n];
                      setOverrides(next);
                      setResult(null);
                    }}
                  >
                    Remove
                  </button>
                </p>
              )),
            )}
          </details>
          <button
            className="primary"
            onClick={() => {
              if (!/^(0|[1-9][0-9]*)$/.test(slot)) {
                setError('Use a nonnegative integer slot index.');
                return;
              }
              void onSubmit({
                workflow: model.reference,
                contexts,
                property_sets: [],
                overrides,
                inputs: {},
                slot_index: slot,
              });
            }}
          >
            Resolve saved model
          </button>
        </fieldset>
      )}
      {catalog && (
        <details>
          <summary>Saved resolution reports</summary>
          <button
            disabled={busy || locked}
            onClick={() => void reports().catch((e) => setError(explain(e)))}
          >
            Reload reports
          </button>
          {history.map((h) => (
            <p key={h.reference.id}>
              <button disabled={busy || locked} onClick={() => void openReport(h.reference)}>
                Slot {h.slot_index} · {h.status} · {h.reference.id}
              </button>
            </p>
          ))}
          {historyNext && (
            <button
              disabled={busy || locked}
              onClick={() => void reports(historyNext).catch((e) => setError(explain(e)))}
            >
              More reports
            </button>
          )}
        </details>
      )}
      {result && (
        <section aria-label="Stored workflow resolution">
          <h3>{result.report.status}</h3>
          <p>
            Immutable report {result.reference.id} · model r
            {result.report.request.workflow.revision} · slot {result.report.request.slot_index}
          </p>
          <p>
            Values below belong to this stored request. Capability and Skill matches are
            declarations, not operating qualification.
          </p>
          {result.report.violations.map((v, i) => (
            <p key={i} className="notice error">
              <button
                onClick={() => {
                  const node = violationNode(v.location);
                  if (node) setSelected(node);
                }}
              >
                {v.location}
              </button>
              : {v.message} ({v.code})
            </p>
          ))}
          <div className="definition-list">
            {result.report.steps.map((s) => (
              <button
                key={s.node}
                className={selected === s.node ? 'selected' : ''}
                onClick={() => setSelected(s.node)}
              >
                {s.node} · {s.label}
              </button>
            ))}
          </div>
          {step && (
            <section aria-label={`Resolved node ${step.node}`}>
              <h4>
                {step.node} · {step.label}
              </h4>
              <table>
                <thead>
                  <tr>
                    <th>Property</th>
                    <th>Value / range</th>
                    <th>Unit / frame</th>
                    <th>Selected source</th>
                  </tr>
                </thead>
                <tbody>
                  {Object.entries(step.properties).map(([key, value]) => (
                    <tr key={key}>
                      <td>{key}</td>
                      <td>{quantityText(value.value)}</td>
                      <td>
                        {value.value.unit}
                        {value.frame ? ` · ${value.frame}` : ''}
                      </td>
                      <td>
                        {value.selected_source}
                        <details>
                          <summary>Sources and rules</summary>
                          {value.origins.map((o, i) => (
                            <p key={i}>
                              {o.kind} ·{' '}
                              {o.reference ? (
                                <DefinitionName reference={o.reference} names={names} />
                              ) : (
                                'request input'
                              )}{' '}
                              · {o.path} · {quantityText(o.value)} {o.value.unit}
                            </p>
                          ))}
                        </details>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <details>
                <summary>Declared capability and Skill mapping</summary>
                {Object.entries(step.declared_capabilities).map(([k, v]) => (
                  <p key={k}>
                    {k}: {quantityText(v.value)}
                  </p>
                ))}
                {step.skills.map((s, i) => (
                  <p key={i}>
                    {quantityText(s.implementation.value)} · {quantityText(s.version.value)} →{' '}
                    {s.primitive}
                  </p>
                ))}
                <p>
                  Done contract: {step.done.observation} = {quantityText(step.done.equals)}.
                  Failure: {step.on_failure}; unknown: {step.on_unknown}.
                </p>
              </details>
            </section>
          )}
        </section>
      )}
    </section>
  );
}
