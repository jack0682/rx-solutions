import { describe, expect, it } from 'vitest';
import { arrange, connect, diagram, ports, type Flow } from './workflow-graph';
const flow = (): Flow => ({
  id: 'main',
  root: 'sequence',
  nodes: [
    { id: 'sequence', body: { kind: 'SEQUENCE', children: ['a', 'b'] } },
    { id: 'a', body: { kind: 'OPERATION', binding: 'pick' } },
    { id: 'b', body: { kind: 'OPERATION', binding: 'place' } },
    { id: 'branch', body: { kind: 'BRANCH', condition: 'ready', when_true: '', when_false: '' } },
  ],
});
describe('execution-order projection', () => {
  const edges = (source: Flow) => diagram(source).edges.map(({ from, to }) => [from, to]);
  it('folds a long sequence to viewport-width bands without altering its transitions', () => {
    const source: Flow = {
      id: 'main',
      root: 'sequence',
      nodes: [
        {
          id: 'sequence',
          body: { kind: 'SEQUENCE', children: Array.from({ length: 20 }, (_, i) => `step-${i}`) },
        },
        ...Array.from({ length: 20 }, (_, i) => ({ id: `step-${i}`, body: { kind: 'OPERATION' } })),
      ],
    };
    const narrow = diagram(source, 2),
      wide = diagram(source, 6);
    expect(narrow.edges).toEqual(wide.edges);
    expect(new Set(narrow.nodes.map((n) => `${n.point.x}/${n.point.y}`)).size).toBe(21);
    expect(narrow.nodes.every((n) => n.point.x < 600)).toBe(true);
    expect(narrow.nodes.find((n) => n.key === 'node/2')!.point.y).toBeGreaterThan(
      narrow.nodes[0].point.y,
    );
  });
  it('shows actual sequence order instead of a fan-out, without rewriting the source', () => {
    const source = flow();
    const before = structuredClone(source);
    expect(edges(source)).toContainEqual(['node/0', 'node/1']);
    expect(edges(source)).toContainEqual(['node/1', 'node/2']);
    expect(edges(source)).not.toContainEqual(['node/0', 'node/2']);
    expect(arrange(source).a.x).toBeLessThan(arrange(source).b.x);
    const reordered = structuredClone(source);
    reordered.nodes[0].body.children = ['b', 'a'];
    expect(edges(reordered)).toContainEqual(['node/2', 'node/1']);
    expect(source).toEqual(before);
  });
  it('continues after a parallel join, including a nested branch, not after either leaf', () => {
    const source: Flow = {
      id: 'main',
      root: 'sequence',
      nodes: [
        { id: 'sequence', body: { kind: 'SEQUENCE', children: ['parallel', 'finish'] } },
        { id: 'parallel', body: { kind: 'PARALLEL_ALL', children: ['branch', 'other'] } },
        { id: 'branch', body: { kind: 'BRANCH', when_true: 'yes', when_false: 'no' } },
        { id: 'yes', body: { kind: 'OPERATION' } },
        { id: 'no', body: { kind: 'OPERATION' } },
        { id: 'other', body: { kind: 'OPERATION' } },
        { id: 'finish', body: { kind: 'OPERATION' } },
      ],
    };
    const graph = diagram(source);
    expect(graph.issues).toEqual([]);
    expect(edges(source)).toContainEqual(['node/3', 'join/2']);
    expect(edges(source)).toContainEqual(['node/4', 'join/2']);
    expect(edges(source)).toContainEqual(['join/2', 'join/1']);
    expect(edges(source)).toContainEqual(['node/5', 'join/1']);
    expect(graph.edges.filter((e) => e.to === 'node/6')).toEqual([
      { from: 'join/1', to: 'node/6', label: '2' },
    ]);
    expect(graph.nodes.find((n) => n.key === 'join/1')?.boundary).toBe('All paths complete');
    expect(Object.keys(arrange(source))).toHaveLength(source.nodes.length);
  });
  it('shows the repeat body returning to its control and a separate counted exit', () => {
    const source: Flow = {
      id: 'main',
      root: 'sequence',
      nodes: [
        { id: 'sequence', body: { kind: 'SEQUENCE', children: ['repeat', 'finish'] } },
        { id: 'repeat', body: { kind: 'REPEAT', count: '3', child: 'body' } },
        { id: 'body', body: { kind: 'OPERATION' } },
        { id: 'finish', body: { kind: 'CALL', flow: 'another' } },
      ],
    };
    expect(diagram(source).edges).toContainEqual({
      from: 'node/2',
      to: 'node/1',
      label: 'Next iteration',
      repeat: true,
    });
    expect(diagram(source).edges).toContainEqual({
      from: 'node/1',
      to: 'join/1',
      label: 'After 3 iterations',
    });
    expect(edges(source)).toContainEqual(['join/1', 'node/3']);
    expect(edges(source)).not.toContainEqual(['node/2', 'node/3']);
  });
  it('reports incomplete, shared and cyclic source without inventing a bypass', () => {
    const source = flow();
    source.nodes[0].body.children = ['a', 'missing', 'b'];
    expect(diagram(source).issues).toContain('Missing node: missing');
    expect(edges(source)).not.toContainEqual(['node/1', 'node/2']);
    source.nodes[0].body.children = ['a', 'a', 'sequence'];
    expect(diagram(source).issues.some((s) => s.includes('Shared, cyclic'))).toBe(true);
    expect(diagram(source).nodes.filter((n) => !n.boundary)).toHaveLength(4);
  });
});
describe('source-connected workflow graph', () => {
  it('keeps semantic order independent from placement and shows unconnected nodes', () => {
    const source = flow(),
      before = JSON.stringify(source);
    expect(Object.keys(arrange(source))).toHaveLength(4);
    expect(JSON.stringify(source)).toBe(before);
    expect(ports(source.nodes[0]).map((p) => p.target)).toEqual(['a', 'b', '']);
  });
  it('writes explicit branch and child references without silently deleting another reference', () => {
    const source = flow(),
      branch = connect(source, 3, 'when_true', 'a');
    expect(branch.nodes[3].body.when_true).toBe('a');
    expect(branch.nodes[0].body.children).toEqual(['a', 'b']);
    expect(source.nodes[3].body.when_true).toBe('');
    expect(connect(source, 0, 'children/0', 'branch').nodes[0].body.children).toEqual([
      'branch',
      'b',
    ]);
  });
  it('rejects unsupported outputs, missing nodes, self-links and cycles', () => {
    const source = flow();
    expect(() => connect(source, 1, 'success', 'b')).toThrow('output');
    expect(() => connect(source, 0, 'append', 'missing')).toThrow();
    expect(() => connect(source, 0, 'append', 'sequence')).toThrow();
    const cyclic = connect(source, 0, 'append', 'branch');
    expect(() => connect(cyclic, 3, 'when_true', 'sequence')).toThrow('cycle');
    expect(() => connect(source, 0, 'append', 'a')).toThrow('already');
  });
});
