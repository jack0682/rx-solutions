import { useEffect, useState, type ReactNode } from 'react';
import { api, explain } from './api';
import {
  definitionViewSchema,
  refKey,
  valueText,
  type DefinitionBody,
  type DefinitionRef,
  type DefinitionSummary,
  type DefinitionValue,
  type DefinitionView,
  type PropertySpec,
} from './definition-schema';

export const kindLabel = (kind: string) =>
  kind
    .toLowerCase()
    .replaceAll('_', ' ')
    .replace(/^./, (c) => c.toUpperCase());
const emptyRef = (): DefinitionRef => ({ catalog: '', id: '', revision: '1', digest: '' });
export const defaultProperty = (): PropertySpec => ({
  value_type: 'NUMBER',
  category: 'RESOURCE',
  constraint_scope: null,
  unit: 'mm',
  minimum: null,
  maximum: null,
  choices: [],
  vector_length: null,
  overridable: false,
  parameter_mapping: {},
});
export function newBody(kind: DefinitionBody['kind']): DefinitionBody {
  switch (kind) {
    case 'POINT_PATTERN':
      return { kind, resource_type: emptyRef(), origin: '', frame: '', axes: [] };
    case 'PROPERTY':
      return { kind, specification: defaultProperty() };
    case 'OBJECT_TYPE':
    case 'RESOURCE_TYPE':
      return { kind, parent: null, fields: {} };
    case 'OBJECT_MODEL':
      return { kind, object_type: emptyRef(), values: {} };
    case 'RESOURCE_MODEL':
      return { kind, resource_type: emptyRef(), values: {} };
    case 'RESOURCE_INSTANCE':
      return { kind, base: emptyRef(), values: {} };
    case 'PROPERTY_SET':
      return { kind, values: {} };
    case 'TASK':
      return {
        kind,
        metadata: {
          display_name: '',
          category: 'Manipulation',
          services: { kind: 'ALL' },
          completion_description: '',
          default_timeout_ns: null,
        },
        slots: {},
        properties: {},
      };
  }
}
function Field({ title, children }: { title: string; children: ReactNode }) {
  return (
    <label className="definition-field">
      <span>{title}</span>
      {children}
    </label>
  );
}
function Toggle({
  title,
  value,
  onChange,
}: {
  title: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="definition-toggle">
      <input type="checkbox" checked={value} onChange={(e) => onChange(e.target.checked)} />
      {title}
    </label>
  );
}
export function ReferenceSelect({
  title,
  value,
  kinds,
  options,
  onChange,
  optional = false,
}: {
  title: string;
  value: DefinitionRef | null;
  kinds: DefinitionBody['kind'][];
  options: DefinitionSummary[];
  onChange: (v: DefinitionRef | null) => void;
  optional?: boolean;
}) {
  const choices = options.filter((v) => kinds.includes(v.kind) && !v.archived);
  const key = value?.id ? refKey(value) : '';
  return (
    <Field title={title}>
      <select
        value={key}
        onChange={(e) =>
          onChange(choices.find((v) => refKey(v.reference) === e.target.value)?.reference ?? null)
        }
      >
        <option value="">{optional ? 'None' : 'Select a saved definition'}</option>
        {key && !choices.some((v) => refKey(v.reference) === key) && (
          <option value={key}>
            Pinned r{value?.revision} · {value?.id}
          </option>
        )}
        {choices.map((v) => (
          <option key={refKey(v.reference)} value={refKey(v.reference)}>
            {v.label} · r{v.reference.revision}
          </option>
        ))}
      </select>
    </Field>
  );
}
function useReference(value: DefinitionRef | null) {
  const key = value?.id ? refKey(value) : '';
  const [result, setResult] = useState<{ key: string; view?: DefinitionView; error?: string }>({
    key: '',
  });
  useEffect(() => {
    if (!key || !value) return;
    const controller = new AbortController();
    const params = new URLSearchParams({
      catalog: value.catalog,
      id: value.id,
      revision: value.revision,
    });
    void api(`/api/v1/definition?${params}`, {
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(15000)]),
    })
      .then((raw) => {
        const view = definitionViewSchema.parse(raw);
        if (refKey(view.version.definition.reference) !== key) throw new Error('Reference differs');
        if (!controller.signal.aborted) setResult({ key, view });
      })
      .catch((e) => {
        if (!controller.signal.aborted) setResult({ key, error: explain(e) });
      });
    return () => controller.abort();
  }, [key]);
  return result.key === key ? result : { key };
}
function initialValue(spec: PropertySpec): DefinitionValue {
  switch (spec.value_type) {
    case 'NUMBER':
      return { number: spec.minimum ?? 0 };
    case 'BOOLEAN':
      return { boolean: false };
    case 'TEXT':
      return { text: spec.choices[0] ?? '' };
    case 'VECTOR':
      return { vector: Array.from({ length: spec.vector_length ?? 1 }, () => spec.minimum ?? 0) };
  }
}
export function ValueEditor({
  title,
  spec,
  value,
  onChange,
}: {
  title: string;
  spec: PropertySpec;
  value: DefinitionValue;
  onChange: (v: DefinitionValue) => void;
}) {
  if ('boolean' in value)
    return (
      <Toggle title={title} value={value.boolean} onChange={(boolean) => onChange({ boolean })} />
    );
  if ('text' in value)
    return (
      <Field title={title}>
        {spec.choices.length ? (
          <select value={value.text} onChange={(e) => onChange({ text: e.target.value })}>
            {spec.choices.map((v) => (
              <option key={v}>{v}</option>
            ))}
          </select>
        ) : (
          <input
            value={value.text}
            maxLength={2048}
            onChange={(e) => onChange({ text: e.target.value })}
          />
        )}
      </Field>
    );
  if ('vector' in value)
    return (
      <fieldset className="definition-vector">
        <legend>
          {title} · {spec.unit}
        </legend>
        {value.vector.map((n, i) => (
          <label key={i}>
            Component {i + 1}
            <input
              type="number"
              step="any"
              value={n}
              min={spec.minimum ?? undefined}
              max={spec.maximum ?? undefined}
              onChange={(e) => {
                if (e.target.value && Number.isFinite(e.target.valueAsNumber))
                  onChange({
                    vector: value.vector.map((old, at) =>
                      at === i ? e.target.valueAsNumber : old,
                    ),
                  });
              }}
            />
          </label>
        ))}
      </fieldset>
    );
  return (
    <Field title={`${title} · ${spec.unit}`}>
      <input
        type="number"
        step="any"
        value={value.number}
        min={spec.minimum ?? undefined}
        max={spec.maximum ?? undefined}
        onChange={(e) => {
          if (e.target.value && Number.isFinite(e.target.valueAsNumber))
            onChange({ number: e.target.valueAsNumber });
        }}
      />
    </Field>
  );
}
function PropertyEditor({
  spec,
  onChange,
}: {
  spec: PropertySpec;
  onChange: (p: PropertySpec) => void;
}) {
  const [mapping, setMapping] = useState({ skill: '', parameter: '' });
  return (
    <>
      <div className="definition-grid">
        <Field title="Value type">
          <select
            value={spec.value_type}
            onChange={(e) => {
              const type = e.target.value as PropertySpec['value_type'];
              onChange({
                ...spec,
                value_type: type,
                unit: type === 'BOOLEAN' || type === 'TEXT' ? 'unitless' : spec.unit,
                minimum: null,
                maximum: null,
                choices: [],
                vector_length: type === 'VECTOR' ? 3 : null,
              });
            }}
          >
            {['NUMBER', 'BOOLEAN', 'TEXT', 'VECTOR'].map((v) => (
              <option key={v}>{v}</option>
            ))}
          </select>
        </Field>
        <Field title="Property category">
          <select
            value={spec.category}
            onChange={(e) => {
              const category = e.target.value as PropertySpec['category'];
              onChange({
                ...spec,
                category,
                constraint_scope: category === 'CONSTRAINT' ? 'OBJECT' : null,
                overridable: category === 'CONSTRAINT' ? false : spec.overridable,
              });
            }}
          >
            {['OBJECT', 'RESOURCE', 'EXECUTION', 'CONSTRAINT'].map((v) => (
              <option key={v}>{v}</option>
            ))}
          </select>
        </Field>
        {spec.category === 'CONSTRAINT' && (
          <Field title="Constraint scope">
            <select
              value={spec.constraint_scope ?? 'OBJECT'}
              onChange={(e) =>
                onChange({
                  ...spec,
                  constraint_scope: e.target.value as PropertySpec['constraint_scope'],
                })
              }
            >
              {['OBJECT', 'RESOURCE', 'SYSTEM'].map((v) => (
                <option key={v}>{v}</option>
              ))}
            </select>
          </Field>
        )}
        {(spec.value_type === 'NUMBER' || spec.value_type === 'VECTOR') && (
          <>
            <Field title="Unit">
              <input
                value={spec.unit}
                onChange={(e) => onChange({ ...spec, unit: e.target.value })}
              />
            </Field>
            {(['minimum', 'maximum'] as const).map((key) => (
              <Field key={key} title={kindLabel(key)}>
                <input
                  type="number"
                  step="any"
                  value={spec[key] ?? ''}
                  onChange={(e) =>
                    onChange({
                      ...spec,
                      [key]: e.target.value === '' ? null : e.target.valueAsNumber,
                    })
                  }
                />
              </Field>
            ))}
          </>
        )}
        {spec.value_type === 'VECTOR' && (
          <Field title="Vector length">
            <input
              type="number"
              min={1}
              max={128}
              value={spec.vector_length ?? 3}
              onChange={(e) => onChange({ ...spec, vector_length: e.target.valueAsNumber })}
            />
          </Field>
        )}
        {spec.value_type === 'TEXT' && (
          <Field title="Allowed choices (one per line; empty allows any text)">
            <textarea
              value={spec.choices.join('\n')}
              onChange={(e) =>
                onChange({ ...spec, choices: e.target.value ? e.target.value.split('\n') : [] })
              }
            />
          </Field>
        )}
      </div>
      {spec.category !== 'CONSTRAINT' && (
        <Toggle
          title="Allow task overrides and Property Sets"
          value={spec.overridable}
          onChange={(overridable) => onChange({ ...spec, overridable })}
        />
      )}
      <h4>Skill parameter names</h4>
      <p className="muted">
        Declare logical names here. Runtime implementation matching is checked separately.
      </p>
      {Object.entries(spec.parameter_mapping).map(([skill, parameter]) => (
        <div className="definition-row" key={skill}>
          <b>{skill}</b>
          <input
            aria-label={`${skill} parameter`}
            value={parameter}
            onChange={(e) =>
              onChange({
                ...spec,
                parameter_mapping: { ...spec.parameter_mapping, [skill]: e.target.value },
              })
            }
          />
          <button
            type="button"
            onClick={() => {
              const map = { ...spec.parameter_mapping };
              delete map[skill];
              onChange({ ...spec, parameter_mapping: map });
            }}
          >
            Remove
          </button>
        </div>
      ))}
      <div className="definition-row">
        <input
          aria-label="Skill name"
          placeholder="Skill name"
          value={mapping.skill}
          onChange={(e) => setMapping({ ...mapping, skill: e.target.value })}
        />
        <input
          aria-label="Parameter name"
          placeholder="Parameter name"
          value={mapping.parameter}
          onChange={(e) => setMapping({ ...mapping, parameter: e.target.value })}
        />
        <button
          type="button"
          disabled={!mapping.skill || !mapping.parameter || mapping.skill in spec.parameter_mapping}
          onClick={() => {
            onChange({
              ...spec,
              parameter_mapping: { ...spec.parameter_mapping, [mapping.skill]: mapping.parameter },
            });
            setMapping({ skill: '', parameter: '' });
          }}
        >
          Add mapping
        </button>
      </div>
    </>
  );
}
function AddProperty({
  options,
  used,
  onAdd,
}: {
  options: DefinitionSummary[];
  used: string[];
  onAdd: (key: string, ref: DefinitionRef) => void;
}) {
  const [key, setKey] = useState('');
  const [ref, setRef] = useState<DefinitionRef | null>(null);
  return (
    <div className="definition-row">
      <Field title="Field key">
        <input
          value={key}
          maxLength={128}
          onChange={(e) => setKey(e.target.value)}
          placeholder="e.g. surface_height"
        />
      </Field>
      <ReferenceSelect
        title="Property definition"
        value={ref}
        kinds={['PROPERTY']}
        options={options}
        onChange={setRef}
      />
      <button
        type="button"
        disabled={!key.trim() || used.includes(key.trim()) || !ref}
        onClick={() => {
          if (ref) {
            onAdd(key.trim(), ref);
            setKey('');
            setRef(null);
          }
        }}
      >
        Add property
      </button>
    </div>
  );
}
function AssignedValue({
  title,
  reference,
  value,
  optional,
  onChange,
}: {
  title: string;
  reference: DefinitionRef;
  value: DefinitionValue | null;
  optional: boolean;
  onChange: (v: DefinitionValue | null) => void;
}) {
  const { view, error } = useReference(reference);
  const body = view?.version.definition.body;
  if (error)
    return (
      <p role="alert">
        {title}: {error}
      </p>
    );
  if (!body || body.kind !== 'PROPERTY') return <p>{title} · Loading property…</p>;
  return (
    <div className="definition-value">
      {value ? (
        <>
          <ValueEditor title={title} spec={body.specification} value={value} onChange={onChange} />
          {optional && (
            <button type="button" onClick={() => onChange(null)}>
              Remove value
            </button>
          )}
        </>
      ) : (
        <button type="button" onClick={() => onChange(initialValue(body.specification))}>
          Set {title}
        </button>
      )}
    </div>
  );
}
function ModelEditor({
  body,
  options,
  onChange,
}: {
  body: Extract<DefinitionBody, { kind: 'OBJECT_MODEL' | 'RESOURCE_MODEL' | 'RESOURCE_INSTANCE' }>;
  options: DefinitionSummary[];
  onChange: (b: DefinitionBody) => void;
}) {
  const ref =
    body.kind === 'OBJECT_MODEL'
      ? body.object_type
      : body.kind === 'RESOURCE_MODEL'
        ? body.resource_type
        : body.base;
  const { view, error } = useReference(ref);
  const fields = view?.effective.fields ?? {};
  const changeRef = (value: DefinitionRef | null) => {
    const changed =
      body.kind === 'OBJECT_MODEL'
        ? { ...body, object_type: value ?? emptyRef(), values: {} }
        : body.kind === 'RESOURCE_MODEL'
          ? { ...body, resource_type: value ?? emptyRef(), values: {} }
          : { ...body, base: value ?? emptyRef(), values: {} };
    if (
      !Object.keys(body.values).length ||
      window.confirm('Changing the base clears local field values. Continue?')
    )
      onChange(changed);
  };
  return (
    <>
      <ReferenceSelect
        title={body.kind === 'RESOURCE_INSTANCE' ? 'Base type or model' : 'Type'}
        value={ref}
        kinds={
          body.kind === 'OBJECT_MODEL'
            ? ['OBJECT_TYPE']
            : body.kind === 'RESOURCE_MODEL'
              ? ['RESOURCE_TYPE']
              : ['RESOURCE_TYPE', 'RESOURCE_MODEL']
        }
        options={options}
        onChange={changeRef}
      />
      {error && <p role="alert">{error}</p>}
      {ref.id && !view && !error && <p>Loading inherited fields…</p>}
      {Object.entries(fields).map(([key, field]) => (
        <div className="definition-value" key={key}>
          <div>
            <b>
              {key}
              {field.required ? ' · Required' : ''}
            </b>
            <small>
              Property r{field.property.revision} · {field.specification.unit}
            </small>
          </div>
          {body.values[key] ? (
            <>
              <ValueEditor
                title={key}
                spec={field.specification}
                value={body.values[key]}
                onChange={(value) =>
                  onChange({ ...body, values: { ...body.values, [key]: value } })
                }
              />
              <button
                type="button"
                onClick={() => {
                  const values = { ...body.values };
                  delete values[key];
                  onChange({ ...body, values });
                }}
              >
                Use inherited value / unset
              </button>
            </>
          ) : (
            <>
              <p>
                {view?.effective.values[key]
                  ? `Inherited: ${valueText(view.effective.values[key].value)}`
                  : 'Not set'}
              </p>
              <button
                type="button"
                onClick={() =>
                  onChange({
                    ...body,
                    values: {
                      ...body.values,
                      [key]:
                        view?.effective.values[key]?.value ?? initialValue(field.specification),
                    },
                  })
                }
              >
                Set local value
              </button>
            </>
          )}
        </div>
      ))}
      {Object.keys(body.values)
        .filter((key) => !fields[key])
        .map((key) => (
          <p role="alert" key={key}>
            Unresolved field: {key}. Keep the current base until its fields can be loaded.
          </p>
        ))}
    </>
  );
}
function TaskEditor({
  body,
  options,
  onChange,
}: {
  body: Extract<DefinitionBody, { kind: 'TASK' }>;
  options: DefinitionSummary[];
  onChange: (b: DefinitionBody) => void;
}) {
  const [slotKey, setSlotKey] = useState('');
  const meta = body.metadata;
  return (
    <>
      <div className="definition-grid">
        <Field title="Display name">
          <input
            value={meta.display_name}
            onChange={(e) =>
              onChange({ ...body, metadata: { ...meta, display_name: e.target.value } })
            }
          />
        </Field>
        <Field title="Action category">
          <input
            value={meta.category}
            onChange={(e) => onChange({ ...body, metadata: { ...meta, category: e.target.value } })}
          />
        </Field>
        <Field title="Completion description">
          <textarea
            value={meta.completion_description}
            onChange={(e) =>
              onChange({ ...body, metadata: { ...meta, completion_description: e.target.value } })
            }
          />
        </Field>
        <Field title="Default timeout (nanoseconds; blank means unset)">
          <input
            inputMode="numeric"
            value={meta.default_timeout_ns ?? ''}
            onChange={(e) =>
              onChange({
                ...body,
                metadata: { ...meta, default_timeout_ns: e.target.value || null },
              })
            }
          />
        </Field>
      </div>
      <Toggle
        title="Available to all services"
        value={meta.services.kind === 'ALL'}
        onChange={(all) =>
          onChange({
            ...body,
            metadata: { ...meta, services: all ? { kind: 'ALL' } : { kind: 'ONLY', names: [] } },
          })
        }
      />
      {meta.services.kind === 'ONLY' && (
        <Field title="Service names (one per line)">
          <textarea
            value={meta.services.names.join('\n')}
            onChange={(e) =>
              onChange({
                ...body,
                metadata: {
                  ...meta,
                  services: { kind: 'ONLY', names: e.target.value.split('\n') },
                },
              })
            }
          />
        </Field>
      )}
      <h4>Context slots</h4>
      {Object.entries(body.slots).map(([key, slot]) => {
        const update = (next: typeof slot) =>
          onChange({ ...body, slots: { ...body.slots, [key]: next } });
        return (
          <section className="definition-value" key={key}>
            <div className="definition-row">
              <b>{key}</b>
              <button
                type="button"
                onClick={() => {
                  const slots = { ...body.slots };
                  delete slots[key];
                  onChange({ ...body, slots });
                }}
              >
                Remove slot
              </button>
            </div>
            <Field title="Slot label">
              <input
                value={slot.label}
                onChange={(e) => update({ ...slot, label: e.target.value })}
              />
            </Field>
            <Field title="Slot kind">
              <select
                value={slot.kind}
                onChange={(e) =>
                  update({
                    ...slot,
                    kind: e.target.value as typeof slot.kind,
                    accepted_types: [],
                    multiple: false,
                  })
                }
              >
                <option>OBJECT</option>
                <option>RESOURCE</option>
              </select>
            </Field>
            <Toggle
              title="Required context"
              value={slot.required}
              onChange={(required) => update({ ...slot, required })}
            />
            {slot.kind === 'RESOURCE' && (
              <Toggle
                title="Allow multiple targets"
                value={slot.multiple}
                onChange={(multiple) => update({ ...slot, multiple })}
              />
            )}
            {slot.accepted_types.map((ref) => (
              <div className="definition-row" key={refKey(ref)}>
                <span>
                  {options.find((v) => refKey(v.reference) === refKey(ref))?.label ?? ref.id} · r
                  {ref.revision}
                </span>
                <button
                  type="button"
                  onClick={() =>
                    update({
                      ...slot,
                      accepted_types: slot.accepted_types.filter((r) => refKey(r) !== refKey(ref)),
                    })
                  }
                >
                  Remove type
                </button>
              </div>
            ))}
            <ReferenceSelect
              title="Add accepted type"
              value={null}
              kinds={slot.kind === 'OBJECT' ? ['OBJECT_TYPE'] : ['RESOURCE_TYPE']}
              options={options}
              onChange={(ref) => {
                if (ref && !slot.accepted_types.some((r) => refKey(r) === refKey(ref)))
                  update({ ...slot, accepted_types: [...slot.accepted_types, ref] });
              }}
            />
          </section>
        );
      })}
      <div className="definition-row">
        <Field title="New slot key">
          <input
            value={slotKey}
            onChange={(e) => setSlotKey(e.target.value)}
            placeholder="object, from or to"
          />
        </Field>
        <button
          type="button"
          disabled={!slotKey.trim() || slotKey.trim() in body.slots}
          onClick={() => {
            const key = slotKey.trim();
            onChange({
              ...body,
              slots: {
                ...body.slots,
                [key]: {
                  label: key,
                  kind: 'RESOURCE',
                  accepted_types: [],
                  required: true,
                  multiple: false,
                },
              },
            });
            setSlotKey('');
          }}
        >
          Add slot
        </button>
      </div>
      <h4>Property contracts</h4>
      {Object.entries(body.properties).map(([key, p]) => {
        const update = (next: typeof p) =>
          onChange({ ...body, properties: { ...body.properties, [key]: next } });
        return (
          <section className="definition-value" key={key}>
            <div className="definition-row">
              <b>{key}</b>
              <Toggle
                title="Required value"
                value={p.required}
                onChange={(required) => update({ ...p, required })}
              />
              <button
                type="button"
                onClick={() => {
                  const properties = { ...body.properties };
                  delete properties[key];
                  onChange({ ...body, properties });
                }}
              >
                Remove property
              </button>
            </div>
            <p>Sources are checked from top to bottom.</p>
            {p.sources.map((s, i) => (
              <div className="definition-row" key={i}>
                <select
                  aria-label={`${key} source ${i + 1}`}
                  value={s.kind}
                  onChange={(e) => {
                    const kind = e.target.value as typeof s.kind;
                    const next =
                      kind === 'CONTEXT'
                        ? { kind, slot: '', field: '' }
                        : kind === 'INPUT'
                          ? { kind, key: '' }
                          : { kind };
                    update({ ...p, sources: p.sources.map((old, at) => (at === i ? next : old)) });
                  }}
                >
                  {['OVERRIDE', 'CONTEXT', 'PROPERTY_SET', 'DEFAULT', 'INPUT'].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </select>
                {s.kind === 'CONTEXT' && (
                  <>
                    <input
                      aria-label="Context slot"
                      placeholder="Slot key"
                      value={s.slot}
                      onChange={(e) =>
                        update({
                          ...p,
                          sources: p.sources.map((old, at) =>
                            at === i ? { ...s, slot: e.target.value } : old,
                          ),
                        })
                      }
                    />
                    <input
                      aria-label="Context field"
                      placeholder="Field key"
                      value={s.field}
                      onChange={(e) =>
                        update({
                          ...p,
                          sources: p.sources.map((old, at) =>
                            at === i ? { ...s, field: e.target.value } : old,
                          ),
                        })
                      }
                    />
                  </>
                )}
                {s.kind === 'INPUT' && (
                  <input
                    aria-label="Runtime input key"
                    value={s.key}
                    onChange={(e) =>
                      update({
                        ...p,
                        sources: p.sources.map((old, at) =>
                          at === i ? { ...s, key: e.target.value } : old,
                        ),
                      })
                    }
                  />
                )}
                <button
                  type="button"
                  disabled={i === 0}
                  onClick={() => {
                    const sources = [...p.sources];
                    [sources[i - 1], sources[i]] = [sources[i], sources[i - 1]];
                    update({ ...p, sources });
                  }}
                >
                  Up
                </button>
                <button
                  type="button"
                  onClick={() => update({ ...p, sources: p.sources.filter((_, at) => at !== i) })}
                >
                  Remove
                </button>
              </div>
            ))}
            <button
              type="button"
              onClick={() => update({ ...p, sources: [...p.sources, { kind: 'INPUT', key: '' }] })}
            >
              Add source
            </button>
            {p.sources.some((s) => s.kind === 'DEFAULT') ? (
              <AssignedValue
                title={`${key} default`}
                reference={p.property}
                value={p.default}
                optional
                onChange={(value) => update({ ...p, default: value })}
              />
            ) : (
              p.default && (
                <button type="button" onClick={() => update({ ...p, default: null })}>
                  Remove unused default
                </button>
              )
            )}
          </section>
        );
      })}
      <AddProperty
        options={options}
        used={Object.keys(body.properties)}
        onAdd={(key, property) =>
          onChange({
            ...body,
            properties: {
              ...body.properties,
              [key]: { property, required: true, sources: [{ kind: 'INPUT', key }], default: null },
            },
          })
        }
      />
    </>
  );
}
export function DefinitionFields({
  body,
  options,
  onChange,
}: {
  body: DefinitionBody;
  options: DefinitionSummary[];
  onChange: (body: DefinitionBody) => void;
}) {
  if (body.kind === 'POINT_PATTERN')
    return (
      <>
        <p>Point rules are versioned package data. Apply a new revision with the rx CLI.</p>
        <pre>{JSON.stringify(body, null, 2)}</pre>
      </>
    );
  if (body.kind === 'PROPERTY')
    return (
      <PropertyEditor
        spec={body.specification}
        onChange={(specification) => onChange({ ...body, specification })}
      />
    );
  if (
    body.kind === 'OBJECT_MODEL' ||
    body.kind === 'RESOURCE_MODEL' ||
    body.kind === 'RESOURCE_INSTANCE'
  )
    return <ModelEditor body={body} options={options} onChange={onChange} />;
  if (body.kind === 'TASK') return <TaskEditor body={body} options={options} onChange={onChange} />;
  if (body.kind === 'PROPERTY_SET')
    return (
      <>
        {Object.entries(body.values).map(([key, item]) => (
          <section className="definition-value" key={key}>
            <AssignedValue
              title={key}
              reference={item.property}
              value={item.value}
              optional={false}
              onChange={(value) => {
                if (value)
                  onChange({ ...body, values: { ...body.values, [key]: { ...item, value } } });
              }}
            />
            <button
              type="button"
              onClick={() => {
                const values = { ...body.values };
                delete values[key];
                onChange({ ...body, values });
              }}
            >
              Remove property
            </button>
          </section>
        ))}
        <SetAddition
          options={options}
          used={Object.keys(body.values)}
          onAdd={(key, property, value) =>
            onChange({ ...body, values: { ...body.values, [key]: { property, value } } })
          }
        />
      </>
    );
  return (
    <>
      <ReferenceSelect
        title="Parent type"
        optional
        value={body.parent}
        kinds={[body.kind]}
        options={options}
        onChange={(parent) => onChange({ ...body, parent })}
      />
      <p>Inherited fields are fixed by their saved parent revision. Add distinct fields here.</p>
      {Object.entries(body.fields).map(([key, field]) => (
        <div className="definition-row" key={key}>
          <b>{key}</b>
          <span>
            {options.find((v) => refKey(v.reference) === refKey(field.property))?.label ??
              field.property.id}{' '}
            · r{field.property.revision}
          </span>
          <Toggle
            title="Required"
            value={field.required}
            onChange={(required) =>
              onChange({ ...body, fields: { ...body.fields, [key]: { ...field, required } } })
            }
          />
          <button
            type="button"
            onClick={() => {
              const fields = { ...body.fields };
              delete fields[key];
              onChange({ ...body, fields });
            }}
          >
            Remove
          </button>
        </div>
      ))}
      <AddProperty
        options={options}
        used={Object.keys(body.fields)}
        onAdd={(key, property) =>
          onChange({ ...body, fields: { ...body.fields, [key]: { property, required: true } } })
        }
      />
    </>
  );
}
function SetAddition({
  options,
  used,
  onAdd,
}: {
  options: DefinitionSummary[];
  used: string[];
  onAdd: (key: string, ref: DefinitionRef, value: DefinitionValue) => void;
}) {
  const [entry, setEntry] = useState<{ key: string; ref: DefinitionRef } | null>(null);
  const { view, error } = useReference(entry?.ref ?? null);
  const b = view?.version.definition.body;
  return (
    <>
      <AddProperty options={options} used={used} onAdd={(key, ref) => setEntry({ key, ref })} />
      {error && <p role="alert">{error}</p>}
      {entry && b?.kind === 'PROPERTY' && (
        <div className="definition-value">
          {!b.specification.overridable || b.specification.category === 'CONSTRAINT' ? (
            <p role="alert">This property does not allow Property Set overrides.</p>
          ) : (
            <button
              type="button"
              onClick={() => {
                onAdd(entry.key, entry.ref, initialValue(b.specification));
                setEntry(null);
              }}
            >
              Include {entry.key} and edit value
            </button>
          )}
        </div>
      )}
    </>
  );
}
