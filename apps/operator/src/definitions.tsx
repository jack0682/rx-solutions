import { DefinitionName, useDefinitionNames } from './definition-name';
import { provenanceRefs } from './definition-names';
import { DefinitionPoints } from './definition-points';
import { useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import { stableDocument } from './draft-schema';
import { DefinitionFields, kindLabel, newBody } from './definition-fields';
import {
  catalogPageSchema,
  catalogSaveSchema,
  definitionKinds,
  definitionPageSchema,
  definitionViewSchema,
  definitionHistorySchema,
  definitionSaveSchema,
  definitionInput,
  refKey,
  valueText,
  type Catalog,
  type CatalogSave,
  type DefinitionSave,
  type DefinitionView,
  type DefinitionSummary,
  type DefinitionReceipt,
} from './definition-schema';
import type { Pending } from './schema';

type Props = {
  principal: string;
  terminal?: string | null;
  canEdit: boolean;
  canSave: boolean;
  locked: boolean;
  receipt: DefinitionReceipt | null;
  onDirty: (dirty: boolean) => void;
  onSubmit: (
    route: Pending['route'],
    command: Record<string, unknown>,
    label: string,
  ) => Promise<void>;
};
export function Definitions({
  principal,
  terminal,
  canEdit,
  canSave,
  locked,
  receipt,
  onDirty,
  onSubmit,
}: Props) {
  const [catalogs, setCatalogs] = useState<Catalog[]>([]);
  const [catalogNext, setCatalogNext] = useState<string | null>(null);
  const [selected, setSelected] = useState('');
  const [items, setItems] = useState<DefinitionSummary[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [kind, setKind] = useState<(typeof definitionKinds)[number]>('RESOURCE_TYPE');
  const [showArchived, setShowArchived] = useState(false);
  const [form, setForm] = useState<DefinitionSave | null>(null);
  const [saved, setSaved] = useState<DefinitionView | null>(null);
  const [original, setOriginal] = useState('');
  const [catalogForm, setCatalogForm] = useState<CatalogSave | null>(null);
  const [catalogOriginal, setCatalogOriginal] = useState('');
  const [history, setHistory] = useState<DefinitionSummary[]>([]);
  const [historyNext, setHistoryNext] = useState<string | null>(null);
  const [historical, setHistorical] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [revision, setRevision] = useState(0);
  const generation = useRef(0);
  const receiptSeen = useRef('');
  const catalog = catalogs.find((v) => v.id === selected);
  const dirty =
    (!!form && stableDocument(form) !== original) ||
    (!!catalogForm && stableDocument(catalogForm) !== catalogOriginal);
  const editable =
    canEdit &&
    !!catalog &&
    !catalog.archived &&
    (catalog.owner === principal || catalog.members[principal] === 'EDIT') &&
    (!terminal || catalog.terminals.includes(terminal));
  useEffect(() => onDirty(dirty), [dirty, onDirty]);
  const discard = () => !dirty || window.confirm('Discard unsaved definition edits?');
  function clearEditor() {
    setForm(null);
    setSaved(null);
    setOriginal('');
    setCatalogForm(null);
    setCatalogOriginal('');
    setHistory([]);
    setHistoryNext(null);
    setHistorical(false);
    setError('');
  }
  async function loadCatalogs(after?: string, signal?: AbortSignal) {
    const value = catalogPageSchema.parse(
      await api(`/api/v1/definition-catalogs${after ? `?${new URLSearchParams({ after })}` : ''}`, {
        signal,
      }),
    );
    if (signal?.aborted) return;
    setCatalogs((old) => (after ? [...old, ...value.catalogs] : value.catalogs));
    setCatalogNext(value.next);
    setSelected((old) => old || value.catalogs[0]?.id || '');
  }
  useEffect(() => {
    const controller = new AbortController();
    void loadCatalogs(
      undefined,
      AbortSignal.any([controller.signal, AbortSignal.timeout(15000)]),
    ).catch((e) => {
      if (!controller.signal.aborted) setError(explain(e));
    });
    return () => controller.abort();
  }, [revision]);
  async function loadDefinitions(after: string | undefined, g: number, signal?: AbortSignal) {
    if (!selected) return;
    setLoading(true);
    try {
      const params = new URLSearchParams({ catalog: selected });
      if (after) params.set('after', after);
      const value = definitionPageSchema.parse(
        await api(`/api/v1/definitions?${params}`, { signal }),
      );
      if (g !== generation.current || signal?.aborted) return;
      if (value.catalog !== selected) throw new Error('Catalog differs');
      setItems((old) => (after ? [...old, ...value.definitions] : value.definitions));
      setNext(value.next);
    } catch (e) {
      if (g === generation.current)
        setError(
          signal?.aborted ? 'Definition listing timed out. Reload the library.' : explain(e),
        );
    } finally {
      if (g === generation.current) setLoading(false);
    }
  }
  useEffect(() => {
    const g = ++generation.current;
    const controller = new AbortController();
    setItems([]);
    setNext(null);
    setLoading(false);
    void loadDefinitions(
      undefined,
      g,
      AbortSignal.any([controller.signal, AbortSignal.timeout(15000)]),
    );
    return () => {
      generation.current++;
      controller.abort();
    };
  }, [selected, revision]);
  useEffect(() => {
    if (!receipt || receipt.key === receiptSeen.current) return;
    receiptSeen.current = receipt.key;
    if (receipt.route === '/api/v1/definitions') {
      if (form && stableDocument(form) === stableDocument(receipt.command)) {
        const input = definitionInput(receipt.value);
        setForm(input);
        setOriginal(stableDocument(input));
        setSaved(receipt.value);
        setHistorical(false);
      }
    } else if (catalogForm && stableDocument(catalogForm) === stableDocument(receipt.command)) {
      setCatalogForm(null);
      setCatalogOriginal('');
      setSelected(receipt.value.id);
    }
    setRevision((v) => v + 1);
  }, [receipt, form, catalogForm]);
  async function openDefinition(item: DefinitionSummary, historicalVersion = false) {
    if (!discard()) return;
    const g = ++generation.current;
    setLoading(true);
    setError('');
    try {
      const params = new URLSearchParams({
        catalog: selected,
        id: item.reference.id,
        revision: item.reference.revision,
      });
      const view = definitionViewSchema.parse(await api(`/api/v1/definition?${params}`));
      if (g !== generation.current) return;
      if (refKey(view.version.definition.reference) !== refKey(item.reference))
        throw new Error('Selected definition differs');
      const input = definitionInput(view);
      setForm(input);
      setOriginal(stableDocument(input));
      setSaved(view);
      setHistorical(historicalVersion);
      setCatalogForm(null);
      setCatalogOriginal('');
      if (!historicalVersion) {
        setHistory([]);
        setHistoryNext(null);
      }
    } catch (e) {
      if (g === generation.current) setError(explain(e));
    } finally {
      if (g === generation.current) setLoading(false);
    }
  }
  async function loadHistory(before?: string) {
    if (!form) return;
    const g = generation.current;
    const id = form.id;
    setLoading(true);
    setError('');
    try {
      const params = new URLSearchParams({ catalog: selected, id });
      if (before) params.set('before', before);
      const value = definitionHistorySchema.parse(
        await api(`/api/v1/definition-history?${params}`),
      );
      if (g !== generation.current) return;
      if (value.catalog !== selected || value.id !== id) throw new Error('History differs');
      setHistory((old) => (before ? [...old, ...value.versions] : value.versions));
      setHistoryNext(value.next);
    } catch (e) {
      if (g === generation.current) setError(explain(e));
    } finally {
      if (g === generation.current) setLoading(false);
    }
  }
  const options = items.filter((i) => i.reference.id !== form?.id);
  const filtered = items.filter(
    (i) =>
      i.kind === kind &&
      (showArchived || !i.archived) &&
      i.label.toLowerCase().includes(query.toLowerCase()),
  );
  return (
    <section className="definitions-workspace" aria-label="Definitions workspace">
      <header className="panel definition-heading">
        <div>
          <p className="eyebrow">REUSABLE PROCESS KNOWLEDGE</p>
          <h2>Definitions</h2>
          <p>
            Configure parts, trays, site resources and tasks. Saved revisions keep their own values
            and references.
          </p>
        </div>
        <div className="definition-row">
          <label>
            Catalog
            <select
              value={selected}
              disabled={locked || loading}
              onChange={(e) => {
                if (discard()) {
                  clearEditor();
                  setSelected(e.target.value);
                }
              }}
            >
              <option value="">Select a catalog</option>
              {catalogs.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.title}
                  {c.archived ? ' · Archived' : ''}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={!canEdit || locked || loading}
            onClick={() => {
              if (discard()) {
                clearEditor();
                setCatalogForm({
                  id: crypto.randomUUID(),
                  expected: null,
                  title: '',
                  members: {},
                  terminals: terminal ? [terminal] : [],
                  archived: false,
                });
              }
            }}
          >
            New catalog
          </button>
          {catalog && (
            <button
              type="button"
              disabled={locked || loading}
              onClick={() => {
                if (discard()) {
                  clearEditor();
                  const { revision: expected, owner: _owner, ...rest } = catalog;
                  const input = { ...rest, expected };
                  setCatalogForm(input);
                  setCatalogOriginal(stableDocument(input));
                }
              }}
            >
              Catalog access
            </button>
          )}
          {selected && (
            <button
              type="button"
              disabled={loading || locked}
              onClick={() => {
                setError('');
                setRevision((v) => v + 1);
              }}
            >
              Reload library
            </button>
          )}
          {catalogNext && (
            <button
              type="button"
              onClick={() => void loadCatalogs(catalogNext).catch((e) => setError(explain(e)))}
            >
              More catalogs
            </button>
          )}
        </div>
      </header>
      {error && (
        <p className="notice" role="alert">
          {error}
        </p>
      )}
      {catalogForm ? (
        <CatalogEditor
          input={catalogForm}
          editable={canEdit && (!catalogForm.expected || catalog?.owner === principal)}
          locked={locked}
          canSave={canSave}
          onChange={setCatalogForm}
          onCancel={() => {
            if (discard()) clearEditor();
          }}
          onSubmit={async () => {
            const result = catalogSaveSchema.safeParse(catalogForm);
            if (!result.success) {
              setError(
                result.error.issues.map((i) => `${i.path.join('.')}: ${i.message}`).join('; '),
              );
              return;
            }
            await onSubmit(
              '/api/v1/definition-catalogs',
              result.data,
              `Save catalog ${result.data.title}`,
            );
          }}
        />
      ) : !selected ? (
        <div className="panel empty">
          <h3>Create a shared definition catalog</h3>
          <p>
            A catalog can be prepared before any cell is installed. Its access list is independent
            of cell permissions.
          </p>
        </div>
      ) : (
        <div className="definition-layout">
          <aside className="panel definition-library">
            <p className="eyebrow">DEFINITION LIBRARY</p>
            {definitionKinds.map((k) => (
              <button
                type="button"
                className={kind === k ? 'selected' : ''}
                key={k}
                onClick={() => setKind(k)}
              >
                {kindLabel(k)}
                <span>{items.filter((i) => i.kind === k && !i.archived).length}</span>
              </button>
            ))}
            <label>
              Search loaded definitions
              <input value={query} onChange={(e) => setQuery(e.target.value)} />
            </label>
            <label className="definition-toggle">
              <input
                type="checkbox"
                checked={showArchived}
                onChange={(e) => setShowArchived(e.target.checked)}
              />
              Show archived
            </label>
            <button
              type="button"
              className="primary"
              disabled={!editable || locked || loading || kind === 'POINT_PATTERN'}
              onClick={() => {
                if (discard()) {
                  clearEditor();
                  setForm({
                    catalog: selected,
                    id: crypto.randomUUID(),
                    expected: null,
                    label: '',
                    body: newBody(kind),
                    archived: false,
                  });
                }
              }}
            >
              New {kindLabel(kind).toLowerCase()}
            </button>
            <div className="definition-list" aria-busy={loading}>
              {filtered.map((item) => (
                <button
                  type="button"
                  key={item.reference.id}
                  disabled={locked || loading}
                  className={form?.id === item.reference.id ? 'selected' : ''}
                  onClick={() => void openDefinition(item)}
                >
                  <b>{item.label}</b>
                  <small>
                    r{item.reference.revision}
                    {item.archived ? ' · Archived' : ''}
                  </small>
                </button>
              ))}
              {!loading && !filtered.length && <p>No matching loaded definitions.</p>}
            </div>
            {next && (
              <button
                type="button"
                disabled={loading || locked}
                onClick={() => void loadDefinitions(next, generation.current)}
              >
                Load more definitions and reference choices
              </button>
            )}
          </aside>
          <div className="panel definition-detail">
            {!form ? (
              <div className="empty">
                <h3>{kindLabel(kind)}</h3>
                <p>Select a definition, or create one to configure its fields and values.</p>
                <p>Start with properties, then types, models and site instances.</p>
              </div>
            ) : (
              <>
                <div className="definition-row">
                  <div>
                    <p className="eyebrow">{kindLabel(form.body.kind)}</p>
                    <h3>{form.label || 'New definition'}</h3>
                    <small>
                      {historical
                        ? 'Historical revision · read only'
                        : form.expected
                          ? `Saved r${form.expected}`
                          : 'Not saved'}
                      {dirty ? ' · Unsaved changes' : ''}
                    </small>
                  </div>
                  {form.expected && (
                    <button
                      type="button"
                      disabled={loading || locked}
                      onClick={() => void loadHistory()}
                    >
                      Version history
                    </button>
                  )}
                </div>
                {history.length > 0 && (
                  <div className="definition-history">
                    {history.map((v) => (
                      <button
                        key={v.reference.revision}
                        type="button"
                        disabled={locked || loading}
                        onClick={() => void openDefinition(v, true)}
                      >
                        r{v.reference.revision} · {v.label}
                      </button>
                    ))}
                    {historyNext && (
                      <button
                        type="button"
                        disabled={loading || locked}
                        onClick={() => void loadHistory(historyNext)}
                      >
                        Older revisions
                      </button>
                    )}
                  </div>
                )}
                <form
                  onSubmit={async (e) => {
                    e.preventDefault();
                    setError('');
                    const result = definitionSaveSchema.safeParse(form);
                    if (!result.success) {
                      setError(
                        result.error.issues
                          .map((i) => `${i.path.join('.')}: ${i.message}`)
                          .join('; '),
                      );
                      return;
                    }
                    await onSubmit(
                      '/api/v1/definitions',
                      result.data,
                      `Save definition ${result.data.label}`,
                    );
                  }}
                >
                  <fieldset disabled={!editable || locked || historical || loading}>
                    <label className="definition-field">
                      Definition name
                      <input
                        value={form.label}
                        maxLength={120}
                        onChange={(e) => setForm({ ...form, label: e.target.value })}
                      />
                    </label>
                    <DefinitionFields
                      key={`${form.id}/${form.expected ?? 'new'}`}
                      body={form.body}
                      options={options}
                      onChange={(body) => setForm({ ...form, body })}
                    />
                    <label className="definition-toggle">
                      <input
                        type="checkbox"
                        checked={form.archived}
                        onChange={(e) => setForm({ ...form, archived: e.target.checked })}
                      />
                      Archive this definition
                    </label>
                    <div className="definition-actions">
                      <button
                        type="button"
                        onClick={() => {
                          if (discard()) {
                            if (saved) {
                              const input = definitionInput(saved);
                              setForm(input);
                              setOriginal(stableDocument(input));
                            } else clearEditor();
                          }
                        }}
                      >
                        Discard edits
                      </button>
                      <button className="primary" disabled={!canSave || !dirty}>
                        Save new revision
                      </button>
                    </div>
                  </fieldset>
                </form>
                {historical && (
                  <button
                    type="button"
                    disabled={!editable || locked || loading}
                    onClick={() => {
                      setForm({
                        ...form,
                        id: crypto.randomUUID(),
                        expected: null,
                        label: `${form.label} copy`,
                        archived: false,
                      });
                      setOriginal('');
                      setSaved(null);
                      setHistory([]);
                      setHistorical(false);
                    }}
                  >
                    Copy this revision
                  </button>
                )}
                {saved && <EffectiveValues view={saved} dirty={dirty} />}
                {saved &&
                  ['RESOURCE_MODEL', 'RESOURCE_INSTANCE'].includes(
                    saved.version.definition.body.kind,
                  ) && (
                    <DefinitionPoints
                      key={refKey(saved.version.definition.reference)}
                      view={saved}
                      dirty={dirty}
                    />
                  )}
              </>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
function EffectiveValues({ view, dirty }: { view: DefinitionView; dirty: boolean }) {
  const effective = view.effective;
  const d = view.version.definition;
  const names = useDefinitionNames(provenanceRefs(view), { [refKey(d.reference)]: d.label });
  return (
    <section className="definition-effective">
      <h4>Saved field values and sources</h4>
      {dirty && (
        <p>
          These values describe the saved revision. Save your edits to validate and recalculate
          them.
        </p>
      )}
      {view.version.definition.body.kind === 'TASK' && (
        <p>
          Task sources are declarations. Context and runtime input resolution are not evaluated
          here.
        </p>
      )}
      {Object.entries(effective.fields).map(([key, field]) => (
        <article key={key}>
          <b>
            {key}{' '}
            <small>
              {field.specification.unit}
              {field.required ? ' · Required' : ''}
            </small>
          </b>
          <p>{effective.values[key] ? valueText(effective.values[key].value) : 'Not set'}</p>
          <small>
            Field declared by <DefinitionName reference={field.declared_by} names={names} />
          </small>
          {effective.values[key] && (
            <small>
              Value from{' '}
              <DefinitionName reference={effective.values[key].declared_by} names={names} />
            </small>
          )}
          {effective.shadowed[key]?.map((v, i) => (
            <small key={i}>
              Overridden: {valueText(v.value)} ·{' '}
              <DefinitionName reference={v.declared_by} names={names} />
            </small>
          ))}
        </article>
      ))}
      {!!effective.missing.length && (
        <p className="notice">Required values not supplied: {effective.missing.join(', ')}</p>
      )}
    </section>
  );
}
function CatalogEditor({
  input,
  editable,
  locked,
  canSave,
  onChange,
  onCancel,
  onSubmit,
}: {
  input: CatalogSave;
  editable: boolean;
  locked: boolean;
  canSave: boolean;
  onChange: (v: CatalogSave) => void;
  onCancel: () => void;
  onSubmit: () => Promise<void>;
}) {
  const [member, setMember] = useState('');
  const [terminal, setTerminal] = useState('');
  return (
    <form
      className="panel definition-detail"
      onSubmit={(e) => {
        e.preventDefault();
        void onSubmit();
      }}
    >
      <h3>{input.expected ? 'Catalog access' : 'New catalog'}</h3>
      <p>
        The owner manages membership. Members must be existing Engineer or Verifier accounts.
        Registered terminal sessions also need an allowed terminal.
      </p>
      <fieldset disabled={!editable || locked}>
        <label className="definition-field">
          Catalog name
          <input
            value={input.title}
            maxLength={120}
            onChange={(e) => onChange({ ...input, title: e.target.value })}
          />
        </label>
        <h4>Members</h4>
        {Object.entries(input.members).map(([id, access]) => (
          <div className="definition-row" key={id}>
            <b>{id}</b>
            <select
              aria-label={`${id} access`}
              value={access}
              onChange={(e) =>
                onChange({
                  ...input,
                  members: { ...input.members, [id]: e.target.value as 'READ' | 'EDIT' },
                })
              }
            >
              <option>READ</option>
              <option>EDIT</option>
            </select>
            <button
              type="button"
              onClick={() => {
                const members = { ...input.members };
                delete members[id];
                onChange({ ...input, members });
              }}
            >
              Remove
            </button>
          </div>
        ))}
        <div className="definition-row">
          <input
            aria-label="Member account"
            value={member}
            onChange={(e) => setMember(e.target.value)}
            placeholder="Existing account ID"
          />
          <button
            type="button"
            disabled={!member.trim() || member.trim() in input.members}
            onClick={() => {
              onChange({ ...input, members: { ...input.members, [member.trim()]: 'READ' } });
              setMember('');
            }}
          >
            Add member
          </button>
        </div>
        <h4>Allowed terminals</h4>
        {input.terminals.map((id) => (
          <div className="definition-row" key={id}>
            <b>{id}</b>
            <button
              type="button"
              onClick={() =>
                onChange({ ...input, terminals: input.terminals.filter((v) => v !== id) })
              }
            >
              Remove
            </button>
          </div>
        ))}
        <div className="definition-row">
          <input
            aria-label="Terminal ID"
            value={terminal}
            onChange={(e) => setTerminal(e.target.value)}
            placeholder="Registered terminal ID"
          />
          <button
            type="button"
            disabled={!terminal.trim() || input.terminals.includes(terminal.trim())}
            onClick={() => {
              onChange({ ...input, terminals: [...input.terminals, terminal.trim()] });
              setTerminal('');
            }}
          >
            Allow terminal
          </button>
        </div>
        <label className="definition-toggle">
          <input
            type="checkbox"
            checked={input.archived}
            onChange={(e) => onChange({ ...input, archived: e.target.checked })}
          />
          Archive catalog (definitions become read only)
        </label>
        <button className="primary" disabled={!canSave}>
          Save catalog
        </button>
      </fieldset>
      <button type="button" disabled={locked} onClick={onCancel}>
        Close
      </button>
    </form>
  );
}
