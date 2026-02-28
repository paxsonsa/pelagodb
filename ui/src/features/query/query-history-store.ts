const STORAGE_KEY = "pelagodb.console.queryHistory";
const MAX_HISTORY = 50;

export type QueryHistoryEntry = {
  timestamp: string;
  mode: "cel" | "pql";
  expression: string;
  entityType: string;
  rowCount: number;
  executionTimeMs: number;
};

function load(): QueryHistoryEntry[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as QueryHistoryEntry[]) : [];
  } catch {
    return [];
  }
}

function save(entries: QueryHistoryEntry[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries));
  } catch {
    // quota
  }
}

export function pushQueryHistory(entry: QueryHistoryEntry) {
  const entries = [entry, ...load()].slice(0, MAX_HISTORY);
  save(entries);
}

export function getQueryHistory(): QueryHistoryEntry[] {
  return load();
}

export function clearQueryHistory() {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // restricted
  }
}
