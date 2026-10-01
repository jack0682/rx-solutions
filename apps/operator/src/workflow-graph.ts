import type { EditableSource } from './draft-schema';
export type Flow = EditableSource['flows'][number];
export type Point = { x: number; y: number };
export const nodeKinds: Record<string, string> = {
  OPERATION: 'Operation',
  SEQUENCE: 'Sequence',
  PARALLEL_ALL: 'Parallel',
  BRANCH: 'Branch',
  REPEAT: 'Repeat',
  CALL: 'Call workflow',
  WAIT: 'Wait for condition',
  INTERVENTION: 'Operator intervention',
};
export type Port = { key: string; label: string; target: string };
const text = (v: unknown) => (typeof v === 'string' ? v : '');
export function ports(node: Flow['nodes'][number]): Port[] {
  const b = node.body;
  if (b.kind === 'SEQUENCE' || b.kind === 'PARALLEL_ALL') {
    return (Array.isArray(b.children) ? b.children : [])
      .map((id, i) => ({
        key: `children/${i}`,
        label: b.kind === 'SEQUENCE' ? `${i + 1}` : `Path ${i + 1}`,
        target: text(id),
      }))
      .concat([{ key: 'append', label: '+', target: '' }]);
  }
  if (b.kind === 'BRANCH')
    return [
      { key: 'when_true', label: 'True', target: text(b.when_true) },
      { key: 'when_false', label: 'False', target: text(b.when_false) },
    ];
  if (b.kind === 'REPEAT') return [{ key: 'child', label: 'Body', target: text(b.child) }];
  return [];
}
export type DiagramNode = {
  key: string;
  sourceIndex: number;
  point: Point;
  boundary?: string;
};
export type DiagramEdge = { from: string; to: string; label: string; repeat?: boolean };
export type Diagram = { nodes: DiagramNode[]; edges: DiagramEdge[]; issues: string[] };
/** A projection of v1 control semantics, never a second executable workflow. */
export function diagram(flow: Flow, columns = 3): Diagram {
  const result: Diagram = { nodes: [], edges: [], issues: [] };
  const indexes = new Map<string, number>();
  flow.nodes.forEach((node, index) => {
    if (indexes.has(node.id)) result.issues.push(`Duplicate node: ${node.id}`);
    else indexes.set(node.id, index);
  });
  const visited = new Set<number>();
  type Span = { exits: string[]; column: number; bottom: number };
  const edge = (from: string, to: string, label = '', repeat = false) =>
    result.edges.push({ from, to, label, ...(repeat ? { repeat } : {}) });
  const add = (
    key: string,
    sourceIndex: number,
    column: number,
    row: number,
    boundary?: string,
  ) => {
    result.nodes.push({
      key,
      sourceIndex,
      point: { x: 48 + column * 280, y: 48 + row * 180 },
      ...(boundary ? { boundary } : {}),
    });
    return key;
  };
  function visit(id: string, column: number, row: number, depth: number): Span {
    const index = indexes.get(id);
    const empty = { exits: [], column, bottom: row };
    if (index === undefined) {
      result.issues.push(`Missing node: ${id || '(not connected)'}`);
      return empty;
    }
    if (visited.has(index) || depth > 64) {
      result.issues.push(`Shared, cyclic or too deeply nested node: ${id}`);
      return empty;
    }
    visited.add(index);
    const key = add(`node/${index}`, index, column, row);
    const node = flow.nodes[index];
    const children = ports(node).filter((port) => port.key !== 'append');
    function child(port: Port, x: number, y: number, from: string[], label: string): Span {
      const span = visit(port.target, x, y, depth + 1);
      const target = indexes.get(port.target);
      if (target !== undefined) for (const start of from) edge(start, `node/${target}`, label);
      return span;
    }
    if (node.body.kind === 'SEQUENCE') {
      let span: Span = { exits: [key], column: column + 1, bottom: row };
      if (!children.length) result.issues.push(`Empty sequence: ${id}`);
      for (const port of children) {
        const next = child(port, span.column, row, span.exits, port.label);
        span = { ...next, bottom: Math.max(span.bottom, next.bottom) };
      }
      return children.length ? span : { ...span, exits: [] };
    }
    if (node.body.kind === 'PARALLEL_ALL' || node.body.kind === 'BRANCH') {
      let nextRow = row,
        right = column + 1;
      const exits: string[] = [];
      if (!children.length) result.issues.push(`Empty parallel: ${id}`);
      for (const port of children) {
        const span = child(port, column + 1, nextRow, [key], port.label);
        exits.push(...span.exits);
        right = Math.max(right, span.column);
        nextRow = span.bottom + 1;
      }
      const join = add(
        `join/${index}`,
        index,
        right,
        row,
        node.body.kind === 'PARALLEL_ALL' ? 'All paths complete' : 'Selected path complete',
      );
      for (const exit of exits) edge(exit, join);
      return {
        exits: exits.length ? [join] : [],
        column: right + 1,
        bottom: Math.max(row, nextRow - 1),
      };
    }
    if (node.body.kind === 'REPEAT') {
      const body = child(children[0], column + 1, row + 1, [key], 'Body');
      for (const exit of body.exits) edge(exit, key, 'Next iteration', true);
      const done = add(
        `join/${index}`,
        index,
        Math.max(column + 1, body.column),
        row,
        'Repeat complete',
      );
      edge(key, done, `After ${String(node.body.count ?? '?')} iterations`);
      return {
        exits: body.exits.length ? [done] : [],
        column: Math.max(column + 1, body.column) + 1,
        bottom: body.bottom,
      };
    }
    return { exits: [key], column: column + 1, bottom: row };
  }
  let bottom = visit(flow.root, 0, 0, 0).bottom;
  for (let index = 0; index < flow.nodes.length; index++) {
    if (!visited.has(index)) {
      if (indexes.get(flow.nodes[index].id) === index) {
        bottom = visit(flow.nodes[index].id, 0, bottom + 2, 0).bottom;
      } else {
        add(`node/${index}`, index, 0, (bottom += 2));
      }
    }
  }
  // Fold long paths into bands without changing their edges or source ordering.
  const bandColumns = Math.max(1, Math.min(16, Math.floor(columns) || 3));
  const rows = Math.max(1, ...result.nodes.map((node) => (node.point.y - 48) / 180 + 1));
  for (const node of result.nodes) {
    const column = (node.point.x - 48) / 280;
    node.point = {
      x: 48 + (column % bandColumns) * 280,
      y: node.point.y + Math.floor(column / bandColumns) * rows * 180,
    };
  }
  return result;
}
/** Only source-node coordinates are persisted; joins are derived control boundaries. */
export function arrange(flow: Flow, columns = 3): Record<string, Point> {
  return Object.fromEntries(
    diagram(flow, columns)
      .nodes.filter((node) => !node.boundary)
      .map((node) => [
        flow.nodes[node.sourceIndex].id,
        { x: Math.min(100000, node.point.x), y: Math.min(100000, node.point.y) },
      ]),
  );
}
export function connect(flow: Flow, index: number, port: string, target: string): Flow {
  const result = structuredClone(flow);
  const node = result.nodes[index];
  if (!node || node.id === target || !result.nodes.some((n) => n.id === target))
    throw new Error('Choose a different node in this workflow.');
  if (!ports(node).some((p) => p.key === port))
    throw new Error('This output is no longer available.');
  const seen = new Set<string>();
  function reaches(id: string): boolean {
    if (id === node.id) return true;
    if (seen.has(id)) return false;
    seen.add(id);
    const next = result.nodes.find((n) => n.id === id);
    return !!next && ports(next).some((p) => p.target && reaches(p.target));
  }
  if (reaches(target))
    throw new Error(
      'This connection would create a cycle. Use a Repeat node for bounded repetition.',
    );
  if (port === 'append') {
    const children = Array.isArray(node.body.children) ? node.body.children : [];
    if (children.includes(target)) throw new Error('This node is already connected.');
    node.body.children = [...children, target];
  } else if (port.startsWith('children/')) {
    const children = Array.isArray(node.body.children) ? [...node.body.children] : [];
    children[Number(port.split('/')[1])] = target;
    node.body.children = children;
  } else node.body[port] = target;
  return result;
}
