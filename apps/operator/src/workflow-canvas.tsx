import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { arrange, diagram, nodeKinds, ports, type Flow, type Point } from './workflow-graph';
const NODE_WIDTH = 218;
const clamp = (n: number) => Math.max(0, Math.min(100000, Math.round(n)));
export function WorkflowLibrary({
  disabled,
  operations,
  onAdd,
}: {
  disabled: boolean;
  operations: string[];
  onAdd: (kind: string, point?: Point, binding?: string) => void;
}) {
  const [query, setQuery] = useState('');
  function item(kind: string, label: string, binding?: string) {
    if (!label.toLowerCase().includes(query.toLowerCase())) return null;
    return (
      <button
        key={binding ?? kind}
        className="wf-library-item"
        disabled={disabled}
        draggable={!disabled}
        onDragStart={(e) =>
          e.dataTransfer.setData(
            'application/x-rx-workflow-node',
            JSON.stringify({ kind, binding }),
          )
        }
        onClick={() => onAdd(kind, undefined, binding)}
      >
        <span className={`wf-kind-icon kind-${kind.toLowerCase()}`}>
          {kind === 'OPERATION'
            ? '□'
            : kind === 'BRANCH'
              ? '◇'
              : kind === 'PARALLEL_ALL'
                ? '⑂'
                : kind === 'REPEAT'
                  ? '↻'
                  : '▣'}
        </span>
        <span>
          <b>{label}</b>
          <small>
            {binding
              ? 'Operation reference'
              : kind === 'PARALLEL_ALL'
                ? 'Wait for every branch'
                : kind === 'REPEAT'
                  ? 'Finite repetition'
                  : 'Add to this workflow'}
          </small>
        </span>
      </button>
    );
  }
  return (
    <aside className="wf-library" aria-label="Workflow library">
      <h3>Add to workflow</h3>
      <input
        aria-label="Search workflow library"
        placeholder="Search nodes and operations"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      <h4>Nodes</h4>
      {Object.entries(nodeKinds).map(([kind, label]) => item(kind, label))}
      {operations.length > 0 && (
        <>
          <h4>Operations in this draft</h4>
          {operations.map((binding) => item('OPERATION', binding, binding))}
        </>
      )}
      <p>
        Drop onto the canvas or select an item. Operation references still require device bindings.
      </p>
    </aside>
  );
}
export function WorkflowCanvas({
  flow,
  positions,
  selected,
  disabled,
  onSelect,
  onOpenFlow,
  onMove,
  onArrange,
  onConnect,
  onAdd,
}: {
  flow: Flow;
  positions: Record<string, Point>;
  selected: number;
  disabled: boolean;
  onSelect: (index: number) => void;
  onOpenFlow: (id: string) => void;
  onMove: (id: string, point: Point) => void;
  onArrange: (positions: Record<string, Point>) => void;
  onConnect: (index: number, port: string, target: string) => void;
  onAdd: (kind: string, point?: Point, binding?: string) => void;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  const [zoom, setZoom] = useState(1);
  const [columns, setColumns] = useState(2);
  const [pending, setPending] = useState<{ node: string; port: string } | null>(null);
  const [dragging, setDragging] = useState<{ id: string; point: Point } | null>(null);
  const drag = useRef<{ id: string; point: Point; x: number; y: number; moved: boolean } | null>(
    null,
  );
  const graph = useMemo(() => diagram(flow, columns), [flow, columns]);
  const vertices = useMemo(() => new Map(graph.nodes.map((node) => [node.key, node])), [graph]);
  const fallback = useMemo(() => arrange(flow, columns), [flow, columns]);
  const arrowId = useId();
  const point = (id: string): Point =>
    dragging?.id === id ? dragging.point : (positions[id] ?? fallback[id] ?? { x: 48, y: 48 });
  const graphPoint = (key: string): Point => {
    const vertex = vertices.get(key);
    if (!vertex) return { x: 48, y: 48 };
    const id = flow.nodes[vertex.sourceIndex].id;
    if (!vertex.boundary) return point(id);
    const owner = point(id),
      original = fallback[id];
    return {
      x: clamp(vertex.point.x + owner.x - original.x),
      y: clamp(vertex.point.y + owner.y - original.y),
    };
  };
  const width = Math.max(800, ...graph.nodes.map((n) => graphPoint(n.key).x + NODE_WIDTH + 80));
  const height = Math.max(540, ...graph.nodes.map((n) => graphPoint(n.key).y + 180));
  function fit() {
    if (!scroll.current) return;
    setZoom(
      Math.max(
        0.3,
        Math.min(
          1,
          (scroll.current.clientWidth - 24) / width,
          (scroll.current.clientHeight - 24) / height,
        ),
      ),
    );
    scroll.current.scrollTo(0, 0);
  }
  useEffect(() => {
    setZoom(1);
    scroll.current?.scrollTo(0, 0);
    setPending(null);
  }, [flow.id]);
  useEffect(() => {
    if (!scroll.current) return;
    const element = scroll.current;
    const resize = new ResizeObserver(() =>
      setColumns(Math.max(1, Math.min(6, Math.floor((element.clientWidth - 48) / 280)))),
    );
    resize.observe(element);
    return () => resize.disconnect();
  }, []);
  const referenced = new Set(flow.nodes.flatMap((node) => ports(node).map((port) => port.target)));
  const issues = flow.nodes.filter(
    (node) => node.id !== flow.root && !referenced.has(node.id),
  ).length;
  return (
    <section className="wf-canvas" aria-label="Workflow canvas">
      <div className="wf-canvas-toolbar">
        <div>
          <h3>Action sequence</h3>
          <span>
            {flow.nodes.length} nodes{issues ? ` · ${issues} unconnected` : ''}
          </span>
        </div>
        <div className="wf-canvas-tools">
          <button aria-label="Zoom out" onClick={() => setZoom((z) => Math.max(0.4, z - 0.1))}>
            −
          </button>
          <button aria-label="Reset zoom" onClick={() => setZoom(1)}>
            {Math.round(zoom * 100)}%
          </button>
          <button aria-label="Zoom in" onClick={() => setZoom((z) => Math.min(1.6, z + 0.1))}>
            +
          </button>
          <button disabled={disabled} onClick={() => onArrange(fallback)}>
            Arrange
          </button>
          <button onClick={fit}>Fit</button>
        </div>
      </div>
      <p className="wf-connection-hint" role="status">
        {pending
          ? 'Select the input of another node. Escape cancels the connection.'
          : 'Arrows show execution order. Edit the order in node settings. Moving nodes only changes the layout.'}
      </p>
      {graph.issues.length > 0 && (
        <p className="wf-diagram-issues" role="alert">
          Diagram needs review: {graph.issues.slice(0, 3).join('; ')}
          {graph.issues.length > 3 ? `; ${graph.issues.length - 3} more` : ''}
        </p>
      )}
      <div
        className="wf-canvas-scroll"
        ref={scroll}
        tabIndex={0}
        role="region"
        aria-label="Scrollable action graph"
        onKeyDown={(e) => {
          if (e.key === 'Escape') setPending(null);
        }}
        onDragOver={(e) => {
          if (!disabled && e.dataTransfer.types.includes('application/x-rx-workflow-node'))
            e.preventDefault();
        }}
        onDrop={(e) => {
          e.preventDefault();
          if (disabled || !scroll.current) return;
          try {
            const value: unknown = JSON.parse(
              e.dataTransfer.getData('application/x-rx-workflow-node'),
            );
            if (
              !value ||
              typeof value !== 'object' ||
              !('kind' in value) ||
              typeof value.kind !== 'string' ||
              !nodeKinds[value.kind]
            )
              return;
            const box = scroll.current.getBoundingClientRect();
            onAdd(
              value.kind,
              {
                x: clamp((e.clientX - box.left + scroll.current.scrollLeft) / zoom),
                y: clamp((e.clientY - box.top + scroll.current.scrollTop) / zoom),
              },
              'binding' in value && typeof value.binding === 'string' ? value.binding : undefined,
            );
          } catch {
            /* Foreign drag content is not a workflow node. */
          }
        }}
      >
        <div style={{ width: width * zoom, height: height * zoom }}>
          <div className="wf-plane" style={{ width, height, transform: `scale(${zoom})` }}>
            <svg
              className="wf-edges"
              width={width}
              height={height}
              aria-label="Authored execution order"
            >
              <defs>
                <marker
                  id={arrowId}
                  viewBox="0 0 10 10"
                  refX="9"
                  refY="5"
                  markerWidth="7"
                  markerHeight="7"
                  orient="auto-start-reverse"
                >
                  <path className="wf-arrow-head" d="M 0 0 L 10 5 L 0 10 z" />
                </marker>
              </defs>
              {graph.edges.map((edge, index) => {
                const a = graphPoint(edge.from),
                  b = graphPoint(edge.to);
                const bend = Math.max(60, Math.abs(b.x - a.x - NODE_WIDTH) / 2);
                const wrapped = !edge.repeat && b.y > a.y && b.x <= a.x;
                const path = wrapped
                  ? `M${a.x + NODE_WIDTH},${a.y + 57} H${a.x + NODE_WIDTH + 24} V${(a.y + b.y) / 2 + 57} H${b.x - 24} V${b.y + 57} H${b.x}`
                  : `M${a.x + NODE_WIDTH},${a.y + 57} C${a.x + NODE_WIDTH + bend},${a.y + 57} ${b.x - bend},${b.y + 57} ${b.x},${b.y + 57}`;
                return (
                  <g key={index} data-from={edge.from} data-to={edge.to}>
                    <title>
                      {edge.label || 'Next'}: {edge.from} → {edge.to}
                    </title>
                    <path
                      className={edge.repeat ? 'wf-repeat-edge' : undefined}
                      markerEnd={`url(#${arrowId})`}
                      d={path}
                    />
                    <text x={(a.x + NODE_WIDTH + b.x) / 2} y={(a.y + b.y) / 2 + 49}>
                      {edge.label}
                    </text>
                  </g>
                );
              })}
            </svg>
            {graph.nodes
              .filter((n) => n.boundary)
              .map((n) => {
                const at = graphPoint(n.key);
                return (
                  <button
                    key={n.key}
                    className="wf-boundary"
                    style={{ left: at.x, top: at.y + 24, width: NODE_WIDTH }}
                    onClick={() => onSelect(n.sourceIndex)}
                    aria-label={`${n.boundary}: ${flow.nodes[n.sourceIndex].id}`}
                  >
                    <small>Continue when</small>
                    <strong>{n.boundary}</strong>
                  </button>
                );
              })}
            {flow.nodes.map((node, index) => {
              const at = point(node.id),
                kind = String(node.body.kind),
                label =
                  kind === 'OPERATION'
                    ? String(node.body.binding || 'Operation binding required')
                    : kind === 'CALL'
                      ? String(node.body.flow || 'Select workflow')
                      : node.id;
              return (
                <article
                  key={`${node.id}/${index}`}
                  className={`wf-node kind-${kind.toLowerCase()} ${index === selected ? 'selected' : ''} ${node.id === flow.root ? 'root-node' : ''}`}
                  style={{ left: at.x, top: at.y, width: NODE_WIDTH }}
                  aria-label={`${nodeKinds[kind] ?? 'Unsupported node'} ${label}`}
                >
                  <button
                    className="wf-input-port"
                    aria-label={`Input ${node.id}`}
                    title="Connect this input"
                    disabled={disabled || !pending}
                    onClick={() => {
                      if (pending) {
                        const source = flow.nodes.findIndex((n) => n.id === pending.node);
                        onConnect(source, pending.port, node.id);
                        setPending(null);
                      }
                    }}
                  />
                  <button
                    className="wf-node-heading"
                    aria-label={`Select ${node.id}`}
                    onClick={() => onSelect(index)}
                  >
                    <span>{nodeKinds[kind] ?? 'Unsupported node'}</span>
                    {node.id === flow.root && <small>Entry</small>}
                    <strong>{label}</strong>
                  </button>
                  <button
                    className="wf-drag-handle"
                    aria-label={`Move ${node.id}`}
                    title="Drag to move; arrow keys move by 20 pixels"
                    disabled={disabled}
                    onKeyDown={(e) => {
                      const delta: Record<string, Point> = {
                        ArrowLeft: { x: -20, y: 0 },
                        ArrowRight: { x: 20, y: 0 },
                        ArrowUp: { x: 0, y: -20 },
                        ArrowDown: { x: 0, y: 20 },
                      };
                      if (delta[e.key]) {
                        e.preventDefault();
                        onMove(node.id, {
                          x: clamp(at.x + delta[e.key].x),
                          y: clamp(at.y + delta[e.key].y),
                        });
                      }
                    }}
                    onPointerDown={(e) => {
                      if (disabled) return;
                      e.currentTarget.setPointerCapture(e.pointerId);
                      drag.current = {
                        id: node.id,
                        point: at,
                        x: e.clientX,
                        y: e.clientY,
                        moved: false,
                      };
                      onSelect(index);
                    }}
                    onPointerMove={(e) => {
                      const d = drag.current;
                      if (!d || d.id !== node.id) return;
                      const dx = (e.clientX - d.x) / zoom,
                        dy = (e.clientY - d.y) / zoom;
                      if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
                      if (d.moved)
                        setDragging({
                          id: d.id,
                          point: { x: clamp(d.point.x + dx), y: clamp(d.point.y + dy) },
                        });
                    }}
                    onPointerUp={(e) => {
                      const d = drag.current;
                      if (d?.moved)
                        onMove(d.id, {
                          x: clamp(d.point.x + (e.clientX - d.x) / zoom),
                          y: clamp(d.point.y + (e.clientY - d.y) / zoom),
                        });
                      drag.current = null;
                      setDragging(null);
                    }}
                    onPointerCancel={() => {
                      drag.current = null;
                      setDragging(null);
                    }}
                  >
                    ⠿
                  </button>
                  <div className="wf-node-ports">
                    {ports(node).map((port) => (
                      <button
                        key={port.key}
                        className={port.target ? 'connected' : ''}
                        disabled={disabled}
                        aria-label={`Connect ${node.id} ${port.key}`}
                        title={
                          port.target ? `${port.label} → ${port.target}` : 'Connect another child'
                        }
                        onClick={() => {
                          onSelect(index);
                          setPending({ node: node.id, port: port.key });
                        }}
                      >
                        {port.label}
                        <span>●</span>
                      </button>
                    ))}
                    {kind === 'CALL' ? (
                      <button
                        aria-label={`Open workflow ${String(node.body.flow ?? '')}`}
                        onClick={() => onOpenFlow(String(node.body.flow ?? ''))}
                      >
                        Open workflow
                      </button>
                    ) : (
                      ports(node).length === 0 && <small>Returns to its enclosing flow</small>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
        </div>
      </div>
    </section>
  );
}
