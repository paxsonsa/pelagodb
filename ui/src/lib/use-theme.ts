import { useCallback, useEffect, useSyncExternalStore } from "react";

type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "pelagodb.console.theme";
const darkQuery = "(prefers-color-scheme: dark)";

function getStoredTheme(): Theme {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    // SSR / restricted storage
  }
  return "system";
}

function resolveEffective(theme: Theme): "light" | "dark" {
  if (theme === "system") {
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      return window.matchMedia(darkQuery).matches ? "dark" : "light";
    }
    return "light";
  }
  return theme;
}

function applyToDOM(effective: "light" | "dark") {
  if (effective === "dark") {
    document.documentElement.classList.add("dark");
  } else {
    document.documentElement.classList.remove("dark");
  }
}

// Tiny pub/sub so React can subscribe via useSyncExternalStore
let listeners: Array<() => void> = [];
let snapshot: { theme: Theme; effective: "light" | "dark" } = {
  theme: getStoredTheme(),
  effective: resolveEffective(getStoredTheme()),
};

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
  return snapshot;
}

// Respond to OS-level preference changes
if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
  window.matchMedia(darkQuery).addEventListener("change", () => {
    if (snapshot.theme === "system") {
      const effective = resolveEffective("system");
      snapshot = { ...snapshot, effective };
      applyToDOM(effective);
      emit();
    }
  });
}

export function useTheme() {
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  const setTheme = useCallback((next: Theme) => {
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // quota / restricted
    }
    const effective = resolveEffective(next);
    snapshot = { theme: next, effective };
    applyToDOM(effective);
    emit();
  }, []);

  // Ensure DOM is in sync on mount
  useEffect(() => {
    applyToDOM(state.effective);
  }, [state.effective]);

  return {
    theme: state.theme,
    effective: state.effective,
    setTheme,
    toggleTheme: useCallback(() => {
      const next = state.effective === "dark" ? "light" : "dark";
      setTheme(next);
    }, [setTheme, state.effective]),
  };
}
