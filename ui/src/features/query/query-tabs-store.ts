import { useSyncExternalStore } from "react";

export type QueryTab = {
  id: string;
  title: string;
  mode: "cel" | "pql";
  entityType: string;
  expression: string;
  snapshotMode: "strict" | "best_effort";
  allowDegrade: boolean;
  result: unknown | null;
  explainResult: unknown | null;
  executionTimeMs: number | null;
  resultKind: "cel" | "pql" | null;
};

const STORAGE_KEY = "pelagodb.console.queryTabs";
const MAX_TABS = 8;

let tabIdCounter = 1;

function newTab(partial?: Partial<QueryTab>): QueryTab {
  return {
    id: `tab-${tabIdCounter++}`,
    title: partial?.title ?? `Query ${tabIdCounter - 1}`,
    mode: partial?.mode ?? "cel",
    entityType: partial?.entityType ?? "",
    expression: partial?.expression ?? "",
    snapshotMode: partial?.snapshotMode ?? "strict",
    allowDegrade: partial?.allowDegrade ?? true,
    result: null,
    explainResult: null,
    executionTimeMs: null,
    resultKind: null,
  };
}

type TabsState = {
  tabs: QueryTab[];
  activeId: string;
};

function loadInitialState(): TabsState {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as TabsState;
      if (parsed.tabs?.length > 0) {
        tabIdCounter = Math.max(...parsed.tabs.map((t) => {
          const num = parseInt(t.id.replace("tab-", ""), 10);
          return isNaN(num) ? 0 : num;
        })) + 1;
        return parsed;
      }
    }
  } catch {
    // corrupt data
  }
  const initial = newTab({ title: "Query 1" });
  return { tabs: [initial], activeId: initial.id };
}

let state = loadInitialState();
let listeners: Array<() => void> = [];

function emit() {
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify({
      tabs: state.tabs.map((t) => ({ ...t, result: null, explainResult: null })),
      activeId: state.activeId,
    }));
  } catch {
    // quota exceeded
  }
  for (const fn of listeners) fn();
}

function subscribe(listener: () => void) {
  listeners = [...listeners, listener];
  return () => {
    listeners = listeners.filter((l) => l !== listener);
  };
}

function getSnapshot() {
  return state;
}

export function useQueryTabs() {
  const s = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  return {
    tabs: s.tabs,
    activeId: s.activeId,
    activeTab: s.tabs.find((t) => t.id === s.activeId) ?? s.tabs[0],

    setActiveId(id: string) {
      state = { ...state, activeId: id };
      emit();
    },

    addTab(partial?: Partial<QueryTab>) {
      if (state.tabs.length >= MAX_TABS) return;
      const tab = newTab(partial);
      state = { tabs: [...state.tabs, tab], activeId: tab.id };
      emit();
    },

    closeTab(id: string) {
      if (state.tabs.length <= 1) return;
      const idx = state.tabs.findIndex((t) => t.id === id);
      const next = state.tabs.filter((t) => t.id !== id);
      const newActive = id === state.activeId
        ? next[Math.min(idx, next.length - 1)].id
        : state.activeId;
      state = { tabs: next, activeId: newActive };
      emit();
    },

    renameTab(id: string, title: string) {
      state = {
        ...state,
        tabs: state.tabs.map((t) => (t.id === id ? { ...t, title } : t)),
      };
      emit();
    },

    updateTab(id: string, patch: Partial<QueryTab>) {
      state = {
        ...state,
        tabs: state.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)),
      };
      emit();
    },
  };
}
