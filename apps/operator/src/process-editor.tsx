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
  SEQUENCE: '순차',
  PARALLEL_ALL: '병렬',
  BRANCH: '분기',
  REPEAT: '반복',
  CALL: '공정 호출',
  OPERATION: '작업',
  WAIT: '조건 대기',
  INTERVENTION: '작업자 개입',
};
const issueLabels: Record<string, string> = {
  DOCUMENT_SHAPE: '필수 항목과 값 형식을 확인하세요.',
  SOURCE_SHAPE: '공정 형식 또는 하위 공정 수를 확인하세요.',
  DUPLICATE_FLOW: '공정 식별 이름이 중복되었습니다.',
  ENTRY_MISSING: '진입 공정을 선택하세요.',
  NODE_LIMIT: '노드 수를 확인하세요.',
  DUPLICATE_NODE: '노드 식별 이름이 중복되었습니다.',
  ROOT_MISSING: '시작 노드를 선택하세요.',
  CHILD_MISSING: '연결한 하위 노드가 없습니다.',
  EMPTY_CONTROL: '순차·병렬 노드에 하위 노드를 연결하세요.',
  REPEAT_LIMIT: '반복 횟수는 1–1024의 유한 값이어야 합니다.',
  CALL_MISSING: '호출할 공정이 없습니다.',
  CONDITION_MISSING: '선택한 조건의 정의가 없습니다.',
  WAIT_LIMIT: '대기 기한을 지정하세요.',
  SHARED_OR_CYCLIC_NODE:
    '하위 노드가 중복 연결되었거나 순환합니다. 재사용은 공정 호출로 구성하세요.',
  TREE_DEPTH_OR_CYCLE: '순환 또는 구조 깊이를 확인하세요.',
  UNREACHABLE_NODE: '시작 노드에서 연결되지 않은 노드가 있습니다.',
  CONDITION_SHAPE: '조건식의 구조와 범위를 확인하세요.',
  EXPANSION_LIMIT: '반복·호출을 전개한 공정이 너무 큽니다.',
  CALL_CYCLE: '공정 호출이 순환합니다.',
  UNREACHABLE_FLOW: '진입 공정에서 호출하지 않는 하위 공정이 있습니다.',
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
                  <small>{kinds[str(node.body.kind)] ?? '미지원 노드'}</small>
                  <strong>
                    {node.body.kind === 'OPERATION'
                      ? str(node.body.binding) || '작업 연결 필요'
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
          <h3>공정 초안</h3>
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
              title: '새 공정',
              document: blank(),
              dirty: true,
              validation: null,
              baseline: null,
            });
            setFlow(0);
            setSelected(0);
          }}
        >
          새 초안
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
              {d.structurally_valid ? '구조 확인됨' : `${d.issue_count}개 확인 필요`}
            </small>
          </button>
        ))}
        {next && <button onClick={() => void list(next)}>더 보기</button>}
        {buffer?.dirty && (
          <p className="muted">다른 초안을 열기 전에 저장하거나 변경을 버리세요.</p>
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
            <h2>공정을 초안으로 설계합니다</h2>
            <p>작업과 흐름을 구성하고 저장한 버전의 검사 결과를 확인합니다.</p>
            <small>초안 저장은 현재 셀 구성이나 실행 중인 작업을 변경하지 않습니다.</small>
          </div>
        ) : (
          <>
            <header className="panel draft-header">
              <div>
                <p className="eyebrow">PROCESS DRAFT</p>
                <label>
                  초안 제목
                  <input
                    value={buffer.title}
                    maxLength={120}
                    disabled={disabled}
                    onChange={(e) => onBuffer({ ...buffer, title: e.target.value, dirty: true })}
                  />
                </label>
                <p className="muted">
                  {buffer.expected ? `기준 r${buffer.expected}` : '저장 전'} ·{' '}
                  {pendingEdit ? '저장되지 않은 변경 있음' : '저장된 버전'} · 작성 중인 구성을
                  실행에 적용하지 않습니다.
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
                  초안 저장·구조 확인
                </button>
                <button
                  disabled={!pendingEdit || loading}
                  onClick={() => {
                    onBuffer(buffer.baseline ? fromDetail(buffer.baseline) : null);
                    setComparison(null);
                  }}
                >
                  변경 버리기
                </button>
                <button
                  disabled={disabled || pendingEdit}
                  onClick={() => {
                    onBuffer({
                      ...buffer,
                      id: crypto.randomUUID(),
                      expected: null,
                      title: `${buffer.title} 복사`.slice(0, 120),
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
                  초안 복사
                </button>
                <button onClick={exportJson}>소스 내보내기</button>
                {buffer.expected && <button onClick={() => void compare()}>서버 버전 비교</button>}
              </div>
            </header>
            {comparison && (
              <div className="panel draft-comparison">
                <h3>편집 중인 내용과 서버 기록</h3>
                <p>
                  편집 기준 r{buffer.expected} / 서버 r{comparison.version.revision}
                </p>
                <div className="draft-history-picker">
                  <label>
                    비교할 저장 버전
                    <input
                      inputMode="numeric"
                      value={historyVersion}
                      onChange={(e) => setHistoryVersion(e.target.value)}
                      placeholder="예: 1"
                    />
                  </label>
                  <button
                    disabled={!/^[1-9][0-9]*$/.test(historyVersion)}
                    onClick={() => void compare(historyVersion)}
                  >
                    이 버전 불러오기
                  </button>
                </div>
                <div className="compare-grid">
                  <div>
                    <b>내 편집</b>
                    <p>{buffer.title}</p>
                    <pre>{JSON.stringify(buffer.document, null, 2)}</pre>
                  </div>
                  <div>
                    <b>서버 기록</b>
                    <p>{comparison.version.title}</p>
                    <pre>{JSON.stringify(comparison.document, null, 2)}</pre>
                  </div>
                </div>
                <p className="muted">
                  이 비교는 편집 중인 내용을 덮어쓰지 않습니다. 변경을 버린 뒤 목록에서 다시 열면
                  서버의 최신 버전을 읽습니다.
                </p>
              </div>
            )}
            <section className="panel draft-validation">
              <div className="section-heading">
                <h3>저장 버전의 검사 결과</h3>
                <span
                  className={`badge ${buffer.validation?.structurally_valid && !buffer.dirty ? 'good' : 'warning'}`}
                >
                  {buffer.dirty
                    ? '현재 편집 재검사 필요'
                    : buffer.validation?.structurally_valid
                      ? '구조 확인됨'
                      : '구조 확인 필요'}
                </span>
              </div>
              {buffer.validation ? (
                <>
                  <p className="muted">
                    검사 대상 r{buffer.expected} · 전개 노드 {buffer.validation.expanded_nodes}개 ·
                    장비 바인딩과 패키지·실물 검증은 별도입니다.
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
                  <p>연결할 작업: {buffer.validation.required_bindings.join(', ') || '없음'}</p>
                </>
              ) : (
                <p className="muted">
                  초안을 저장하면 구조를 검사합니다. 미완성 내용도 저장할 수 있습니다.
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
                    진입 공정
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
                    편집 공정
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
                    하위 공정 추가
                  </button>
                  <label>
                    공정 식별 이름
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
                          <h3>공정 흐름</h3>
                          <span className="muted">{flow.nodes.length}개 노드</span>
                        </div>
                        <label>
                          시작 노드
                          <select
                            disabled={disabled}
                            value={flow.root}
                            onChange={(e) =>
                              change((s) => {
                                s.flows[flowIndex].root = e.target.value;
                              })
                            }
                          >
                            <option value="">선택 필요</option>
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
                          <h3>노드 설정</h3>
                        </div>
                        <label>
                          노드 선택
                          <select
                            value={selected}
                            onChange={(e) => setSelected(Number(e.target.value))}
                          >
                            {flow.nodes.map((n, i) => (
                              <option key={i} value={i}>
                                {n.id} · {kinds[str(n.body.kind)] ?? '미지원'}
                              </option>
                            ))}
                          </select>
                        </label>
                        <div className="add-node">
                          <select
                            aria-label="추가할 노드 종류"
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
                            노드 추가
                          </button>
                        </div>
                        {node && (
                          <>
                            <p className="node-kind">
                              {kinds[str(node.body.kind)] ?? '미지원 노드'} · {node.id}
                            </p>
                            {node.body.kind === 'OPERATION' && (
                              <label>
                                작업 연결 이름
                                <input
                                  value={str(node.body.binding)}
                                  disabled={disabled}
                                  onChange={(e) => field('binding', e.target.value)}
                                  placeholder="예: load-material"
                                />
                              </label>
                            )}
                            {(node.body.kind === 'SEQUENCE' ||
                              node.body.kind === 'PARALLEL_ALL') && (
                              <div>
                                <b>하위 노드 순서</b>
                                {strings(node.body.children).map((child, i) => (
                                  <div className="child-order" key={`${child}/${i}`}>
                                    <span>{child}</span>
                                    <button
                                      aria-label={`${child} 위로`}
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
                                      aria-label={`${child} 연결 해제`}
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
                                  aria-label="하위 노드 연결"
                                  disabled={disabled}
                                  value=""
                                  onChange={(e) =>
                                    field('children', [
                                      ...strings(node.body.children),
                                      e.target.value,
                                    ])
                                  }
                                >
                                  <option value="">노드 연결</option>
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
                                조건 이름
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
                                  {key === 'when_true' ? '조건 충족 시' : '조건 미충족 시'}
                                  <select
                                    disabled={disabled}
                                    value={str(node.body[key])}
                                    onChange={(e) => field(key, e.target.value)}
                                  >
                                    <option value="">선택 필요</option>
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
                                  반복 횟수
                                  <input
                                    inputMode="numeric"
                                    value={str(node.body.count)}
                                    disabled={disabled}
                                    onChange={(e) => field('count', e.target.value)}
                                  />
                                </label>
                                <label>
                                  반복할 노드
                                  <select
                                    disabled={disabled}
                                    value={str(node.body.child)}
                                    onChange={(e) => field('child', e.target.value)}
                                  >
                                    <option value="">선택 필요</option>
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
                                대기 기한(ns)
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
                                호출할 공정
                                <select
                                  disabled={disabled}
                                  value={str(node.body.flow)}
                                  onChange={(e) => field('flow', e.target.value)}
                                >
                                  <option value="">선택 필요</option>
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
                                절차 artifact의 연결은 고급 소스 편집에서 지정합니다. 미지정 상태는
                                구조 확인 필요로 저장됩니다.
                              </p>
                            )}
                            <button
                              className="text-button danger"
                              disabled={disabled}
                              onClick={remove}
                            >
                              이 노드 삭제
                            </button>
                          </>
                        )}
                      </section>
                    </div>
                  </>
                )}
                <section className="panel">
                  <h3>조건 정의</h3>
                  <p className="muted">
                    조건식은 장비의 관측 이름과 연결합니다. 현재는 JSON 조건식을 편집하며 장비
                    신호를 자동 승인하지 않습니다.
                  </p>
                  <button
                    disabled={disabled}
                    onClick={() => setConditionRaw(JSON.stringify(source.conditions, null, 2))}
                  >
                    조건 편집 열기
                  </button>
                  {editingConditions && (
                    <>
                      <textarea
                        aria-label="조건 정의 JSON"
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
                            setError('조건 JSON 형식을 확인하세요.');
                          }
                        }}
                      >
                        조건 적용
                      </button>
                      <button disabled={inputDisabled} onClick={() => setConditionRaw(null)}>
                        조건 편집 취소
                      </button>
                    </>
                  )}
                </section>
              </>
            ) : (
              <div className="notice warning">
                구조 편집기로 표시할 수 없는 문서입니다. 고급 소스 편집으로 확인할 수 있으며 저장된
                원문은 유지됩니다.
              </div>
            )}
            <details className="panel advanced-source">
              <summary>고급 소스 편집·가져오기</summary>
              <p className="muted">
                JSON 원문을 변경합니다. 편집 내용은 저장 전까지 활성 공정에 영향을 주지 않습니다.
              </p>
              <button
                disabled={disabled}
                onClick={() => setRaw(JSON.stringify(buffer.document, null, 2))}
              >
                소스 편집 열기
              </button>
              {editingSource && (
                <>
                  <textarea
                    aria-label="공정 소스 JSON"
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
                        setError('JSON 형식을 확인하세요.');
                      }
                    }}
                  >
                    소스 적용
                  </button>
                  <button disabled={inputDisabled} onClick={() => setRaw(null)}>
                    소스 편집 취소
                  </button>
                </>
              )}
              <label>
                JSON 파일 가져오기
                <input
                  type="file"
                  accept="application/json,.json"
                  disabled={disabled}
                  onChange={(e) => {
                    const file = e.target.files?.[0];
                    if (file) {
                      if (file.size > 524288) {
                        setError('가져올 문서는 512 KiB 이하여야 합니다.');
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
                          setError('JSON 파일 형식을 확인하세요.');
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
