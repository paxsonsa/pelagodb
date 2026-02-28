import { FormEvent, useCallback, useEffect, useState } from "react";
import { FileSearch } from "lucide-react";

import type { ConsoleContext } from "@/App";
import { EntityTypeInput } from "@/components/shared/EntityTypeInput";
import { InlineNotice } from "@/components/shared/InlineNotice";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { apiRequest, normalizeApiError } from "@/lib/api";
import type { ExplainResponse, FindNodesResponse, PqlResponse } from "@/lib/types";
import { QueryCodeEditor } from "./QueryCodeEditor";
import { pushQueryHistory } from "./query-history-store";
import type { QueryTab } from "./query-tabs-store";

type QueryIssue = {
  kind: "info" | "ok" | "error";
  title: string;
  detail?: string;
};

const pqlPresets = [
  { name: "People Over 30", query: "Person @filter(age >= 30) { uid name age }" },
  { name: "Projects In Flight", query: "Project @filter(status == 'active') { uid name status }" },
  { name: "High Priority Tasks", query: "Task @filter(priority >= 7) { uid task_code stage priority }" },
];

export function QueryTabContent({
  tab,
  ctx,
  onUpdateTab,
}: {
  tab: QueryTab;
  ctx: ConsoleContext;
  onUpdateTab: (patch: Partial<QueryTab>) => void;
}) {
  const { session, scope, schemaCatalog } = ctx;

  const [loading, setLoading] = useState(false);
  const [issue, setIssue] = useState<QueryIssue | null>(null);

  // Sync entity type from catalog on first load
  useEffect(() => {
    if (!tab.entityType && schemaCatalog.length > 0) {
      onUpdateTab({ entityType: schemaCatalog[0].name });
    }
  }, [tab.entityType, schemaCatalog, onUpdateTab]);

  const runFind = useCallback(async (event?: FormEvent) => {
    event?.preventDefault();

    if (!tab.entityType.trim()) {
      setIssue({ kind: "error", title: "Entity type is required" });
      return;
    }
    if (!tab.expression.trim()) {
      setIssue({ kind: "error", title: "CEL expression is required" });
      return;
    }

    setLoading(true);
    setIssue({ kind: "info", title: "Running CEL query..." });

    const start = performance.now();
    try {
      const response = await apiRequest<FindNodesResponse>("/query/find", {
        method: "POST",
        session,
        scope,
        body: {
          entity_type: tab.entityType,
          cel_expression: tab.expression,
          limit: 100,
          consistency: "strong",
          snapshot_mode: tab.snapshotMode,
          allow_degrade_to_best_effort: tab.allowDegrade,
        },
      });
      const elapsed = Math.round(performance.now() - start);
      onUpdateTab({ result: response, resultKind: "cel", executionTimeMs: elapsed });
      pushQueryHistory({
        timestamp: new Date().toISOString(),
        mode: "cel",
        expression: tab.expression,
        entityType: tab.entityType,
        rowCount: response.items.length,
        executionTimeMs: elapsed,
      });
      setIssue({
        kind: "ok",
        title: `CEL query completed — ${response.items.length} row(s) in ${elapsed}ms`,
        detail: response.degraded ? `Degraded: ${response.degraded_reason}` : undefined,
      });
    } catch (error) {
      setIssue({ kind: "error", title: "CEL query failed", detail: normalizeApiError(error) });
    } finally {
      setLoading(false);
    }
  }, [tab, session, scope, onUpdateTab]);

  const runPql = useCallback(async (event?: FormEvent) => {
    event?.preventDefault();

    if (!tab.expression.trim()) {
      setIssue({ kind: "error", title: "PQL query is required" });
      return;
    }

    setLoading(true);
    setIssue({ kind: "info", title: "Running PQL query..." });

    const start = performance.now();
    try {
      const response = await apiRequest<PqlResponse>("/query/pql", {
        method: "POST",
        session,
        scope,
        body: {
          pql: tab.expression,
          explain: false,
          snapshot_mode: tab.snapshotMode,
          allow_degrade_to_best_effort: tab.allowDegrade,
        },
      });
      const elapsed = Math.round(performance.now() - start);
      onUpdateTab({ result: response, resultKind: "pql", executionTimeMs: elapsed });
      pushQueryHistory({
        timestamp: new Date().toISOString(),
        mode: "pql",
        expression: tab.expression,
        entityType: tab.entityType,
        rowCount: response.items.length,
        executionTimeMs: elapsed,
      });
      setIssue({
        kind: "ok",
        title: `PQL query completed — ${response.items.length} row(s) in ${elapsed}ms`,
        detail: response.degraded ? `Degraded: ${response.degraded_reason}` : undefined,
      });
    } catch (error) {
      setIssue({ kind: "error", title: "PQL query failed", detail: normalizeApiError(error) });
    } finally {
      setLoading(false);
    }
  }, [tab, session, scope, onUpdateTab]);

  const runExplain = useCallback(async () => {
    if (!tab.entityType.trim() || !tab.expression.trim()) {
      setIssue({ kind: "error", title: "Entity type and expression required for explain" });
      return;
    }

    setLoading(true);
    setIssue({ kind: "info", title: "Generating explain plan..." });

    try {
      const response = await apiRequest<ExplainResponse>("/query/explain", {
        method: "POST",
        session,
        scope,
        body: { entity_type: tab.entityType, cel_expression: tab.expression },
      });
      onUpdateTab({ explainResult: response });
      setIssue({
        kind: "ok",
        title: "Explain plan generated",
        detail: `Estimated rows: ${response.estimated_rows}, cost: ${response.estimated_cost.toFixed(2)}`,
      });
    } catch (error) {
      setIssue({ kind: "error", title: "Explain failed", detail: normalizeApiError(error) });
    } finally {
      setLoading(false);
    }
  }, [tab, session, scope, onUpdateTab]);

  const onExecute = useCallback(() => {
    if (tab.mode === "cel") void runFind();
    else void runPql();
  }, [tab.mode, runFind, runPql]);

  return (
    <div className="space-y-3">
      <Tabs value={tab.mode} onValueChange={(next) => onUpdateTab({ mode: next as "cel" | "pql" })}>
        <TabsList>
          <TabsTrigger value="cel">CEL</TabsTrigger>
          <TabsTrigger value="pql">PQL</TabsTrigger>
        </TabsList>

        <TabsContent value="cel" className="mt-4">
          <form className="space-y-3" onSubmit={(e) => void runFind(e)}>
            <EntityTypeInput
              value={tab.entityType}
              onChange={(value) => onUpdateTab({ entityType: value })}
              schemaCatalog={schemaCatalog}
            />
            <div className="space-y-1">
              <Label>CEL Expression</Label>
              <QueryCodeEditor
                mode="cel"
                value={tab.expression}
                onChange={(value) => onUpdateTab({ expression: value })}
                onExecute={onExecute}
                minHeight={180}
                placeholder="age >= 30"
              />
            </div>

            <details className="rounded-md border border-border bg-surface-subtle p-3">
              <summary className="cursor-pointer text-sm font-medium">Advanced Query Options</summary>
              <div className="mt-3 grid gap-3 sm:grid-cols-2">
                <label className="space-y-1">
                  <Label>Snapshot Mode</Label>
                  <Select
                    value={tab.snapshotMode}
                    onValueChange={(value) => onUpdateTab({ snapshotMode: value as "strict" | "best_effort" })}
                  >
                    <SelectTrigger><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="strict">Strict</SelectItem>
                      <SelectItem value="best_effort">Best Effort</SelectItem>
                    </SelectContent>
                  </Select>
                </label>
                <label className="space-y-1">
                  <Label>Allow degrade to best effort</Label>
                  <Select
                    value={tab.allowDegrade ? "yes" : "no"}
                    onValueChange={(value) => onUpdateTab({ allowDegrade: value === "yes" })}
                  >
                    <SelectTrigger><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="yes">Yes</SelectItem>
                      <SelectItem value="no">No</SelectItem>
                    </SelectContent>
                  </Select>
                </label>
              </div>
            </details>

            <div className="flex flex-wrap items-center gap-2">
              <Button type="submit" disabled={loading}>
                {loading ? "Running..." : "Run CEL Find"}
              </Button>
              <Button type="button" variant="secondary" disabled={loading} onClick={() => void runExplain()}>
                <FileSearch className="h-4 w-4" /> Explain
              </Button>
              <span className="text-[11px] text-muted">⌘+Enter to run</span>
            </div>
          </form>
        </TabsContent>

        <TabsContent value="pql" className="mt-4">
          <form className="space-y-3" onSubmit={(e) => void runPql(e)}>
            <label className="space-y-1">
              <Label>Preset</Label>
              <Select
                onValueChange={(value) => {
                  const preset = pqlPresets.find((item) => item.name === value);
                  if (preset) onUpdateTab({ expression: preset.query });
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
                value={tab.expression}
                onChange={(value) => onUpdateTab({ expression: value })}
                onExecute={onExecute}
                minHeight={300}
                placeholder="Person @filter(age >= 30) { uid name age }"
              />
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <label className="space-y-1">
                <Label>Snapshot Mode</Label>
                <Select
                  value={tab.snapshotMode}
                  onValueChange={(value) => onUpdateTab({ snapshotMode: value as "strict" | "best_effort" })}
                >
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="strict">Strict</SelectItem>
                    <SelectItem value="best_effort">Best Effort</SelectItem>
                  </SelectContent>
                </Select>
              </label>
              <label className="space-y-1">
                <Label>Allow degrade to best effort</Label>
                <Select
                  value={tab.allowDegrade ? "yes" : "no"}
                  onValueChange={(value) => onUpdateTab({ allowDegrade: value === "yes" })}
                >
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="yes">Yes</SelectItem>
                    <SelectItem value="no">No</SelectItem>
                  </SelectContent>
                </Select>
              </label>
            </div>

            <div className="flex items-center gap-2">
              <Button type="submit" disabled={loading}>
                {loading ? "Running..." : "Run PQL"}
              </Button>
              <span className="text-[11px] text-muted">⌘+Enter to run</span>
            </div>
          </form>
        </TabsContent>
      </Tabs>

      {issue ? <InlineNotice kind={issue.kind} title={issue.title} detail={issue.detail} className="mt-3" /> : null}
    </div>
  );
}
