import { useEffect, useState, useRef } from 'react';
import { api, explain } from './api';
import {
  bindingCatalogSchema,
  bindingViewSchema,
  compileInputSchema,
  type BindingCatalog,
  type BindingEdit,
  type BindingVersion,
  type BindingView,
} from './draft-bindings-schema';
import type { DraftBuffer } from './draft-schema';
import { short } from './labels';
export function DraftBindings({
  buffer,
  onBuffer,
  canEdit,
  canSave,
  receipt,
  onSave,
}: {
  buffer: DraftBuffer;
  onBuffer: (v: DraftBuffer) => void;
  canEdit: boolean;
  canSave: boolean;
  receipt: BindingVersion | null;
  onSave: (edit: BindingEdit) => Promise<unknown>;
}) {
  const [catalog, setCatalog] = useState<BindingCatalog | null>(null);
  const [view, setView] = useState<BindingView | null>(null);
  const [error, setError] = useState('');
  const [working, setWorking] = useState(false);
  const generation = useRef(0);
  async function load() {
    if (!buffer.expected) return;
    const request = ++generation.current;
    const expectedDraft = buffer.id;
    const expectedCell = buffer.cell;
    setWorking(true);
    setError('');
    try {
      const b = await api(
        `/api/v1/process-draft-bindings?cell=${encodeURIComponent(buffer.cell)}&id=${buffer.id}`,
      );
      const current = bindingViewSchema.parse(b);
      if (current.cell !== expectedCell || current.draft !== expectedDraft)
        throw new Error('binding response identity');
      if (request !== generation.current) return;
      setView(current);
      const plans = current.binding?.device_plans ?? [];
      const a = await api(
        `/api/v1/process-draft/binding-options${plans.length ? '' : `?cell=${encodeURIComponent(buffer.cell)}`}`,
        plans.length ? { body: { cell: buffer.cell, device_plans: plans } } : {},
      );
      if (request !== generation.current) return;
      const options = bindingCatalogSchema.parse(a);
      if (
        options.cell !== expectedCell ||
        current.cell !== expectedCell ||
        current.draft !== expectedDraft
      )
        throw new Error('binding response identity');
      if (options.catalog_digest !== current.current_catalog_digest)
        throw new Error('binding context changed during read');
      setCatalog(options);
      setView(current);
    } catch (e) {
      if (request === generation.current) {
        setError(explain(e));
        setCatalog(null);
      }
    } finally {
      if (request === generation.current) setWorking(false);
    }
  }
  useEffect(() => {
    void load();
    return () => {
      generation.current++;
    };
  }, [buffer.id, buffer.expected, receipt]);
  const sourceReady =
    !!buffer.expected &&
    !buffer.dirty &&
    buffer.sourceText == null &&
    buffer.conditionText == null &&
    !!buffer.validation?.structurally_valid;
  const required = buffer.validation?.required_bindings ?? [];
  const edit = buffer.bindingEdit;
  const selections = edit?.selections ?? view?.binding?.selections ?? {};
  const extra = Object.keys(selections).filter((k) => !required.includes(k));
  const contextCurrent =
    !!catalog &&
    !!view &&
    view.current_source_revision === buffer.expected &&
    catalog.catalog_digest === view.current_catalog_digest;
  function begin(): BindingEdit | null {
    if (!catalog || !view || !buffer.expected) return null;
    return {
      sourceRevision: buffer.expected,
      catalogDigest: catalog.catalog_digest,
      expected: view.binding?.revision ?? null,
      selections: { ...selections },
      devicePlans: catalog.device_plans,
    };
  }
  function choose(binding: string, step: string) {
    if (!canEdit || !sourceReady || !contextCurrent) return;
    const next = edit ?? begin();
    if (!next) return;
    const values = { ...next.selections };
    if (step) values[binding] = step;
    else delete values[binding];
    onBuffer({ ...buffer, bindingEdit: { ...next, selections: values } });
  }
  const changedContext =
    !!edit &&
    (edit.sourceRevision !== buffer.expected ||
      edit.catalogDigest !== catalog?.catalog_digest ||
      edit.expected !== (view?.binding?.revision ?? null));
  async function exportBundle() {
    if (!view?.binding || !buffer.expected) return;
    setWorking(true);
    setError('');
    try {
      const value = compileInputSchema.parse(
        await api(
          `/api/v1/process-draft-compile-input?cell=${encodeURIComponent(buffer.cell)}&id=${buffer.id}&source_revision=${buffer.expected}&binding_revision=${view.binding.revision}`,
        ),
      );
      if (
        value.draft !== buffer.id ||
        value.source_revision !== buffer.expected ||
        value.binding_revision !== view.binding.revision
      )
        throw new Error('compile input correlation');
      const blob = URL.createObjectURL(
        new Blob([JSON.stringify(value, null, 2) + '\n'], { type: 'application/json' }),
      );
      const a = document.createElement('a');
      a.href = blob;
      a.download = 'process-compile-input.json';
      a.click();
      URL.revokeObjectURL(blob);
    } catch (e) {
      setError(explain(e));
    } finally {
      setWorking(false);
    }
  }
  return (
    <section className="panel draft-bindings">
      {!!view?.binding?.device_plans.length && (
        <p className="notice">
          This workflow draft uses a candidate from a reviewed device change proposal. It has not
          been applied to the running configuration.
        </p>
      )}
      <div className="section-heading">
        <div>
          <p className="eyebrow">ACTION BINDINGS</p>
          <h3>Workflow and device operation bindings</h3>
        </div>
        <span
          className={`badge ${!edit && view?.binding?.complete && !view.stale.length ? 'good' : 'warning'}`}
        >
          {edit
            ? 'Unsaved selections'
            : !view?.binding
              ? 'Binding selections required'
              : view.stale.length
                ? 'Configuration review required'
                : view.binding.complete
                  ? 'All bindings selected'
                  : 'Some operations are unbound'}
        </span>
      </div>
      <p className="muted">
        Select from operations registered in the current cell. Saving bindings and exporting
        compiler input do not execute devices or grant verification approval.
      </p>
      {!sourceReady && (
        <p className="notice">
          Save the workflow source first and check its structural validation results.
        </p>
      )}
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {view?.stale.length !== 0 && view?.stale.length && (
        <p className="notice warning">
          {view.stale.includes('SOURCE_CHANGED')
            ? 'The saved bindings refer to different workflow content than the current draft. '
            : ''}
          {view.stale.includes('CATALOG_CHANGED')
            ? 'The device operation configuration has changed. '
            : ''}
          Review the existing selections and save them again against the current configuration.
        </p>
      )}
      {changedContext && (
        <p className="notice warning">
          The editing baseline has changed. Compare the new list with your selections and review
          against the current baseline.
        </p>
      )}
      <div className="binding-table">
        {required.map((binding) => {
          const selected = catalog?.candidates.find((c) => c.step === selections[binding]);
          return (
            <div className="binding-row" key={binding}>
              <label>
                {binding}
                <select
                  aria-label={`${binding} Device operation`}
                  value={selections[binding] ?? ''}
                  disabled={!canEdit || !sourceReady || !contextCurrent || working}
                  onChange={(e) => choose(binding, e.target.value)}
                >
                  <option value="">Select operation</option>
                  {catalog?.candidates.map((c) => (
                    <option key={c.step} value={c.step}>
                      {c.step} · {c.host} / {c.target}
                    </option>
                  ))}
                  {selections[binding] && !selected && (
                    <option value={selections[binding]}>
                      Previous selection · {selections[binding]}
                    </option>
                  )}
                </select>
              </label>
              {selected && (
                <div className="binding-info">
                  <span>{selected.host}</span>
                  <span>Target {selected.target}</span>
                  <small>
                    Resources {selected.resources.join(', ')} · intent{' '}
                    {short(selected.intent_digest)}
                  </small>
                </div>
              )}
            </div>
          );
        })}
      </div>
      {extra.length > 0 && (
        <div className="notice warning">
          Previous selections absent from the current workflow: {extra.join(', ')}
          <button
            disabled={!canEdit || working}
            onClick={() => {
              const next = edit ?? begin();
              if (next) {
                const values = { ...next.selections };
                for (const name of extra) delete values[name];
                onBuffer({ ...buffer, bindingEdit: { ...next, selections: values } });
              }
            }}
          >
            Remove previous selection
          </button>
        </div>
      )}
      <div className="draft-actions">
        <button
          disabled={!canEdit || !sourceReady || !contextCurrent || working}
          onClick={() => {
            const next = begin();
            if (next) onBuffer({ ...buffer, bindingEdit: next });
          }}
        >
          Review against current baseline
        </button>
        <button
          className="primary"
          disabled={!canSave || !sourceReady || !edit || changedContext || working}
          onClick={() => edit && void onSave(edit)}
        >
          Save bindings
        </button>
        <button
          disabled={!edit || working}
          onClick={() => onBuffer({ ...buffer, bindingEdit: null })}
        >
          Discard selection changes
        </button>
        <button
          disabled={
            !sourceReady || !!edit || !view?.binding?.complete || !!view?.stale.length || working
          }
          onClick={() => void exportBundle()}
        >
          Export compiler input
        </button>
        <button disabled={working || !buffer.expected} onClick={() => void load()}>
          Refresh device operations
        </button>
      </div>
      {view?.binding && (
        <p className="muted">
          Bindings r{view.binding.revision} · source r{view.binding.source_revision} baseline ·
          edited by {view.binding.updated_by}
        </p>
      )}
    </section>
  );
}
