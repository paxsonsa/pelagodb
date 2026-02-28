import { useMemo, useState } from "react";
import { Copy, Download, FileSearch, FlaskConical, Sparkles } from "lucide-react";

import { EmptyState } from "@/components/shared/EmptyState";
import { JsonInspector } from "@/components/shared/JsonInspector";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { copyJson, downloadCsv, downloadJson } from "@/lib/export-utils";
import type { ExplainResponse, FindNodesResponse, NodeModel, PqlResponse } from "@/lib/types";
import { RowDetailSheet } from "./RowDetailSheet";

const RESULTS_PAGE_SIZE = 20;
const PROPERTY_PREVIEW_LIMIT = 4;

function formatPreviewValue(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) {
    const preview = value.slice(0, 3).map(formatPreviewValue).join(", ");
    return value.length > 3 ? `[${preview}, ...]` : `[${preview}]`;
  }
  if (typeof value === "object") return "{...}";
  if (typeof value === "string") return value.length > 42 ? `${value.slice(0, 39)}...` : value;
  return String(value);
}

function previewProperties(properties: Record<string, unknown> | undefined, limit = PROPERTY_PREVIEW_LIMIT): string {
  if (!properties) return "-";
  const entries = Object.entries(properties);
  if (entries.length === 0) return "-";
  const preview = entries
    .slice(0, limit)
    .map(([key, value]) => `${key}: ${formatPreviewValue(value)}`)
    .join(", ");
  return entries.length > limit ? `${preview}, +${entries.length - limit} more` : preview;
}

export function QueryResultsPanel({
  queryResult,
  explainResult,
  resultKind,
  executionTimeMs,
}: {
  queryResult: FindNodesResponse | PqlResponse | null;
  explainResult: ExplainResponse | null;
  resultKind: "cel" | "pql" | null;
  executionTimeMs: number | null;
}) {
  const [outputTab, setOutputTab] = useState<"results" | "explain">("results");
  const [resultsPage, setResultsPage] = useState(1);
  const [detailNode, setDetailNode] = useState<NodeModel | null>(null);

  const celRows = useMemo(() => {
    if (!queryResult || resultKind !== "cel") return [] as NodeModel[];
    return (queryResult as FindNodesResponse).items;
  }, [queryResult, resultKind]);

  const pqlRows = useMemo(() => {
    if (!queryResult || resultKind !== "pql") return [] as PqlResponse["items"];
    return (queryResult as PqlResponse).items;
  }, [queryResult, resultKind]);

  // Reset page on new results
  useMemo(() => setResultsPage(1), [queryResult, resultKind]);

  const totalRows = resultKind === "cel" ? celRows.length : resultKind === "pql" ? pqlRows.length : 0;
  const pageCount = Math.max(1, Math.ceil(totalRows / RESULTS_PAGE_SIZE));
  const currentPage = Math.min(resultsPage, pageCount);
  const pageStart = totalRows === 0 ? 0 : (currentPage - 1) * RESULTS_PAGE_SIZE;
  const pageEnd = Math.min(pageStart + RESULTS_PAGE_SIZE, totalRows);
  const pagedCelRows = useMemo(() => celRows.slice(pageStart, pageEnd), [celRows, pageStart, pageEnd]);
  const pagedPqlRows = useMemo(() => pqlRows.slice(pageStart, pageEnd), [pqlRows, pageStart, pageEnd]);

  const degraded = queryResult?.degraded;

  const exportRows = resultKind === "cel" ? celRows : pqlRows;

  return (
    <>
      <RowDetailSheet node={detailNode} open={!!detailNode} onOpenChange={(open) => { if (!open) setDetailNode(null); }} />

      <Tabs value={outputTab} onValueChange={(next) => setOutputTab(next as "results" | "explain")}>
        <div className="flex items-center justify-between">
          <TabsList>
            <TabsTrigger value="results">Results</TabsTrigger>
            <TabsTrigger value="explain">Explain</TabsTrigger>
          </TabsList>

          <div className="flex items-center gap-2">
            {executionTimeMs !== null && totalRows > 0 && (
              <Badge variant="default">{totalRows} rows in {Math.round(executionTimeMs)}ms</Badge>
            )}
            <StatusBadge state={queryResult ? "active" : "idle"}>
              {queryResult ? `${totalRows} rows` : "No data"}
            </StatusBadge>
            <StatusBadge state={degraded ? "warning" : "ok"}>
              {degraded ? "Degraded" : "Consistency OK"}
            </StatusBadge>
          </div>
        </div>

        <TabsContent value="results" className="mt-3 space-y-3">
          {queryResult ? (
            <>
              {/* Export toolbar */}
              <div className="flex gap-1">
                <Button type="button" variant="ghost" size="sm" onClick={() => void copyJson(exportRows)}>
                  <Copy className="mr-1.5 h-3.5 w-3.5" /> Copy JSON
                </Button>
                <Button type="button" variant="ghost" size="sm" onClick={() => downloadJson(exportRows)}>
                  <Download className="mr-1.5 h-3.5 w-3.5" /> JSON
                </Button>
                {resultKind === "cel" && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => downloadCsv(celRows as unknown as Record<string, unknown>[])}
                  >
                    <Download className="mr-1.5 h-3.5 w-3.5" /> CSV
                  </Button>
                )}
              </div>

              <div className="rounded-md border">
                <div className="overflow-auto" style={{ maxHeight: "calc(100% - 40px)" }}>
                  {resultKind === "cel" ? (
                    <Table className="min-w-[920px]">
                      <TableHeader>
                        <TableRow>
                          <TableHead>Entity</TableHead>
                          <TableHead>ID</TableHead>
                          <TableHead>Locality</TableHead>
                          <TableHead>Properties</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {pagedCelRows.map((row) => (
                          <TableRow
                            key={`${row.entity_type}:${row.id}`}
                            className="cursor-pointer hover:bg-foreground/5"
                            onClick={() => setDetailNode(row)}
                          >
                            <TableCell>{row.entity_type}</TableCell>
                            <TableCell className="font-mono text-xs">{row.id}</TableCell>
                            <TableCell>{row.locality}</TableCell>
                            <TableCell className="max-w-[460px] text-xs text-muted" title={previewProperties(row.properties)}>
                              {previewProperties(row.properties)}
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  ) : (
                    <Table className="min-w-[1080px]">
                      <TableHeader>
                        <TableRow>
                          <TableHead>Block</TableHead>
                          <TableHead>Node</TableHead>
                          <TableHead>Node Properties</TableHead>
                          <TableHead>Edge</TableHead>
                          <TableHead>Edge Properties</TableHead>
                          <TableHead>Explain</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {pagedPqlRows.map((row, index) => (
                          <TableRow
                            key={`${row.block_name}-${pageStart + index}`}
                            className="cursor-pointer hover:bg-foreground/5"
                            onClick={() => row.node && setDetailNode(row.node)}
                          >
                            <TableCell>{row.block_name || "-"}</TableCell>
                            <TableCell className="font-mono text-xs">
                              {row.node ? `${row.node.entity_type}:${row.node.id}` : "-"}
                            </TableCell>
                            <TableCell className="max-w-[260px] text-xs text-muted" title={previewProperties(row.node?.properties)}>
                              {previewProperties(row.node?.properties)}
                            </TableCell>
                            <TableCell className="font-mono text-xs">{row.edge?.label ?? "-"}</TableCell>
                            <TableCell className="max-w-[260px] text-xs text-muted" title={previewProperties(row.edge?.properties)}>
                              {previewProperties(row.edge?.properties)}
                            </TableCell>
                            <TableCell className="max-w-[280px] text-xs text-muted" title={row.explain || "-"}>
                              {row.explain || "-"}
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  )}
                </div>
              </div>

              {/* Pagination */}
              <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted">
                <span>
                  Showing {totalRows === 0 ? 0 : pageStart + 1}-{pageEnd} of {totalRows}
                </span>
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={currentPage <= 1}
                    onClick={() => setResultsPage((p) => Math.max(1, p - 1))}
                  >
                    Previous
                  </Button>
                  <span>
                    Page {currentPage} / {pageCount}
                  </span>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={currentPage >= pageCount}
                    onClick={() => setResultsPage((p) => Math.min(pageCount, p + 1))}
                  >
                    Next
                  </Button>
                </div>
              </div>

              <JsonInspector value={queryResult} maxHeight={440} fontSizeClass="text-[13px]" />
            </>
          ) : (
            <EmptyState
              title="No Query Results"
              description="Run CEL or PQL to inspect result rows and metadata."
              icon={<FlaskConical className="h-5 w-5" />}
            />
          )}
        </TabsContent>

        <TabsContent value="explain" className="mt-3 space-y-4">
          {explainResult ? (
            <>
              <div className="flex flex-wrap gap-2">
                <StatusBadge state="active">Cost {explainResult.estimated_cost.toFixed(2)}</StatusBadge>
                <StatusBadge state="ok">Rows {explainResult.estimated_rows}</StatusBadge>
              </div>
              <JsonInspector value={explainResult} maxHeight={520} fontSizeClass="text-[13px]" />
            </>
          ) : (
            <EmptyState
              title="Explain Not Generated"
              description="Run Explain from CEL mode to inspect planner output."
              icon={<Sparkles className="h-5 w-5" />}
            />
          )}
        </TabsContent>
      </Tabs>
    </>
  );
}
