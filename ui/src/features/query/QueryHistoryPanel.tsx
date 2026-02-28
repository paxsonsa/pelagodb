import { useMemo, useState } from "react";
import { Clock, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { clearQueryHistory, getQueryHistory, type QueryHistoryEntry } from "./query-history-store";

export function QueryHistoryPanel({
  onRestore,
}: {
  onRestore: (entry: QueryHistoryEntry) => void;
}) {
  const [open, setOpen] = useState(false);
  const [version, setVersion] = useState(0);

  const entries = useMemo(() => getQueryHistory(), [version, open]);

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger asChild>
        <Button type="button" variant="ghost" size="sm" className="gap-1.5">
          <Clock className="h-3.5 w-3.5" />
          History
        </Button>
      </SheetTrigger>
      <SheetContent side="right" className="w-[380px] sm:w-[420px]">
        <SheetHeader>
          <SheetTitle className="flex items-center justify-between">
            Query History
            {entries.length > 0 && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  clearQueryHistory();
                  setVersion((v) => v + 1);
                }}
              >
                <Trash2 className="mr-1 h-3.5 w-3.5" />
                Clear
              </Button>
            )}
          </SheetTitle>
        </SheetHeader>
        <div className="mt-4 space-y-2 overflow-y-auto pr-1">
          {entries.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted">No query history yet.</p>
          ) : (
            entries.map((entry, idx) => (
              <button
                key={`${entry.timestamp}-${idx}`}
                type="button"
                onClick={() => {
                  onRestore(entry);
                  setOpen(false);
                }}
                className="w-full rounded-md border border-border bg-surface-subtle p-3 text-left transition-colors hover:bg-foreground/5"
              >
                <div className="mb-1 flex items-center justify-between">
                  <span className="text-[11px] font-semibold uppercase tracking-wide text-accent">
                    {entry.mode}
                  </span>
                  <span className="text-[10px] text-muted">
                    {entry.rowCount} rows · {entry.executionTimeMs}ms
                  </span>
                </div>
                <pre className="truncate font-mono text-xs text-foreground/80">
                  {entry.expression}
                </pre>
                <time className="mt-1 block text-[10px] text-muted">
                  {new Date(entry.timestamp).toLocaleString()}
                </time>
              </button>
            ))
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
