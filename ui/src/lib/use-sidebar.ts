import { useCallback, useSyncExternalStore } from "react";

const STORAGE_KEY = "pelagodb.console.sidebar";

let listeners: Array<() => void> = [];
let collapsed = false;

try {
  collapsed = localStorage.getItem(STORAGE_KEY) === "collapsed";
} catch {
  // restricted storage
}

function emit() {
  for (const fn of listeners) fn();
}

function subscribe(listener: () => void) {
  listeners = [...listeners, listener];
  return () => {
    listeners = listeners.filter((l) => l !== listener);
  };
}

function getSnapshot() {
  return collapsed;
}

export function useSidebar() {
  const isCollapsed = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  const setCollapsed = useCallback((next: boolean) => {
    collapsed = next;
    try {
      localStorage.setItem(STORAGE_KEY, next ? "collapsed" : "expanded");
    } catch {
      // quota / restricted
    }
    emit();
  }, []);

  const toggle = useCallback(() => {
    setCollapsed(!collapsed);
  }, [setCollapsed]);

  return { collapsed: isCollapsed, setCollapsed, toggle };
}
