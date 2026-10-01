import { useEffect, useState } from 'react';
import { fetchName } from './definition-names';
import { refKey, type DefinitionRef } from './definition-schema';

export function useDefinitionNames(refs: DefinitionRef[], known: Record<string, string>) {
  const signature = JSON.stringify({ refs, known });
  const [state, setState] = useState<{ signature: string; names: Record<string, string> } | null>(
    null,
  );
  useEffect(() => {
    const controller = new AbortController();
    const names = { ...known };
    const pending = refs.filter((r) => !(refKey(r) in names));
    let index = 0;
    async function worker() {
      while (index < pending.length && !controller.signal.aborted) {
        const ref = pending[index++];
        try {
          names[refKey(ref)] = await fetchName(ref, controller.signal);
        } catch {
          /* Keep identifiers available without inventing a name. */
        }
      }
    }
    void Promise.all(Array.from({ length: Math.min(4, pending.length) }, worker)).then(() => {
      if (!controller.signal.aborted) setState({ signature, names });
    });
    return () => controller.abort();
  }, [signature]);
  return state?.signature === signature ? state.names : known;
}

export function DefinitionName({
  reference,
  names,
}: {
  reference: DefinitionRef;
  names: Record<string, string>;
}) {
  return (
    <span
      title={`Catalog ${reference.catalog}\nID ${reference.id}\nr${reference.revision}\nSHA-256 ${reference.digest}`}
    >
      {names[refKey(reference)] ?? 'Name unavailable'} · r{reference.revision}
    </span>
  );
}
