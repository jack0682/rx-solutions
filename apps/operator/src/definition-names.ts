import { api } from './api';
import {
  definitionViewSchema,
  refKey,
  type DefinitionRef,
  type DefinitionView,
} from './definition-schema';

export function pinnedName(ref: DefinitionRef, response: unknown): string {
  const definition = definitionViewSchema.parse(response).version.definition;
  if (refKey(definition.reference) !== refKey(ref))
    throw new Error('Definition name reference differs');
  return definition.label;
}

export function provenanceRefs(view: DefinitionView): DefinitionRef[] {
  const e = view.effective;
  const refs = [
    view.version.definition.reference,
    ...Object.values(e.fields).map((f) => f.declared_by),
    ...Object.values(e.values).map((v) => v.declared_by),
    ...Object.values(e.shadowed)
      .flat()
      .map((v) => v.declared_by),
  ];
  return [...new Map(refs.map((r) => [refKey(r), r])).values()];
}

export async function fetchName(ref: DefinitionRef, signal: AbortSignal) {
  const params = new URLSearchParams({ catalog: ref.catalog, id: ref.id, revision: ref.revision });
  return pinnedName(ref, await api(`/api/v1/definition?${params}`, { signal }));
}

export function conflictText(expected: string | null, label: string, revision: string): string {
  return expected === null
    ? `STALE_REVISION: expected=null is create-only; “${label}” already exists in the current read at r${revision}. Use a new ID to create, or inspect the existing definition and explicitly set expected="${revision}" to revise it.`
    : `STALE_REVISION: expected=r${expected}; current read is “${label}” r${revision}. Reopen and review changes before submitting a new revision.`;
}

export async function definitionConflict(command: Record<string, unknown>): Promise<string | null> {
  if (
    typeof command.catalog !== 'string' ||
    typeof command.id !== 'string' ||
    !(command.expected === null || typeof command.expected === 'string')
  )
    return null;
  try {
    const params = new URLSearchParams({ catalog: command.catalog, id: command.id });
    const view = definitionViewSchema.parse(await api(`/api/v1/definition?${params}`));
    const d = view.version.definition;
    if (d.reference.catalog !== command.catalog || d.reference.id !== command.id) return null;
    return conflictText(command.expected, d.label, d.reference.revision);
  } catch {
    return null;
  }
}
