import type { EditableSource } from './draft-schema';
export type PreviewRow = {
  nodeIndex: number;
  nodeId: string;
  depth: number;
  problem: string | null;
};
export function previewRows(source: EditableSource, flowIndex: number, limit = 256): PreviewRow[] {
  const flow = source.flows[flowIndex];
  if (!flow) return [];
  const rows: PreviewRow[] = [];
  const index = new Map(flow.nodes.map((n, i) => [n.id, i]));
  function visit(id: string, path: Set<string>, depth: number) {
    if (rows.length >= limit) return;
    const i = index.get(id);
    if (i === undefined) {
      rows.push({ nodeIndex: -1, nodeId: id, depth, problem: 'Unbound' });
      return;
    }
    if (depth > 32 || path.has(id)) {
      rows.push({ nodeIndex: i, nodeId: id, depth, problem: 'Cycle or display depth exceeded' });
      return;
    }
    rows.push({ nodeIndex: i, nodeId: id, depth, problem: null });
    const body = flow.nodes[i].body;
    let children: string[] = [];
    if (body.kind === 'SEQUENCE' || body.kind === 'PARALLEL_ALL')
      children = Array.isArray(body.children)
        ? body.children.filter((v): v is string => typeof v === 'string')
        : [];
    else if (body.kind === 'BRANCH')
      children = [
        typeof body.when_true === 'string' ? body.when_true : '',
        typeof body.when_false === 'string' ? body.when_false : '',
      ];
    else if (body.kind === 'REPEAT') children = [typeof body.child === 'string' ? body.child : ''];
    const next = new Set(path);
    next.add(id);
    for (const child of children) {
      if (rows.length >= limit) break;
      visit(child, next, depth + 1);
    }
  }
  visit(flow.root, new Set(), 0);
  if (rows.length === limit)
    rows.push({
      nodeIndex: -1,
      nodeId: '',
      depth: 0,
      problem: 'The display limit has been reached. Saving validates the full structure.',
    });
  return rows;
}
