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
          검토된 장비 변경안의 후보를 사용하는 공정 초안입니다. 실행 중인 구성에는 아직 적용되지
          않았습니다.
        </p>
      )}
      <div className="section-heading">
        <div>
          <p className="eyebrow">ACTION BINDINGS</p>
          <h3>공정 작업과 장비 작업 연결</h3>
        </div>
        <span
          className={`badge ${!edit && view?.binding?.complete && !view.stale.length ? 'good' : 'warning'}`}
        >
          {edit
            ? '저장되지 않은 선택'
            : !view?.binding
              ? '연결 선택 필요'
              : view.stale.length
                ? '구성 재검토 필요'
                : view.binding.complete
                  ? '모든 연결 선택됨'
                  : '미연결 작업 있음'}
        </span>
      </div>
      <p className="muted">
        현재 셀에 등록된 작업에서 선택합니다. 바인딩 저장과 컴파일 입력은 장비 실행이나 검증 승인을
        만들지 않습니다.
      </p>
      {!sourceReady && (
        <p className="notice">공정 원문을 먼저 저장하고 구조 검사 결과를 확인하세요.</p>
      )}
      {error && (
        <p className="notice error" role="alert">
          {error}
        </p>
      )}
      {view?.stale.length !== 0 && view?.stale.length && (
        <p className="notice warning">
          {view.stale.includes('SOURCE_CHANGED')
            ? '저장한 연결의 공정 내용이 현재 초안과 다릅니다. '
            : ''}
          {view.stale.includes('CATALOG_CHANGED') ? '장비 작업 구성이 변경되었습니다. ' : ''}기존
          선택을 확인하고 현재 구성으로 다시 저장해야 합니다.
        </p>
      )}
      {changedContext && (
        <p className="notice warning">
          편집 기준이 변경됐습니다. 새 목록과 선택을 대조한 뒤 현재 기준으로 검토하세요.
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
                  aria-label={`${binding} 장비 작업`}
                  value={selections[binding] ?? ''}
                  disabled={!canEdit || !sourceReady || !contextCurrent || working}
                  onChange={(e) => choose(binding, e.target.value)}
                >
                  <option value="">작업 선택</option>
                  {catalog?.candidates.map((c) => (
                    <option key={c.step} value={c.step}>
                      {c.step} · {c.host} / {c.target}
                    </option>
                  ))}
                  {selections[binding] && !selected && (
                    <option value={selections[binding]}>이전 선택 · {selections[binding]}</option>
                  )}
                </select>
              </label>
              {selected && (
                <div className="binding-info">
                  <span>{selected.host}</span>
                  <span>대상 {selected.target}</span>
                  <small>
                    자원 {selected.resources.join(', ')} · intent {short(selected.intent_digest)}
                  </small>
                </div>
              )}
            </div>
          );
        })}
      </div>
      {extra.length > 0 && (
        <div className="notice warning">
          현재 공정에 없는 이전 선택: {extra.join(', ')}
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
            이전 선택 제외
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
          현재 기준으로 검토
        </button>
        <button
          className="primary"
          disabled={!canSave || !sourceReady || !edit || changedContext || working}
          onClick={() => edit && void onSave(edit)}
        >
          바인딩 저장
        </button>
        <button
          disabled={!edit || working}
          onClick={() => onBuffer({ ...buffer, bindingEdit: null })}
        >
          선택 변경 버리기
        </button>
        <button
          disabled={
            !sourceReady || !!edit || !view?.binding?.complete || !!view?.stale.length || working
          }
          onClick={() => void exportBundle()}
        >
          컴파일 입력 내보내기
        </button>
        <button disabled={working || !buffer.expected} onClick={() => void load()}>
          장비 작업 다시 조회
        </button>
      </div>
      {view?.binding && (
        <p className="muted">
          바인딩 r{view.binding.revision} · 원문 r{view.binding.source_revision} 기준 · 수정자{' '}
          {view.binding.updated_by}
        </p>
      )}
    </section>
  );
}
