import { FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GitBranch, RefreshCw } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EntityTypeInput } from "@/components/shared/EntityTypeInput";
import { EmptyState } from "@/components/shared/EmptyState";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { apiRequest, normalizeApiError } from "@/lib/api";
import { entityColor, entityColorBorder } from "@/lib/entity-colors";
import type { EdgeModel, GraphEdgesResponse, GraphNodeResponse, NodeModel } from "@/lib/types";
import { GraphControls } from "./GraphControls";
import { NodeDetailPanel } from "./NodeDetailPanel";

type GraphDirection = "outgoing" | "incoming" | "both";

const EDGE_PAGE_SIZE = 20;

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
  const [selectedNode, setSelectedNode] = useState<NodeModel | null>(null);
  const [showLabels, setShowLabels] = useState(true);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [edgeFilter, setEdgeFilter] = useState("");
  const [edgePage, setEdgePage] = useState(1);

  const graphRef = useRef<HTMLDivElement | null>(null);
  const cyRef = useRef<any>(null);

  async function fetchNodeAndEdges(event?: FormEvent) {
    event?.preventDefault();
    setLoading(true);

    try {
      const [nodeResponse, edgeResponse] = await Promise.all([
        apiRequest<GraphNodeResponse>(
          `/graph/node/${encodeURIComponent(entityType)}/${encodeURIComponent(nodeId)}?consistency=strong`,
          { session, scope },
        ),
        apiRequest<GraphEdgesResponse>(
          `/graph/edges?entity_type=${encodeURIComponent(entityType)}&node_id=${encodeURIComponent(nodeId)}&direction=${direction}&limit=120`,
          { session, scope },
        ),
      ]);

      setNode(nodeResponse.node ?? null);
      setSelectedNode(nodeResponse.node ?? null);
      setEdges(edgeResponse.items ?? []);
      setNextCursor(edgeResponse.next_cursor ?? "");
      setEdgePage(1);
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
    if (!nextCursor) return;
    setLoadingMore(true);

    try {
      const edgeResponse = await apiRequest<GraphEdgesResponse>(
        `/graph/edges?entity_type=${encodeURIComponent(entityType)}&node_id=${encodeURIComponent(nodeId)}&direction=${direction}&limit=120&cursor=${encodeURIComponent(nextCursor)}`,
        { session, scope },
      );
      setEdges((prev) => [...prev, ...(edgeResponse.items ?? [])]);
      setNextCursor(edgeResponse.next_cursor ?? "");
    } catch (error) {
      notify(normalizeApiError(error), "error");
    } finally {
      setLoadingMore(false);
    }
  }

  // Collect unique entity types for legend
  const entityTypes = useMemo(() => {
    const types = new Set<string>();
    if (node) types.add(node.entity_type);
    for (const edge of edges) {
      if (edge.source) types.add(edge.source.entity_type);
      if (edge.target) types.add(edge.target.entity_type);
    }
    return Array.from(types);
  }, [node, edges]);

  const graphElements = useMemo(() => {
    if (!node) return [];

    const elements: Array<{ data: Record<string, string | boolean> }> = [];
    const rootId = `${node.entity_type}:${node.id}`;

    elements.push({
      data: { id: rootId, label: rootId, root: true, entityType: node.entity_type },
    });

    const seen = new Set<string>([rootId]);

    for (const edge of edges.slice(0, 400)) {
      if (!edge.source || !edge.target) continue;

      const sourceId = `${edge.source.entity_type}:${edge.source.node_id}`;
      const targetId = `${edge.target.entity_type}:${edge.target.node_id}`;

      if (!seen.has(sourceId)) {
        seen.add(sourceId);
        elements.push({
          data: { id: sourceId, label: sourceId, root: sourceId === rootId, entityType: edge.source.entity_type },
        });
      }

      if (!seen.has(targetId)) {
        seen.add(targetId);
        elements.push({
          data: { id: targetId, label: targetId, root: targetId === rootId, entityType: edge.target.entity_type },
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
    if (!graphRef.current || graphElements.length === 0) return;

    let cancelled = false;
    let instance: any = null;

    void (async () => {
      const cytoscapeModule = await import("cytoscape");
      if (cancelled || !graphRef.current) return;

      instance = cytoscapeModule.default({
        container: graphRef.current,
        elements: graphElements,
        style: [
          {
            selector: "node",
            style: {
              "background-color": (ele: any) => entityColor(ele.data("entityType") || "default"),
              color: "var(--foreground)",
              "font-size": 10,
              "text-wrap": "wrap" as const,
              "text-max-width": "110px",
              label: showLabels ? "data(label)" : "",
              "border-width": 2,
              "border-color": (ele: any) => entityColorBorder(ele.data("entityType") || "default"),
              width: 40,
              height: 40,
            },
          },
          {
            selector: "node[?root]",
            style: {
              "border-width": 3,
              width: 48,
              height: 48,
            },
          },
          {
            selector: "edge",
            style: {
              width: 2,
              "line-color": "var(--border)",
              "target-arrow-color": "var(--muted)",
              "target-arrow-shape": "triangle" as const,
              "curve-style": "bezier" as const,
              label: showLabels ? "data(label)" : "",
              color: "var(--muted)",
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

      cyRef.current = instance;

      // Click on node → select for inspector
      instance.on("tap", "node", (evt: any) => {
        const nodeData = evt.target.data();
        if (nodeData.id) {
          const parts = nodeData.id.split(":");
          if (parts.length >= 2) {
            setSelectedNode({
              entity_type: parts[0],
              id: parts.slice(1).join(":"),
              properties: {},
              locality: "",
            });
          }
        }
      });
    })();

    return () => {
      cancelled = true;
      instance?.destroy();
      cyRef.current = null;
    };
  }, [graphElements, showLabels]);

  const toggleFullscreen = useCallback(() => {
    const el = graphRef.current?.parentElement;
    if (!el) return;
    if (!document.fullscreenElement) {
      el.requestFullscreen().then(() => setIsFullscreen(true)).catch(() => {});
    } else {
      document.exitFullscreen().then(() => setIsFullscreen(false)).catch(() => {});
    }
  }, []);

  // Edge filtering + pagination
  const filteredEdges = useMemo(() => {
    if (!edgeFilter.trim()) return edges;
    const q = edgeFilter.toLowerCase();
    return edges.filter((e) => e.label.toLowerCase().includes(q));
  }, [edges, edgeFilter]);

  const edgePageCount = Math.max(1, Math.ceil(filteredEdges.length / EDGE_PAGE_SIZE));
  const edgePageStart = (Math.min(edgePage, edgePageCount) - 1) * EDGE_PAGE_SIZE;
  const pagedEdges = filteredEdges.slice(edgePageStart, edgePageStart + EDGE_PAGE_SIZE);

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
                <Select value={direction} onValueChange={(value) => setDirection(value as GraphDirection)}>
                  <SelectTrigger><SelectValue placeholder="Direction" /></SelectTrigger>
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
              description="Click nodes to inspect. Entity types are color-coded."
              actions={
                <Button type="button" variant="secondary" size="sm" onClick={fetchNodeAndEdges}>
                  <RefreshCw className="h-4 w-4" /> Refresh
                </Button>
              }
            />
          </CardHeader>
          <CardContent>
            {node ? (
              <div className="relative">
                <div
                  ref={graphRef}
                  className="h-[520px] rounded-md border border-border bg-surface-subtle"
                  aria-label="Graph canvas"
                />
                <GraphControls
                  cy={cyRef.current}
                  rootId={`${node.entity_type}:${node.id}`}
                  showLabels={showLabels}
                  onToggleLabels={() => setShowLabels(!showLabels)}
                  onFullscreen={toggleFullscreen}
                  isFullscreen={isFullscreen}
                />
                {/* Entity legend */}
                {entityTypes.length > 0 && (
                  <div className="absolute bottom-3 left-3 flex flex-wrap gap-1.5 rounded-md border border-border bg-panel/90 px-2 py-1.5 backdrop-blur-sm">
                    {entityTypes.map((type) => (
                      <div key={type} className="flex items-center gap-1 text-[10px]">
                        <div
                          className="h-2.5 w-2.5 rounded-full"
                          style={{ backgroundColor: entityColor(type) }}
                        />
                        <span className="text-muted">{type}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
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
            <NodeDetailPanel node={selectedNode} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <SectionHeader
              title={`Edge Preview (${filteredEdges.length})`}
              description="Filterable, paginated edge listing."
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
              <div className="space-y-3">
                <Input
                  value={edgeFilter}
                  onChange={(e) => { setEdgeFilter(e.target.value); setEdgePage(1); }}
                  placeholder="Filter by edge label..."
                  className="max-w-xs"
                />
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Label</TableHead>
                      <TableHead>Source</TableHead>
                      <TableHead>Target</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {pagedEdges.map((edge) => (
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

                {/* Edge pagination */}
                <div className="flex items-center justify-between text-xs text-muted">
                  <span>
                    {edgePageStart + 1}-{Math.min(edgePageStart + EDGE_PAGE_SIZE, filteredEdges.length)} of {filteredEdges.length}
                  </span>
                  <div className="flex gap-2">
                    <Button type="button" size="sm" variant="outline" disabled={edgePage <= 1} onClick={() => setEdgePage((p) => p - 1)}>
                      Prev
                    </Button>
                    <Button type="button" size="sm" variant="outline" disabled={edgePage >= edgePageCount} onClick={() => setEdgePage((p) => p + 1)}>
                      Next
                    </Button>
                  </div>
                </div>
              </div>
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
