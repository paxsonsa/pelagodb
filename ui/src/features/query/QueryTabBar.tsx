import { useState } from "react";
import { Plus, X } from "lucide-react";

import { cn } from "@/lib/utils";
import type { QueryTab } from "./query-tabs-store";

export function QueryTabBar({
  tabs,
  activeId,
  onSelect,
  onClose,
  onAdd,
  onRename,
}: {
  tabs: QueryTab[];
  activeId: string;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onAdd: () => void;
  onRename: (id: string, title: string) => void;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  function startRename(tab: QueryTab) {
    setEditingId(tab.id);
    setEditValue(tab.title);
  }

  function commitRename() {
    if (editingId && editValue.trim()) {
      onRename(editingId, editValue.trim());
    }
    setEditingId(null);
  }

  return (
    <div className="flex items-center gap-0.5 border-b border-border bg-surface-subtle px-2">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className={cn(
            "group flex h-9 cursor-pointer items-center gap-1.5 rounded-t-md px-3 text-xs font-medium transition-colors",
            tab.id === activeId
              ? "border-b-2 border-accent bg-panel text-foreground"
              : "text-muted hover:text-foreground",
          )}
          onClick={() => onSelect(tab.id)}
          onDoubleClick={() => startRename(tab)}
        >
          {editingId === tab.id ? (
            <input
              className="w-20 bg-transparent text-xs outline-none"
              value={editValue}
              onChange={(e) => setEditValue(e.target.value)}
              onBlur={commitRename}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitRename();
                if (e.key === "Escape") setEditingId(null);
              }}
              autoFocus
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <span className="max-w-[100px] truncate">{tab.title}</span>
          )}
          {tabs.length > 1 && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onClose(tab.id);
              }}
              className="ml-1 hidden rounded-sm p-0.5 text-muted hover:text-foreground group-hover:inline-flex"
              aria-label={`Close ${tab.title}`}
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>
      ))}
      {tabs.length < 8 && (
        <button
          type="button"
          onClick={onAdd}
          className="flex h-9 items-center px-2 text-muted hover:text-foreground"
          aria-label="New query tab"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}
