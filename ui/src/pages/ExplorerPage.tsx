import cytoscape from "cytoscape";
import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { apiRequest } from "../lib/api";
import { useConsoleContext } from "../App";

type NodeResult = {
  node?: {
    id: string;
    entity_type: string;
    properties: Record<string, unknown>;
    locality: string;
  };
};

type EdgesResult = {
  items: Array<{
    edge_id: string;
    label: string;
    source?: { entity_type: string; node_id: string };
    target?: { entity_type: string; node_id: string };
    properties: Record<string, unknown>;
  }>;
  next_cursor: string;
};

export default function ExplorerPage() {
  const { session, scope, notify } = useConsoleContext();
  const [entityType, setEntityType] = useState("Person");
  const [nodeId, setNodeId] = useState("1_0");
  const [direction, setDirection] = useState("outgoing");
  const [node, setNode] = useState<NodeResult["node"] | null>(null);
  const [edges, setEdges] = useState<EdgesResult["items"]>([]);
  const [nextCursor, setNextCursor] = useState("");
  const [loading, setLoading] = useState(false);
  const graphRef = useRef<HTMLDivElement | null>(null);

  async function fetchGraph(event?: FormEvent) {
    event?.preventDefault();
    setLoading(true);
    try {
      const nodeResponse = await apiRequest<NodeResult>(
        `/graph/node/${encodeURIComponent(entityType)}/${encodeURIComponent(nodeId)}?consistency=strong`,
        {
          session,
          scope,
        },
      );
      const edgeResponse = await apiRequest<EdgesResult>(
        `/graph/edges?entity_type=${encodeURIComponent(entityType)}&node_id=${encodeURIComponent(nodeId)}&direction=${direction}&limit=300`,
        {
          session,
          scope,
        },
      );
      setNode(nodeResponse.node ?? null);
      setEdges(edgeResponse.items);
      setNextCursor(edgeResponse.next_cursor);
      notify(`Loaded ${edgeResponse.items.length} edge(s)`);
    } catch (error) {
      notify(error instanceof Error ? error.message : "Failed to load node graph", "error");
      setNode(null);
      setEdges([]);
      setNextCursor("");
    } finally {
      setLoading(false);
    }
  }

  const graphElements = useMemo(() => {
    if (!node) {
      return [];
    }

    const elements: cytoscape.ElementDefinition[] = [];
    const rootId = `${node.entity_type}:${node.id}`;
    elements.push({
      data: {
        id: rootId,
        label: `${node.entity_type}:${node.id}`,
        root: true,
      },
    });

    const seen = new Set<string>([rootId]);
    for (const edge of edges.slice(0, 500)) {
      if (!edge.target || !edge.source) {
        continue;
      }
      const sourceId = `${edge.source.entity_type}:${edge.source.node_id}`;
      const targetId = `${edge.target.entity_type}:${edge.target.node_id}`;
      if (!seen.has(sourceId)) {
        seen.add(sourceId);
        elements.push({
          data: {
            id: sourceId,
            label: sourceId,
            root: sourceId === rootId,
          },
        });
      }
      if (!seen.has(targetId)) {
        seen.add(targetId);
        elements.push({
          data: {
            id: targetId,
            label: targetId,
            root: targetId === rootId,
          },
        });
      }
      elements.push({
        data: {
          id: edge.edge_id || `${sourceId}-${edge.label}-${targetId}`,
          source: sourceId,
          target: targetId,
          label: edge.label,
        },
      });
    }

    return elements;
  }, [edges, node]);

  useEffect(() => {
    if (!graphRef.current || graphElements.length === 0) {
      return;
    }

    const instance = cytoscape({
      container: graphRef.current,
      elements: graphElements,
      style: [
        {
          selector: "node",
          style: {
            "background-color": "#116466",
            "border-width": 2,
            "border-color": "#1f2f3a",
            label: "data(label)",
            color: "#f8f2ea",
            "font-size": 10,
            "text-wrap": "wrap",
            "text-max-width": "100px",
            width: 38,
            height: 38,
          },
        },
        {
          selector: "node[root]",
          style: {
            "background-color": "#d64f4f",
            width: 46,
            height: 46,
          },
        },
        {
          selector: "edge",
          style: {
            width: 2,
            "line-color": "#8ea7b1",
            "target-arrow-color": "#8ea7b1",
            "target-arrow-shape": "triangle",
            "curve-style": "bezier",
            label: "data(label)",
            color: "#20303a",
            "font-size": 9,
          },
        },
      ],
      layout: {
        name: "cose",
        animate: false,
        nodeRepulsion: 7000,
      },
    });

    return () => {
      instance.destroy();
    };
  }, [graphElements]);

  return (
    <section className="page-grid two-col">
      <article className="card">
        <h3>Node Explorer</h3>
        <form className="form-grid" onSubmit={fetchGraph}>
          <label>
            Entity Type
            <input value={entityType} onChange={(event) => setEntityType(event.target.value)} />
          </label>
          <label>
            Node ID
            <input value={nodeId} onChange={(event) => setNodeId(event.target.value)} />
          </label>
          <label>
            Direction
            <select value={direction} onChange={(event) => setDirection(event.target.value)}>
              <option value="outgoing">Outgoing</option>
              <option value="incoming">Incoming</option>
              <option value="both">Both</option>
            </select>
          </label>
          <button type="submit" disabled={loading}>
            {loading ? "Loading..." : "Load Graph"}
          </button>
        </form>

        {node ? (
          <div className="json-block">
            <h4>Node</h4>
            <pre>{JSON.stringify(node, null, 2)}</pre>
          </div>
        ) : (
          <p className="empty-state">No node loaded yet.</p>
        )}

        <div className="json-block">
          <h4>Edges ({edges.length})</h4>
          <pre>{JSON.stringify(edges.slice(0, 25), null, 2)}</pre>
          {nextCursor ? <p className="hint">More edges available (next cursor present).</p> : null}
        </div>
      </article>

      <article className="card">
        <h3>Graph Canvas</h3>
        <p className="hint">Rendering capped at 500 nodes/edges for browser safety.</p>
        <div className="graph-canvas" ref={graphRef} />
      </article>
    </section>
  );
}
