import { FormEvent, useEffect, useMemo, useState } from "react";
import { FileSearch, FlaskConical, Sparkles } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EntityTypeInput } from "@/components/shared/EntityTypeInput";
import { EmptyState } from "@/components/shared/EmptyState";
import { InlineNotice } from "@/components/shared/InlineNotice";
import { JsonInspector } from "@/components/shared/JsonInspector";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { apiRequest, normalizeApiError } from "@/lib/api";
import type {
  ExplainResponse,
  FindNodesResponse,
  NodeModel,
  PqlResponse,
} from "@/lib/types";
import { QueryCodeEditor } from "@/features/query/QueryCodeEditor";

const pqlPresets = [
  {
    name: "People Over 30",
    query: "Person @filter(age >= 30) { uid name age }",
  },
  {
    name: "Projects In Flight",
    query: "Project @filter(status == 'active') { uid name status }",
  },
  {
    name: "High Priority Tasks",
    query: "Task @filter(priority >= 7) { uid task_code stage priority }",
  },
];

type QueryIssue = {
  kind: "info" | "ok" | "error";
  title: string;
  detail?: string;
};

const RESULTS_PAGE_SIZE = 20;
const PROPERTY_PREVIEW_LIMIT = 4;

function formatPreviewValue(value: unknown): string {
  if (value === null) {
    return "null";
  }

  if (Array.isArray(value)) {
    const preview = value.slice(0, 3).map((item) => formatPreviewValue(item)).join(", ");
    return value.length > 3 ? `[${preview}, ...]` : `[${preview}]`;
  }

  if (typeof value === "object") {
    return "{...}";
  }

  if (typeof value === "string") {
    return value.length > 42 ? `${value.slice(0, 39)}...` : value;
  }

  return String(value);
}

function previewProperties(
  properties: Record<string, unknown> | undefined,
  limit = PROPERTY_PREVIEW_LIMIT,
): string {
  if (!properties) {
    return "-";
  }

  const entries = Object.entries(properties);
  if (entries.length === 0) {
    return "-";
  }

  const preview = entries
    .slice(0, limit)
    .map(([key, value]) => `${key}: ${formatPreviewValue(value)}`)
    .join(", ");

  if (entries.length > limit) {
    return `${preview}, +${entries.length - limit} more`;
  }

  return preview;
}

export default function QueryView() {
  const { session, scope, schemaCatalog } = useConsoleContext();

  const [tab, setTab] = useState<"cel" | "pql">("cel");
  const [outputTab, setOutputTab] = useState<"results" | "explain">("results");
  const [entityType, setEntityType] = useState(schemaCatalog[0]?.name ?? "");
  const [celExpression, setCelExpression] = useState("age >= 30");
  const [pql, setPql] = useState("Person @filter(age >= 30) { uid name age }");
  const [snapshotMode, setSnapshotMode] = useState<"strict" | "best_effort">("strict");
  const [allowDegrade, setAllowDegrade] = useState(true);

  const [loading, setLoading] = useState(false);
  const [resultKind, setResultKind] = useState<"cel" | "pql" | null>(null);
  const [queryResult, setQueryResult] = useState<FindNodesResponse | PqlResponse | null>(null);
  const [explainResult, setExplainResult] = useState<ExplainResponse | null>(null);
  const [issue, setIssue] = useState<QueryIssue | null>(null);
  const [resultsPage, setResultsPage] = useState(1);

  useEffect(() => {
    if (!entityType && schemaCatalog.length > 0) {
      setEntityType(schemaCatalog[0].name);
    }
  }, [entityType, schemaCatalog]);

  const celRows = useMemo(() => {
    if (!queryResult || resultKind !== "cel") {
      return [] as NodeModel[];
    }

    return (queryResult as FindNodesResponse).items;
  }, [queryResult, resultKind]);

  const pqlRows = useMemo(() => {
    if (!queryResult || resultKind !== "pql") {
      return [] as PqlResponse["items"];
    }

    return (queryResult as PqlResponse).items;
  }, [queryResult, resultKind]);

  useEffect(() => {
    setResultsPage(1);
  }, [queryResult, resultKind]);

  const totalRows = resultKind === "cel" ? celRows.length : resultKind === "pql" ? pqlRows.length : 0;
  const pageCount = Math.max(1, Math.ceil(totalRows / RESULTS_PAGE_SIZE));
  const currentPage = Math.min(resultsPage, pageCount);
  const pageStart = totalRows === 0 ? 0 : (currentPage - 1) * RESULTS_PAGE_SIZE;
  const pageEnd = Math.min(pageStart + RESULTS_PAGE_SIZE, totalRows);
  const pagedCelRows = useMemo(() => celRows.slice(pageStart, pageEnd), [celRows, pageStart, pageEnd]);
  const pagedPqlRows = useMemo(() => pqlRows.slice(pageStart, pageEnd), [pqlRows, pageStart, pageEnd]);

  async function runFind(event: FormEvent) {
    event.preventDefault();

    if (!entityType.trim()) {
      setIssue({
        kind: "error",
        title: "Entity type is required",
        detail: "Select a schema entity or enter a custom entity type before running CEL.",
      });
      return;
    }

    if (!celExpression.trim()) {
      setIssue({
        kind: "error",
        title: "CEL expression is required",
      });
      return;
    }

    setLoading(true);
    setIssue({ kind: "info", title: "Running CEL query..." });

    try {
      const response = await apiRequest<FindNodesResponse>("/query/find", {
        method: "POST",
        session,
        scope,
        body: {
          entity_type: entityType,
          cel_expression: celExpression,
          limit: 100,
          consistency: "strong",
          snapshot_mode: snapshotMode,
          allow_degrade_to_best_effort: allowDegrade,
        },
      });
      setQueryResult(response);
      setResultKind("cel");
      setOutputTab("results");
      setIssue({
        kind: "ok",
        title: `CEL query completed with ${response.items.length} row(s).`,
        detail: response.degraded ? `Degraded: ${response.degraded_reason}` : "Consistency targets satisfied.",
      });
    } catch (error) {
      setIssue({
        kind: "error",
        title: "CEL query failed",
        detail: normalizeApiError(error),
      });
    } finally {
      setLoading(false);
    }
  }

  async function runPql(event: FormEvent) {
    event.preventDefault();

    if (!pql.trim()) {
      setIssue({
        kind: "error",
        title: "PQL query is required",
      });
      return;
    }

    setLoading(true);
    setIssue({ kind: "info", title: "Running PQL query..." });

    try {
      const response = await apiRequest<PqlResponse>("/query/pql", {
        method: "POST",
        session,
        scope,
        body: {
          pql,
          explain: false,
          snapshot_mode: snapshotMode,
          allow_degrade_to_best_effort: allowDegrade,
        },
      });
      setQueryResult(response);
      setResultKind("pql");
      setOutputTab("results");
      setIssue({
        kind: "ok",
        title: `PQL query completed with ${response.items.length} row(s).`,
        detail: response.degraded ? `Degraded: ${response.degraded_reason}` : "Consistency targets satisfied.",
      });
    } catch (error) {
      setIssue({
        kind: "error",
        title: "PQL query failed",
        detail: normalizeApiError(error),
      });
    } finally {
      setLoading(false);
    }
  }

  async function runExplain() {
    if (!entityType.trim()) {
      setIssue({
        kind: "error",
        title: "Entity type is required",
        detail: "Select a schema entity before generating explain output.",
      });
      return;
    }

    if (!celExpression.trim()) {
      setIssue({
        kind: "error",
        title: "CEL expression is required for explain",
      });
      return;
    }

    setLoading(true);
    setIssue({ kind: "info", title: "Generating explain plan..." });

    try {
      const response = await apiRequest<ExplainResponse>("/query/explain", {
        method: "POST",
        session,
        scope,
        body: {
          entity_type: entityType,
          cel_expression: celExpression,
        },
      });
      setExplainResult(response);
      setOutputTab("explain");
      setIssue({
        kind: "ok",
        title: "Explain plan generated",
        detail: `Estimated rows: ${response.estimated_rows}, cost: ${response.estimated_cost.toFixed(2)}`,
      });
    } catch (error) {
      setIssue({
        kind: "error",
        title: "Explain generation failed",
        detail: normalizeApiError(error),
      });
    } finally {
      setLoading(false);
    }
  }

  const degraded = queryResult?.degraded;

  return (
    <section className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Query Studio</CardTitle>
          <CardDescription>
            CEL and PQL editors with syntax highlighting, keyword autocomplete, and inline execution feedback.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Tabs value={tab} onValueChange={(next) => setTab(next as "cel" | "pql")}>
            <TabsList>
              <TabsTrigger value="cel">CEL</TabsTrigger>
              <TabsTrigger value="pql">PQL</TabsTrigger>
            </TabsList>

            <TabsContent value="cel" className="mt-4">
              <form className="space-y-3" onSubmit={runFind}>
                <EntityTypeInput
                  value={entityType}
                  onChange={(value) => setEntityType(value)}
                  schemaCatalog={schemaCatalog}
                />

                <div className="space-y-1">
                  <Label>CEL Expression</Label>
                  <QueryCodeEditor
                    mode="cel"
                    value={celExpression}
                    onChange={setCelExpression}
                    minHeight={130}
                    placeholder="age >= 30"
                  />
                </div>

                <details className="rounded-md border border-border bg-slate-50 p-3">
                  <summary className="cursor-pointer text-sm font-medium">Advanced Query Options</summary>
                  <div className="mt-3 grid gap-3 sm:grid-cols-2">
                    <label className="space-y-1">
                      <Label>Snapshot Mode</Label>
                      <Select
                        value={snapshotMode}
                        onValueChange={(value) => setSnapshotMode(value as "strict" | "best_effort")}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="strict">Strict</SelectItem>
                          <SelectItem value="best_effort">Best Effort</SelectItem>
                        </SelectContent>
                      </Select>
                    </label>
                    <label className="space-y-1">
                      <Label>Allow degrade to best effort</Label>
                      <Select value={allowDegrade ? "yes" : "no"} onValueChange={(value) => setAllowDegrade(value === "yes")}>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="yes">Yes</SelectItem>
                          <SelectItem value="no">No</SelectItem>
                        </SelectContent>
                      </Select>
                    </label>
                  </div>
                </details>

                <div className="flex flex-wrap gap-2">
                  <Button type="submit" disabled={loading}>
                    {loading ? "Running..." : "Run CEL Find"}
                  </Button>
                  <Button type="button" variant="secondary" disabled={loading} onClick={runExplain}>
                    <FileSearch className="h-4 w-4" /> Explain
                  </Button>
                </div>
              </form>
            </TabsContent>

            <TabsContent value="pql" className="mt-4">
              <form className="space-y-3" onSubmit={runPql}>
                <label className="space-y-1">
                  <Label>Preset</Label>
                  <Select
                    onValueChange={(value) => {
                      const preset = pqlPresets.find((item) => item.name === value);
                      if (preset) {
                        setPql(preset.query);
                      }
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder="Apply a PQL preset" />
                    </SelectTrigger>
                    <SelectContent>
                      {pqlPresets.map((preset) => (
                        <SelectItem key={preset.name} value={preset.name}>
                          {preset.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>

                <div className="space-y-1">
                  <Label>PQL</Label>
                  <QueryCodeEditor
                    mode="pql"
                    value={pql}
                    onChange={setPql}
                    minHeight={210}
                    placeholder="Person @filter(age >= 30) { uid name age }"
                  />
                </div>

                <div className="grid gap-3 sm:grid-cols-2">
                  <label className="space-y-1">
                    <Label>Snapshot Mode</Label>
                    <Select
                      value={snapshotMode}
                      onValueChange={(value) => setSnapshotMode(value as "strict" | "best_effort")}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="strict">Strict</SelectItem>
                        <SelectItem value="best_effort">Best Effort</SelectItem>
                      </SelectContent>
                    </Select>
                  </label>
                  <label className="space-y-1">
                    <Label>Allow degrade to best effort</Label>
                    <Select value={allowDegrade ? "yes" : "no"} onValueChange={(value) => setAllowDegrade(value === "yes")}>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="yes">Yes</SelectItem>
                        <SelectItem value="no">No</SelectItem>
                      </SelectContent>
                    </Select>
                  </label>
                </div>

                <Button type="submit" disabled={loading}>
                  {loading ? "Running..." : "Run PQL"}
                </Button>
              </form>
            </TabsContent>
          </Tabs>

          {issue ? <InlineNotice kind={issue.kind} title={issue.title} detail={issue.detail} className="mt-4" /> : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <SectionHeader
            title="Output Inspector"
            description="Results and explain output share one panel to avoid split-screen crowding."
            actions={
              <div className="flex gap-2">
                <StatusBadge state={queryResult ? "active" : "idle"}>
                  {queryResult ? `${queryResult.items.length} rows` : "No data"}
                </StatusBadge>
                <StatusBadge state={degraded ? "warning" : "ok"}>
                  {degraded ? "Degraded" : "Consistency OK"}
                </StatusBadge>
              </div>
            }
          />
        </CardHeader>
        <CardContent>
          <Tabs value={outputTab} onValueChange={(next) => setOutputTab(next as "results" | "explain")}>
            <TabsList>
              <TabsTrigger value="results">Results</TabsTrigger>
              <TabsTrigger value="explain">Explain</TabsTrigger>
            </TabsList>

            <TabsContent value="results" className="mt-3 space-y-4">
              {queryResult ? (
                <>
                  <div className="rounded-md border">
                    <div className="max-h-80 overflow-auto">
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
                            {pagedCelRows.map((row) => {
                              const propertyPreview = previewProperties(row.properties);
                              return (
                                <TableRow key={`${row.entity_type}:${row.id}`}>
                                  <TableCell>{row.entity_type}</TableCell>
                                  <TableCell className="font-mono text-xs">{row.id}</TableCell>
                                  <TableCell>{row.locality}</TableCell>
                                  <TableCell className="max-w-[460px] text-xs text-muted" title={propertyPreview}>
                                    {propertyPreview}
                                  </TableCell>
                                </TableRow>
                              );
                            })}
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
                            {pagedPqlRows.map((row, index) => {
                              const nodeProperties = previewProperties(row.node?.properties);
                              const edgeProperties = previewProperties(row.edge?.properties);
                              return (
                                <TableRow key={`${row.block_name}-${pageStart + index}`}>
                                  <TableCell>{row.block_name || "-"}</TableCell>
                                  <TableCell className="font-mono text-xs">
                                    {row.node ? `${row.node.entity_type}:${row.node.id}` : "-"}
                                  </TableCell>
                                  <TableCell className="max-w-[260px] text-xs text-muted" title={nodeProperties}>
                                    {nodeProperties}
                                  </TableCell>
                                  <TableCell className="font-mono text-xs">{row.edge?.label ?? "-"}</TableCell>
                                  <TableCell className="max-w-[260px] text-xs text-muted" title={edgeProperties}>
                                    {edgeProperties}
                                  </TableCell>
                                  <TableCell className="max-w-[280px] text-xs text-muted" title={row.explain || "-"}>
                                    {row.explain || "-"}
                                  </TableCell>
                                </TableRow>
                              );
                            })}
                          </TableBody>
                        </Table>
                      )}
                    </div>
                  </div>

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
                        onClick={() => setResultsPage((page) => Math.max(1, page - 1))}
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
                        onClick={() => setResultsPage((page) => Math.min(pageCount, page + 1))}
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
        </CardContent>
      </Card>
    </section>
  );
}
