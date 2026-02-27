import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { GitBranch, RefreshCw } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EntityTypeInput } from "@/components/shared/EntityTypeInput";
import { EmptyState } from "@/components/shared/EmptyState";
import { JsonInspector } from "@/components/shared/JsonInspector";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { apiRequest, normalizeApiError } from "@/lib/api";
import type { EdgeModel, GraphEdgesResponse, GraphNodeResponse, NodeModel } from "@/lib/types";

type GraphDirection = "outgoing" | "incoming" | "both";

export default function ExplorerView() {
  const { session, scope, notify, schemaCatalog } = useConsoleContext();
  const [entityType, setEntityType] = useState("Person");
  const [nodeId, setNodeId] = useState("1_0");
  const [direction, setDirection] = useState<GraphDirection>("outgoing");

  const [node, setNode] = useState<NodeModel | null>(null);
  const [edges, setEdges] = useState<EdgeModel[]>([]);
  const [nextCursor, setNextCursor] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);

  const graphRef = useRef<HTMLDivElement | null>(null);

  async function fetchNodeAndEdges(event?: FormEvent) {
    event?.preventDefault();
    setLoading(true);

    try {
      const [nodeResponse, edgeResponse] = await Promise.all([
        apiRequest<GraphNodeResponse>(
          `/graph/node/${encodeURIComponent(entityType)}/${encodeURIComponent(nodeId)}?consistency=strong`,
          {
            session,
            scope,
          },
        ),
        apiRequest<GraphEdgesResponse>(
          `/graph/edges?entity_type=${encodeURIComponent(entityType)}&node_id=${encodeURIComponent(nodeId)}&direction=${direction}&limit=120`,
          {
            session,
            scope,
          },
        ),
      ]);

      setNode(nodeResponse.node ?? null);
      setEdges(edgeResponse.items ?? []);
      setNextCursor(edgeResponse.next_cursor ?? "");
      notify(`Loaded ${(edgeResponse.items ?? []).length} edge(s)`);
    } catch (error) {
      notify(normalizeApiError(error), "error");
      setNode(null);
      setEdges([]);
      setNextCursor("");
    } finally {
      setLoading(false);
    }
  }

  async function loadMoreEdges() {
    if (!nextCursor) {
      return;
    }

    setLoadingMore(true);

    try {
      const edgeResponse = await apiRequest<GraphEdgesResponse>(
        `/graph/edges?entity_type=${encodeURIComponent(entityType)}&node_id=${encodeURIComponent(nodeId)}&direction=${direction}&limit=120&cursor=${encodeURIComponent(nextCursor)}`,
        {
          session,
          scope,
        },
      );
      setEdges((prev) => [...prev, ...(edgeResponse.items ?? [])]);
      setNextCursor(edgeResponse.next_cursor ?? "");
    } catch (error) {
      notify(normalizeApiError(error), "error");
    } finally {
      setLoadingMore(false);
    }
  }

  const graphElements = useMemo(() => {
    if (!node) {
      return [];
    }

    const elements: Array<{ data: Record<string, string | boolean> }> = [];
    const rootId = `${node.entity_type}:${node.id}`;

    elements.push({
      data: {
        id: rootId,
        label: rootId,
        root: true,
      },
    });

    const seen = new Set<string>([rootId]);

    for (const edge of edges.slice(0, 400)) {
      if (!edge.source || !edge.target) {
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
          id: edge.edge_id || `${sourceId}:${edge.label}:${targetId}`,
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

    let cancelled = false;
    let instance: { destroy: () => void } | null = null;

    void (async () => {
      const cytoscapeModule = await import("cytoscape");
      if (cancelled || !graphRef.current) {
        return;
      }

      instance = cytoscapeModule.default({
        container: graphRef.current,
        elements: graphElements,
        style: [
          {
            selector: "node",
            style: {
              "background-color": "#4de2c3",
              color: "#03191f",
              "font-size": 10,
              "text-wrap": "wrap",
              "text-max-width": "110px",
              label: "data(label)",
              "border-width": 2,
              "border-color": "#0d5165",
              width: 40,
              height: 40,
            },
          },
          {
            selector: "node[root]",
            style: {
              "background-color": "#72a3ff",
              "border-color": "#2b4f95",
              width: 48,
              height: 48,
            },
          },
          {
            selector: "edge",
            style: {
              width: 2,
              "line-color": "#5878b4",
              "target-arrow-color": "#5878b4",
              "target-arrow-shape": "triangle",
              "curve-style": "bezier",
              label: "data(label)",
              color: "#cddcff",
              "font-size": 9,
            },
          },
        ],
        layout: {
          name: "cose",
          animate: false,
          nodeRepulsion: 9000,
        },
      });
    })();

    return () => {
      cancelled = true;
      instance?.destroy();
    };
  }, [graphElements]);

  return (
    <section className="space-y-4">
      <div className="grid gap-4 xl:grid-cols-[380px_minmax(0,1fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Node Exploration Flow</CardTitle>
            <CardDescription>
              Select an entity from schema catalog, provide node id, then inspect adjacency graph.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="space-y-3" onSubmit={fetchNodeAndEdges}>
              <EntityTypeInput
                value={entityType}
                onChange={(value) => setEntityType(value)}
                schemaCatalog={schemaCatalog}
              />
              <label className="space-y-1">
                <Label>Node ID</Label>
                <Input value={nodeId} onChange={(event) => setNodeId(event.target.value)} />
              </label>
              <label className="space-y-1">
                <Label>Direction</Label>
                <Select
                  value={direction}
                  onValueChange={(value) => setDirection(value as GraphDirection)}
                >
                  <SelectTrigger>
                    <SelectValue placeholder="Direction" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="outgoing">Outgoing</SelectItem>
                    <SelectItem value="incoming">Incoming</SelectItem>
                    <SelectItem value="both">Both</SelectItem>
                  </SelectContent>
                </Select>
              </label>

              <Button type="submit" className="w-full" disabled={loading}>
                {loading ? "Loading Graph..." : "Load Graph"}
              </Button>
            </form>

            <div className="mt-4 flex flex-wrap gap-2">
              <StatusBadge state={node ? "active" : "idle"}>{node ? "Node Loaded" : "No Node"}</StatusBadge>
              <StatusBadge state={nextCursor ? "warning" : "ok"}>
                {nextCursor ? "More Edges Available" : "Edge Window Complete"}
              </StatusBadge>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <SectionHeader
              title="Graph Canvas"
              description="Cytoscape loads lazily and renders up to 400 relationships for safety."
              actions={
                <Button type="button" variant="secondary" size="sm" onClick={fetchNodeAndEdges}>
                  <RefreshCw className="h-4 w-4" /> Refresh
                </Button>
              }
            />
          </CardHeader>
          <CardContent>
            {node ? (
              <div
                ref={graphRef}
                className="h-[520px] rounded-md border border-border bg-slate-50"
                aria-label="Graph canvas"
              />
            ) : (
              <EmptyState
                title="Graph Canvas Idle"
                description="Load a node to render topology and connected edges."
                icon={<GitBranch className="h-5 w-5" />}
              />
            )}
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Node Inspector</CardTitle>
          </CardHeader>
          <CardContent>
            {node ? (
              <JsonInspector value={node} maxHeight={340} />
            ) : (
              <EmptyState
                title="No Node Payload"
                description="Node metadata appears here after a graph request."
              />
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <SectionHeader
              title={`Edge Preview (${edges.length})`}
              description="Cursor-aware edge listing with lightweight relationship table."
              actions={
                nextCursor ? (
                  <Button type="button" variant="secondary" size="sm" onClick={loadMoreEdges} disabled={loadingMore}>
                    {loadingMore ? "Loading..." : "Load More Edges"}
                  </Button>
                ) : null
              }
            />
          </CardHeader>
          <CardContent>
            {edges.length > 0 ? (
              <>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Label</TableHead>
                      <TableHead>Source</TableHead>
                      <TableHead>Target</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {edges.slice(0, 12).map((edge) => (
                      <TableRow key={edge.edge_id}>
                        <TableCell>{edge.label}</TableCell>
                        <TableCell className="font-mono text-xs text-muted">
                          {edge.source ? `${edge.source.entity_type}:${edge.source.node_id}` : "-"}
                        </TableCell>
                        <TableCell className="font-mono text-xs text-muted">
                          {edge.target ? `${edge.target.entity_type}:${edge.target.node_id}` : "-"}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
                <JsonInspector value={edges.slice(0, 25)} maxHeight={280} className="mt-3" />
              </>
            ) : (
              <EmptyState
                title="No Edge Rows"
                description="Edge rows appear after loading a node with matching relationships."
              />
            )}
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
