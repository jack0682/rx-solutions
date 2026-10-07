import { useEffect, useRef, useState } from 'react';
import { api, explain } from './api';
import { draftPageSchema, type DraftSummary } from './draft-schema';

type Filters = { query: string; site: string; service: string; state: string };
const empty: Filters = { query: '', site: '', service: '', state: 'active' };
export function DraftLibrary({
  cell,
  selected,
  locked,
  canCreate,
  refresh,
  onCreate,
  onOpen,
}: {
  cell: string;
  selected?: string;
  locked: boolean;
  canCreate: boolean;
  refresh?: string;
  onCreate: () => void;
  onOpen: (id: string) => void;
}) {
  const [open, setOpen] = useState(!selected);
  const [form, setForm] = useState(empty);
  const [filters, setFilters] = useState(empty);
  const [drafts, setDrafts] = useState<DraftSummary[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  async function page(after: string | undefined, version: number, signal?: AbortSignal) {
    setLoading(true);
    setError('');
    const params = new URLSearchParams({ cell });
    if (after) params.set('after', after);
    if (filters.query.trim()) params.set('q', filters.query.trim());
    if (filters.site.trim()) params.set('site', filters.site.trim());
    if (filters.service.trim()) params.set('service', filters.service.trim());
    if (filters.state !== 'all') params.set('archived', String(filters.state === 'archived'));
    try {
      const value = draftPageSchema.parse(
        await api(`/api/v1/process-drafts?${params}`, { signal }),
      );
      if (generation.current !== version) return;
      if (value.cell !== cell) throw new Error('Draft library belongs to a different cell');
      setDrafts((old) => (after ? [...old, ...value.drafts] : value.drafts));
      setNext(value.next);
    } catch (e) {
      if (generation.current === version)
        setError(signal?.aborted ? 'Draft search timed out. Search again.' : explain(e));
    } finally {
      if (generation.current === version) setLoading(false);
    }
  }
  useEffect(() => {
    const version = ++generation.current;
    const controller = new AbortController();
    setDrafts([]);
    setNext(null);
    const timeout = setTimeout(() => controller.abort(), 15000);
    void page(undefined, version, controller.signal).finally(() => clearTimeout(timeout));
    return () => {
      generation.current++;
      clearTimeout(timeout);
      controller.abort();
    };
  }, [cell, filters, refresh]);
  useEffect(() => {
    if (selected) setOpen(false);
  }, [selected]);
  return (
    <details
      className="panel workflow-library-panel"
      open={open}
      onToggle={(e) => setOpen(e.currentTarget.open)}
    >
      <summary>
        Workflow library{' '}
        <span>
          {drafts.length}
          {next ? ' +' : ''} results
        </span>
      </summary>
      <form
        className="workflow-library-filters"
        onSubmit={(e) => {
          e.preventDefault();
          setFilters({ ...form });
        }}
      >
        <label>
          Search drafts
          <input
            value={form.query}
            maxLength={120}
            onChange={(e) => setForm({ ...form, query: e.target.value })}
            placeholder="Workflow title"
          />
        </label>
        <label>
          Filter by site
          <input
            value={form.site}
            maxLength={120}
            onChange={(e) => setForm({ ...form, site: e.target.value })}
            placeholder="Any site"
          />
        </label>
        <label>
          Filter by service
          <input
            value={form.service}
            maxLength={120}
            onChange={(e) => setForm({ ...form, service: e.target.value })}
            placeholder="Any service"
          />
        </label>
        <label>
          Draft state
          <select value={form.state} onChange={(e) => setForm({ ...form, state: e.target.value })}>
            <option value="active">Active drafts</option>
            <option value="archived">Archived</option>
            <option value="all">All drafts</option>
          </select>
        </label>
        <button type="submit">Search</button>
        <button type="button" disabled={!canCreate || locked} onClick={onCreate}>
          New draft
        </button>
      </form>
      {error && <p role="alert">{error}</p>}
      <div className="workflow-library-results" aria-busy={loading}>
        {drafts.map((d) => (
          <button
            key={d.id}
            className={`draft-list-item ${selected === d.id ? 'selected' : ''}`}
            disabled={locked || loading}
            onClick={() => onOpen(d.id)}
          >
            <b>{d.title}</b>
            <small>
              r{d.revision} ·{' '}
              {d.library?.archived
                ? 'Archived'
                : d.structurally_valid
                  ? 'Structure verified'
                  : `${d.issue_count} items to review`}
            </small>
            {(d.library?.site || d.library?.service) && (
              <small>{[d.library.site, d.library.service].filter(Boolean).join(' / ')}</small>
            )}
          </button>
        ))}
        {!loading && !drafts.length && !error && <p>No drafts match these filters.</p>}
        {loading && <p role="status">Loading drafts…</p>}
      </div>
      {next && (
        <button disabled={loading} onClick={() => void page(next, generation.current)}>
          More drafts
        </button>
      )}
    </details>
  );
}
