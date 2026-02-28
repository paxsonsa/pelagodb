import { useCallback, useRef, useState } from "react";

import { useConsoleContext } from "@/App";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import type { ExplainResponse, FindNodesResponse, PqlResponse } from "@/lib/types";

import { QueryHistoryPanel } from "./QueryHistoryPanel";
import { QueryResultsPanel } from "./QueryResultsPanel";
import { QueryTabBar } from "./QueryTabBar";
import { QueryTabContent } from "./QueryTabContent";
import { useQueryTabs, type QueryTab } from "./query-tabs-store";

const MIN_EDITOR_HEIGHT = 120;
const MIN_RESULTS_HEIGHT = 200;

export default function QueryView() {
  const ctx = useConsoleContext();
  const { tabs, activeId, activeTab, setActiveId, addTab, closeTab, renameTab, updateTab } = useQueryTabs();

  // Resizable split
  const [editorHeight, setEditorHeight] = useState(() => {
    try {
      const saved = localStorage.getItem("pelagodb.console.queryEditorHeight");
      return saved ? parseInt(saved, 10) : 480;
    } catch {
      return 480;
    }
  });
  const dragging = useRef(false);
  const startY = useRef(0);
  const startHeight = useRef(0);

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    dragging.current = true;
    startY.current = e.clientY;
    startHeight.current = editorHeight;

    const onMouseMove = (ev: MouseEvent) => {
      if (!dragging.current) return;
      const delta = ev.clientY - startY.current;
      const next = Math.max(MIN_EDITOR_HEIGHT, startHeight.current + delta);
      setEditorHeight(next);
    };

    const onMouseUp = () => {
      dragging.current = false;
      try {
        localStorage.setItem("pelagodb.console.queryEditorHeight", String(editorHeight));
      } catch { /* quota */ }
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  }, [editorHeight]);

  const onUpdateActiveTab = useCallback(
    (patch: Partial<QueryTab>) => {
      updateTab(activeTab.id, patch);
    },
    [activeTab.id, updateTab],
  );

  const onHistoryRestore = useCallback(
    (entry: { mode: "cel" | "pql"; expression: string; entityType: string }) => {
      updateTab(activeTab.id, {
        mode: entry.mode,
        expression: entry.expression,
        entityType: entry.entityType,
      });
    },
    [activeTab.id, updateTab],
  );

  return (
    <section className="space-y-4">
      {/* Editor Card */}
      <Card className="overflow-hidden">
        <QueryTabBar
          tabs={tabs}
          activeId={activeId}
          onSelect={setActiveId}
          onClose={closeTab}
          onAdd={() => addTab()}
          onRename={renameTab}
        />
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Query Studio</CardTitle>
              <CardDescription>
                Multi-tab editor with CEL/PQL, ⌘+Enter execution, and syntax highlighting.
              </CardDescription>
            </div>
            <QueryHistoryPanel onRestore={onHistoryRestore} />
          </div>
        </CardHeader>
        <CardContent>
          <div style={{ minHeight: MIN_EDITOR_HEIGHT }}>
            <QueryTabContent
              tab={activeTab}
              ctx={ctx}
              onUpdateTab={onUpdateActiveTab}
            />
          </div>
        </CardContent>

        {/* Resize handle */}
        <div
          className="flex h-1.5 cursor-row-resize items-center justify-center border-t border-border bg-surface-subtle hover:bg-accent/20"
          onMouseDown={onMouseDown}
          role="separator"
          aria-label="Resize editor and results"
        >
          <div className="h-0.5 w-10 rounded-full bg-muted/40" />
        </div>
      </Card>

      {/* Results Card */}
      <Card>
        <CardHeader>
          <SectionHeader
            title="Output Inspector"
            description="Results, explain output, export, and row detail drawer."
          />
        </CardHeader>
        <CardContent style={{ minHeight: MIN_RESULTS_HEIGHT }}>
          <QueryResultsPanel
            queryResult={activeTab.result as FindNodesResponse | PqlResponse | null}
            explainResult={activeTab.explainResult as ExplainResponse | null}
            resultKind={activeTab.resultKind}
            executionTimeMs={activeTab.executionTimeMs}
          />
        </CardContent>
      </Card>
    </section>
  );
}
