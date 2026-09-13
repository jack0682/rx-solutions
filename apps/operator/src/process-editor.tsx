import { count } from './labels';
import { DraftBindings } from './draft-bindings';
import type { BindingEdit, BindingVersion } from './draft-bindings-schema';
import { previewRows } from './process-preview';
import { useEffect, useState } from 'react';
import { api, explain } from './api';
import {
  draftPageSchema,
  draftDetailSchema,
  editableSourceSchema,
  fromDetail,
  type DraftBuffer,
  type DraftDetail,
  type DraftSummary,
  type EditableSource,
} from './draft-schema';
const kinds: Record<string, string> = {
  SEQUENCE: 'Sequence',
  PARALLEL_ALL: 'Parallel',
  BRANCH: 'Branch',
  REPEAT: 'Repeat',
  CALL: 'Call workflow',
  OPERATION: 'Operation',
  WAIT: 'Wait for condition',
  INTERVENTION: 'Operator intervention',
};
const issueLabels: Record<string, string> = {
  DOCUMENT_SHAPE: 'Check required fields and value types.',
  SOURCE_SHAPE: 'Check the workflow format and number of subworkflows.',
  DUPLICATE_FLOW: 'Workflow identifiers are duplicated.',
  ENTRY_MISSING: 'Select an entry workflow.',
  NODE_LIMIT: 'Check the number of nodes.',
  DUPLICATE_NODE: 'Node identifiers are duplicated.',
  ROOT_MISSING: 'Select a root node.',
  CHILD_MISSING: 'A referenced child node does not exist.',
  EMPTY_CONTROL: 'Connect child nodes to sequence and parallel nodes.',
  REPEAT_LIMIT: 'The repeat count must be a finite value from 1 to 1024.',
  CALL_MISSING: 'The called workflow does not exist.',
  CONDITION_MISSING: 'The selected condition has no definition.',
  WAIT_LIMIT: 'Specify a wait deadline.',
  SHARED_OR_CYCLIC_NODE:
    'Child nodes are linked more than once or form a cycle. Use workflow calls for reuse.',
  TREE_DEPTH_OR_CYCLE: 'Check for cycles and excessive structural depth.',
  UNREACHABLE_NODE: 'Some nodes are unreachable from the root node.',
  CONDITION_SHAPE: 'Check the structure and scope of the condition expression.',
  EXPANSION_LIMIT: 'The workflow is too large after expanding repeats and calls.',
  CALL_CYCLE: 'Workflow calls form a cycle.',
  UNREACHABLE_FLOW: 'Some subworkflows are not called from the entry workflow.',
};
const str = (v: unknown) => (typeof v === 'string' ? v : '');
const strings = (v: unknown) =>
  Array.isArray(v) ? v.filter((s): s is string => typeof s === 'string') : [];
const blank = () => ({
  schema: 'rx.process-source.v1',
  process: 'process/new',
  entry: 'main',
  conditions: {},
  flows: [
    {
      id: 'main',
      root: 'sequence',
      nodes: [{ id: 'sequence', body: { kind: 'SEQUENCE', children: [] } }],
    },
  ],
});
function Tree({
  source,
  flowIndex,
  onSelect,
}: {
  source: EditableSource;
  flowIndex: number;
  nodeId: string;
  onSelect: (i: number) => void;
}) {
  const flow = source.flows[flowIndex];
  return (
    <div className="graph-tree">
      {previewRows(source, flowIndex).map((row, i) => {
        const node = flow?.nodes[row.nodeIndex];
        return (
          <div
            key={i}
            className={`graph-branch graph-depth-${Math.max(0, Math.min(row.depth, 6))}`}
          >
            {row.problem ? (
              <div className="graph-missing">
                {row.problem} · {row.nodeId}
              </div>
            ) : (
              node && (
                <button
                  className={`graph-node kind-${str(node.body.kind).toLowerCase()}`}
                  onClick={() => onSelect(row.nodeIndex)}
                >
                  <small>{kinds[str(node.body.kind)] ?? 'Unsupported node'}</small>
                  <strong>
                    {node.body.kind === 'OPERATION'
                      ? str(node.body.binding) || 'Operation bindings required'
                      : node.id}
                  </strong>
                </button>
              )
            )}
          </div>
        );
      })}
    </div>
  );
}
export function ProcessEditor({
  cell,
  buffer,
  onBuffer,
  canEdit,
  canSave,
  receipt,
  onSave,
  bindingReceipt,
  onSaveBindings,
}: {
  cell: string;
  buffer: DraftBuffer | null;
  onBuffer: (v: DraftBuffer | null) => void;
  canEdit: boolean;
  canSave: boolean;
  receipt: DraftDetail | null;
  onSave: (buffer: DraftBuffer) => Promise<unknown>;
  bindingReceipt: BindingVersion | null;
  onSaveBindings: (edit: BindingEdit) => Promise<unknown>;
}) {
  const [drafts, setDrafts] = useState<DraftSummary[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [flowIndex, setFlow] = useState(0);
  const [selected, setSelected] = useState(0);
  const [nodeKind, setNodeKind] = useState('OPERATION');
  const [historyVersion, setHistoryVersion] = useState('');
  const raw = buffer?.sourceText ?? '';
  const conditionRaw = buffer?.conditionText ?? '';
  const setRaw = (value: string | null) => {
    if (buffer) onBuffer({ ...buffer, sourceText: value });
  };
  const setConditionRaw = (value: string | null) => {
    if (buffer) onBuffer({ ...buffer, conditionText: value });
  };
  const editingSource = buffer?.sourceText != null;
  const editingConditions = buffer?.conditionText != null;
  const pendingEdit =
    !!buffer && (buffer.dirty || editingSource || editingConditions || buffer.bindingEdit != null);
  const [comparison, setComparison] = useState<DraftDetail | null>(null);
  async function list(after?: string) {
    try {
      const page = draftPageSchema.parse(
        await api(
          `/api/v1/process-drafts?cell=${encodeURIComponent(cell)}${after ? `&after=${after}` : ''}`,
        ),
      );
      setDrafts((old) => (after ? [...old, ...page.drafts] : page.drafts));
      setNext(page.next);
    } catch (e) {
      setError(explain(e));
    }
  }
  useEffect(() => {
    let active = true;
    setError('');
    void api(`/api/v1/process-drafts?cell=${encodeURIComponent(cell)}`)
      .then((v) => {
        if (active) {
          const p = draftPageSchema.parse(v);
          setDrafts(p.drafts);
          setNext(p.next);
        }
      })
      .catch((e) => {
        if (active) setError(explain(e));
      });
    return () => {
      active = false;
    };
  }, [cell]);
  useEffect(() => {
    if (receipt?.version.cell === cell) void list();
  }, [receipt]);
  const parsed = editableSourceSchema.safeParse(buffer?.document);
  const source = parsed.success ? parsed.data : null;
  const flow = source?.flows[flowIndex];
  const node = flow?.nodes[selected];
  function change(fn: (s: EditableSource) => void) {
    if (!buffer || !source || !canEdit) return;
    const copy = structuredClone(source);
    fn(copy);
    onBuffer({ ...buffer, document: copy, dirty: true });
    setComparison(null);
  }
  async function load(id: string) {
    if (pendingEdit) return;
    setLoading(true);
    setError('');
    try {
      const d = draftDetailSchema.parse(
        await api(`/api/v1/process-draft?cell=${encodeURIComponent(cell)}&id=${id}`),
      );
      onBuffer(fromDetail(d));
      setFlow(0);
      setSelected(0);
      setComparison(null);
    } catch (e) {
      setError(explain(e));
    } finally {
      setLoading(false);
    }
  }
  async function compare(revision?: string) {
    if (!buffer?.expected) return;
    setError('');
    try {
      setComparison(
        draftDetailSchema.parse(
          await api(
            `/api/v1/process-draft?cell=${encodeURIComponent(cell)}&id=${buffer.id}${revision ? `&revision=${encodeURIComponent(revision)}` : ''}`,
          ),
        ),
      );
    } catch (e) {
      setError(explain(e));
    }
  }
  function add() {
    change((s) => {
      const f = s.flows[flowIndex];
      if (!f) return;
      let index = 1;
      while (f.nodes.some((n) => n.id === `node-${index}`)) index++;
      const id = `node-${index}`;
      let body: Record<string, unknown> = { kind: nodeKind };
      switch (nodeKind) {
        case 'OPERATION':
          body.binding = '';
          break;
        case 'SEQUENCE':
        case 'PARALLEL_ALL':
          body.children = [];
          break;
        case 'BRANCH':
          body = { ...body, condition: '', when_true: '', when_false: '' };
          break;
        case 'REPEAT':
          body = { ...body, count: '1', child: '' };
          break;
        case 'CALL':
          body.flow = '';
          break;
        case 'WAIT':
          body = { ...body, condition: '', timeout_ns: '1000000000' };
          break;
        case 'INTERVENTION':
          body.procedure = null;
          break;
      }
      f.nodes.push({ id, body });
      if (!f.root) f.root = id;
      else {
        const root = f.nodes.find((n) => n.id === f.root);
        if (root?.body.kind === 'SEQUENCE') {
          root.body.children = [...strings(root.body.children), id];
        }
      }
      setSelected(f.nodes.length - 1);
    });
  }
  function field(key: string, value: unknown) {
    change((s) => {
      s.flows[flowIndex].nodes[selected].body[key] = value;
    });
  }
  function remove() {
    if (!node) return;
    change((s) => {
      const f = s.flows[flowIndex];
      const old = f.nodes[selected].id;
      f.nodes.splice(selected, 1);
      if (f.root === old) f.root = '';
      for (const n of f.nodes) {
        if (Array.isArray(n.body.children))
          n.body.children = strings(n.body.children).filter((id) => id !== old);
        for (const k of ['child', 'when_true', 'when_false']) if (n.body[k] === old) n.body[k] = '';
      }
      setSelected(0);
    });
  }
  function exportJson() {
    if (!buffer) return;
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(buffer.document, null, 2) + '\n'], { type: 'application/json' }),
    );
    const link = document.createElement('a');
    link.href = url;
    link.download = 'process-source.json';
    link.click();
    URL.revokeObjectURL(url);
  }
  const inputDisabled = !canEdit || loading;
  const disabled =
    inputDisabled || editingSource || editingConditions || buffer?.bindingEdit != null;
  return (
    <div className="draft-workspace">
      <aside className="panel draft-list">
        <div className="section-heading">
          <h3>Workflow drafts</h3>
          <span className="count">{drafts.length}</span>
        </div>
        <button
          className="primary wide"
          disabled={disabled || pendingEdit}
          onClick={() => {
            onBuffer({
              id: crypto.randomUUID(),
              cell,
              expected: null,
              title: 'New workflow',
              document: blank(),
              dirty: true,
              validation: null,
              baseline: null,
            });
            setFlow(0);
            setSelected(0);
          }}
        >
          New draft
        </button>
        {drafts.map((d) => (
          <button
            className={`draft-list-item ${buffer?.id === d.id ? 'selected' : ''}`}
            key={d.id}
            disabled={loading || pendingEdit}
            onClick={() => void load(d.id)}
          >
            <b>{d.title}</b>
            <small>
              r{d.revision} ·{' '}
              {d.structurally_valid
                ? 'Structure verified'
                : `${count(d.issue_count, 'item')} to review`}
            </small>
          </button>
        ))}
        {next && <button onClick={() => void list(next)}>Load more</button>}
        {buffer?.dirty && (
          <p className="muted">Save or discard your changes before opening another draft.</p>
        )}
      </aside>
      <section className="draft-main">
        {error && (
          <div className="notice error" role="alert">
            {error}
          </div>
        )}
        {!buffer ? (
          <div className="panel empty">
            <h2>Design a workflow draft</h2>
            <p>
              Arrange operations and control flow, then inspect validation results for the saved
              version.
            </p>
            <small>
              Saving a draft does not change the current cell configuration or running operations.
            </small>
          </div>
        ) : (
          <>
            <header className="panel draft-header">
              <div>
                <p className="eyebrow">PROCESS DRAFT</p>
                <label>
                  Draft title
                  <input
                    value={buffer.title}
                    maxLength={120}
                    disabled={disabled}
                    onChange={(e) => onBuffer({ ...buffer, title: e.target.value, dirty: true })}
                  />
                </label>
                <p className="muted">
                  {buffer.expected ? `Baseline r${buffer.expected}` : 'Not saved yet'} ·{' '}
                  {pendingEdit ? 'Unsaved changes' : 'Saved version'} · The configuration being
                  edited is not applied to execution.
                </p>
              </div>
              <div className="draft-actions">
                <button
                  className="primary"
                  disabled={
                    !canSave || !buffer.dirty || loading || editingSource || editingConditions
                  }
                  onClick={() => void onSave(buffer)}
                >
                  Save draft and validate structure
                </button>
                <button
                  disabled={!pendingEdit || loading}
                  onClick={() => {
                    onBuffer(buffer.baseline ? fromDetail(buffer.baseline) : null);
                    setComparison(null);
                  }}
                >
                  Discard changes
                </button>
                <button
                  disabled={disabled || pendingEdit}
                  onClick={() => {
                    onBuffer({
                      ...buffer,
                      id: crypto.randomUUID(),
                      expected: null,
                      title: `${buffer.title} Copy`.slice(0, 120),
                      document: structuredClone(buffer.document),
                      dirty: true,
                      validation: null,
                      baseline: null,
                      sourceText: null,
                      conditionText: null,
                      bindingEdit: null,
                    });
                    setComparison(null);
                  }}
                >
                  Copy draft
                </button>
                <button onClick={exportJson}>Export source</button>
                {buffer.expected && (
                  <button onClick={() => void compare()}>Compare server version</button>
                )}
              </div>
            </header>
            {comparison && (
              <div className="panel draft-comparison">
                <h3>Current edits and server record</h3>
                <p>
                  Editing baseline r{buffer.expected} / server r{comparison.version.revision}
                </p>
                <div className="draft-history-picker">
                  <label>
                    Saved version to compare
                    <input
                      inputMode="numeric"
                      value={historyVersion}
                      onChange={(e) => setHistoryVersion(e.target.value)}
                      placeholder="Example: 1"
                    />
                  </label>
                  <button
                    disabled={!/^[1-9][0-9]*$/.test(historyVersion)}
                    onClick={() => void compare(historyVersion)}
                  >
                    Load this version
                  </button>
                </div>
                <div className="compare-grid">
                  <div>
                    <b>My edits</b>
                    <p>{buffer.title}</p>
                    <pre>{JSON.stringify(buffer.document, null, 2)}</pre>
                  </div>
                  <div>
                    <b>Server record</b>
                    <p>{comparison.version.title}</p>
                    <pre>{JSON.stringify(comparison.document, null, 2)}</pre>
                  </div>
                </div>
                <p className="muted">
                  This comparison does not overwrite your edits. Discard changes and reopen the
                  draft from the list to load the latest server version.
                </p>
              </div>
            )}
            <section className="panel draft-validation">
              <div className="section-heading">
                <h3>Saved version validation results</h3>
                <span
                  className={`badge ${buffer.validation?.structurally_valid && !buffer.dirty ? 'good' : 'warning'}`}
                >
                  {buffer.dirty
                    ? 'Current edits need revalidation'
                    : buffer.validation?.structurally_valid
                      ? 'Structure verified'
                      : 'Structure needs verification'}
                </span>
              </div>
              {buffer.validation ? (
                <>
                  <p className="muted">
                    Validated revision r{buffer.expected} · expanded nodes{' '}
                    {count(buffer.validation.expanded_nodes, 'item')} · device bindings, package
                    verification, and physical verification are separate.
                  </p>
                  <ul className="draft-issues">
                    {buffer.validation.issues.map((issue, i) => (
                      <li key={i}>
                        <b>{issue.location}</b>
                        <span title={`${issue.code}: ${issue.message}`}>
                          {issueLabels[issue.code] ?? issue.message}
                        </span>
                      </li>
                    ))}
                  </ul>
                  <p>
                    Operations to bind: {buffer.validation.required_bindings.join(', ') || 'None'}
                  </p>
                </>
              ) : (
                <p className="muted">
                  Saving the draft validates its structure. Incomplete content can also be saved.
                </p>
              )}
            </section>
            <DraftBindings
              buffer={buffer}
              onBuffer={onBuffer}
              canEdit={canEdit && !loading}
              canSave={canSave}
              receipt={bindingReceipt}
              onSave={onSaveBindings}
            />
            {source ? (
              <>
                <div className="panel flow-toolbar">
                  <label>
                    Entry workflow
                    <select
                      disabled={disabled}
                      value={source.entry}
                      onChange={(e) =>
                        change((s) => {
                          s.entry = e.target.value;
                        })
                      }
                    >
                      {source.flows.map((f, i) => (
                        <option key={i} value={f.id}>
                          {f.id}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Workflow to edit
                    <select
                      value={flowIndex}
                      onChange={(e) => {
                        setFlow(Number(e.target.value));
                        setSelected(0);
                      }}
                    >
                      {source.flows.map((f, i) => (
                        <option key={i} value={i}>
                          {f.id}
                        </option>
                      ))}
                    </select>
                  </label>
                  <button
                    disabled={disabled}
                    onClick={() =>
                      change((s) => {
                        let i = 1;
                        while (s.flows.some((f) => f.id === `flow-${i}`)) i++;
                        s.flows.push({ id: `flow-${i}`, root: '', nodes: [] });
                        setFlow(s.flows.length - 1);
                        setSelected(0);
                      })
                    }
                  >
                    Add subworkflow
                  </button>
                  <label>
                    Workflow identifier
                    <input
                      value={source.process}
                      disabled={disabled}
                      onChange={(e) =>
                        change((s) => {
                          s.process = e.target.value;
                        })
                      }
                    />
                  </label>
                </div>
                {flow && (
                  <>
                    <div className="editor-grid">
                      <section className="panel graph-canvas">
                        <div className="section-heading">
                          <h3>Workflow flow</h3>
                          <span className="muted">{count(flow.nodes.length, 'node')}</span>
                        </div>
                        <label>
                          Root node
                          <select
                            disabled={disabled}
                            value={flow.root}
                            onChange={(e) =>
                              change((s) => {
                                s.flows[flowIndex].root = e.target.value;
                              })
                            }
                          >
                            <option value="">Selection required</option>
                            {flow.nodes.map((n, i) => (
                              <option key={i} value={n.id}>
                                {n.id}
                              </option>
                            ))}
                          </select>
                        </label>
                        <div className="graph-scroll">
                          <Tree
                            source={source}
                            flowIndex={flowIndex}
                            nodeId={flow.root}
                            onSelect={setSelected}
                          />
                        </div>
                      </section>
                      <section className="panel node-inspector">
                        <div className="section-heading">
                          <h3>Node settings</h3>
                        </div>
                        <label>
                          Select node
                          <select
                            value={selected}
                            onChange={(e) => setSelected(Number(e.target.value))}
                          >
                            {flow.nodes.map((n, i) => (
                              <option key={i} value={i}>
                                {n.id} · {kinds[str(n.body.kind)] ?? 'Unsupported'}
                              </option>
                            ))}
                          </select>
                        </label>
                        <div className="add-node">
                          <select
                            aria-label="Node kind to add"
                            disabled={disabled}
                            value={nodeKind}
                            onChange={(e) => setNodeKind(e.target.value)}
                          >
                            {Object.entries(kinds).map(([k, v]) => (
                              <option key={k} value={k}>
                                {v}
                              </option>
                            ))}
                          </select>
                          <button disabled={disabled} onClick={add}>
                            Add node
                          </button>
                        </div>
                        {node && (
                          <>
                            <p className="node-kind">
                              {kinds[str(node.body.kind)] ?? 'Unsupported node'} · {node.id}
                            </p>
                            {node.body.kind === 'OPERATION' && (
                              <label>
                                Operation binding name
                                <input
                                  value={str(node.body.binding)}
                                  disabled={disabled}
                                  onChange={(e) => field('binding', e.target.value)}
                                  placeholder="Example: load-material"
                                />
                              </label>
                            )}
                            {(node.body.kind === 'SEQUENCE' ||
                              node.body.kind === 'PARALLEL_ALL') && (
                              <div>
                                <b>Child node order</b>
                                {strings(node.body.children).map((child, i) => (
                                  <div className="child-order" key={`${child}/${i}`}>
                                    <span>{child}</span>
                                    <button
                                      aria-label={`${child} Move up`}
                                      disabled={disabled || i === 0}
                                      onClick={() => {
                                        const a = strings(node.body.children);
                                        [a[i - 1], a[i]] = [a[i], a[i - 1]];
                                        field('children', a);
                                      }}
                                    >
                                      ↑
                                    </button>
                                    <button
                                      aria-label={`${child} Unlink`}
                                      disabled={disabled}
                                      onClick={() =>
                                        field(
                                          'children',
                                          strings(node.body.children).filter(
                                            (_, index) => index !== i,
                                          ),
                                        )
                                      }
                                    >
                                      −
                                    </button>
                                  </div>
                                ))}
                                <select
                                  aria-label="Connect child node"
                                  disabled={disabled}
                                  value=""
                                  onChange={(e) =>
                                    field('children', [
                                      ...strings(node.body.children),
                                      e.target.value,
                                    ])
                                  }
                                >
                                  <option value="">Connect node</option>
                                  {flow.nodes
                                    .filter((n) => n.id !== node.id)
                                    .map((n, i) => (
                                      <option key={i} value={n.id}>
                                        {n.id}
                                      </option>
                                    ))}
                                </select>
                              </div>
                            )}
                            {['BRANCH', 'WAIT'].includes(str(node.body.kind)) && (
                              <label>
                                Condition name
                                <input
                                  list="draft-conditions"
                                  value={str(node.body.condition)}
                                  disabled={disabled}
                                  onChange={(e) => field('condition', e.target.value)}
                                />
                                <datalist id="draft-conditions">
                                  {Object.keys(source.conditions).map((c) => (
                                    <option key={c}>{c}</option>
                                  ))}
                                </datalist>
                              </label>
                            )}
                            {node.body.kind === 'BRANCH' &&
                              (['when_true', 'when_false'] as const).map((key) => (
                                <label key={key}>
                                  {key === 'when_true'
                                    ? 'When condition is met'
                                    : 'When condition is not met'}
                                  <select
                                    disabled={disabled}
                                    value={str(node.body[key])}
                                    onChange={(e) => field(key, e.target.value)}
                                  >
                                    <option value="">Selection required</option>
                                    {flow.nodes
                                      .filter((n) => n.id !== node.id)
                                      .map((n, i) => (
                                        <option key={i} value={n.id}>
                                          {n.id}
                                        </option>
                                      ))}
                                  </select>
                                </label>
                              ))}
                            {node.body.kind === 'REPEAT' && (
                              <>
                                <label>
                                  Repeat count
                                  <input
                                    inputMode="numeric"
                                    value={str(node.body.count)}
                                    disabled={disabled}
                                    onChange={(e) => field('count', e.target.value)}
                                  />
                                </label>
                                <label>
                                  Node to repeat
                                  <select
                                    disabled={disabled}
                                    value={str(node.body.child)}
                                    onChange={(e) => field('child', e.target.value)}
                                  >
                                    <option value="">Selection required</option>
                                    {flow.nodes
                                      .filter((n) => n.id !== node.id)
                                      .map((n, i) => (
                                        <option key={i} value={n.id}>
                                          {n.id}
                                        </option>
                                      ))}
                                  </select>
                                </label>
                              </>
                            )}
                            {node.body.kind === 'WAIT' && (
                              <label>
                                Wait deadline (ns)
                                <input
                                  inputMode="numeric"
                                  value={str(node.body.timeout_ns)}
                                  disabled={disabled}
                                  onChange={(e) => field('timeout_ns', e.target.value)}
                                />
                              </label>
                            )}
                            {node.body.kind === 'CALL' && (
                              <label>
                                Workflow to call
                                <select
                                  disabled={disabled}
                                  value={str(node.body.flow)}
                                  onChange={(e) => field('flow', e.target.value)}
                                >
                                  <option value="">Selection required</option>
                                  {source.flows.map((f, i) => (
                                    <option key={i} value={f.id}>
                                      {f.id}
                                    </option>
                                  ))}
                                </select>
                              </label>
                            )}
                            {node.body.kind === 'INTERVENTION' && (
                              <p className="muted">
                                Specify procedure artifact bindings in advanced source editing.
                                Unspecified bindings are saved as requiring structural verification.
                              </p>
                            )}
                            <button
                              className="text-button danger"
                              disabled={disabled}
                              onClick={remove}
                            >
                              Delete this node
                            </button>
                          </>
                        )}
                      </section>
                    </div>
                  </>
                )}
                <section className="panel">
                  <h3>Condition definitions</h3>
                  <p className="muted">
                    Condition expressions reference device observation names. Edit the expressions
                    as JSON; device signals are not approved automatically.
                  </p>
                  <button
                    disabled={disabled}
                    onClick={() => setConditionRaw(JSON.stringify(source.conditions, null, 2))}
                  >
                    Open condition editor
                  </button>
                  {editingConditions && (
                    <>
                      <textarea
                        aria-label="Condition definitions JSON"
                        className="source-code"
                        value={conditionRaw}
                        onChange={(e) => setConditionRaw(e.target.value)}
                        disabled={inputDisabled}
                      />
                      <button
                        disabled={inputDisabled}
                        onClick={() => {
                          try {
                            const value: unknown = JSON.parse(conditionRaw);
                            if (!value || Array.isArray(value) || typeof value !== 'object')
                              throw new Error();
                            const next = structuredClone(source);
                            next.conditions = value as Record<string, unknown>;
                            onBuffer({
                              ...buffer,
                              document: next,
                              dirty: true,
                              conditionText: null,
                            });
                          } catch {
                            setError('Check the condition JSON format.');
                          }
                        }}
                      >
                        Apply conditions
                      </button>
                      <button disabled={inputDisabled} onClick={() => setConditionRaw(null)}>
                        Cancel condition editing
                      </button>
                    </>
                  )}
                </section>
              </>
            ) : (
              <div className="notice warning">
                This document cannot be displayed in the structural editor. Use advanced source
                editing to inspect it. The saved source is preserved.
              </div>
            )}
            <details className="panel advanced-source">
              <summary>Advanced source editing and import</summary>
              <p className="muted">
                Edit the JSON source. Edits do not affect the active workflow before saving.
              </p>
              <button
                disabled={disabled}
                onClick={() => setRaw(JSON.stringify(buffer.document, null, 2))}
              >
                Open source editor
              </button>
              {editingSource && (
                <>
                  <textarea
                    aria-label="Workflow source JSON"
                    className="source-code"
                    value={raw}
                    onChange={(e) => setRaw(e.target.value)}
                    disabled={inputDisabled}
                  />
                  <button
                    disabled={inputDisabled}
                    onClick={() => {
                      try {
                        onBuffer({
                          ...buffer,
                          document: JSON.parse(raw) as unknown,
                          dirty: true,
                          sourceText: null,
                        });
                        setFlow(0);
                        setSelected(0);
                      } catch {
                        setError('Check the JSON format.');
                      }
                    }}
                  >
                    Apply source
                  </button>
                  <button disabled={inputDisabled} onClick={() => setRaw(null)}>
                    Cancel source editing
                  </button>
                </>
              )}
              <label>
                Import JSON file
                <input
                  type="file"
                  accept="application/json,.json"
                  disabled={disabled}
                  onChange={(e) => {
                    const file = e.target.files?.[0];
                    if (file) {
                      if (file.size > 524288) {
                        setError('The imported document must not exceed 512 KiB.');
                        return;
                      }
                      void file.text().then((text) => {
                        try {
                          onBuffer({
                            ...buffer,
                            document: JSON.parse(text) as unknown,
                            dirty: true,
                          });
                          setFlow(0);
                          setSelected(0);
                        } catch {
                          setError('Check the JSON file format.');
                        }
                      });
                    }
                  }}
                />
              </label>
            </details>
          </>
        )}
      </section>
    </div>
  );
}
