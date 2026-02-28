import { useCallback, useEffect, useMemo, useState } from "react";
import { Outlet, useLocation, useNavigate, useOutletContext } from "react-router-dom";
import { toast, Toaster } from "sonner";

import { AppShell } from "@/components/layout/AppShell";
import { AuthSheet } from "@/components/layout/AuthSheet";
import { Breadcrumbs } from "@/components/layout/Breadcrumbs";
import { CommandPalette } from "@/components/layout/CommandPalette";
import { ScopeSwitcher } from "@/components/layout/ScopeSwitcher";
import { ThemeToggle } from "@/components/layout/ThemeToggle";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { useHotkey } from "@/hooks/use-hotkeys";
import { useSidebar } from "@/lib/use-sidebar";
import { useTheme } from "@/lib/use-theme";
import { apiRequest, normalizeApiError } from "@/lib/api";
import {
  appendStoredScopeHistory,
  clearStoredSession,
  readStoredScope,
  readStoredScopeHistory,
  readStoredSession,
  writeStoredScope,
  writeStoredSession,
} from "@/lib/session-store";
import type { AuthSession, SchemaDefinition, SchemaListResponse, ScopeContext } from "@/lib/types";

const routeTitles: Record<string, string> = {
  "/explorer": "Explorer Workflow",
  "/query": "Query Studio",
  "/ops": "Operations Surface",
  "/admin": "Administrative Actions",
  "/watch": "Watch Streaming",
  "/schema": "Schema Browser",
};

function parseScopedRole(role: string): ScopeContext | null {
  const prefix = "namespace-admin:";
  if (!role.toLowerCase().startsWith(prefix)) {
    return null;
  }

  const raw = role.slice(prefix.length);
  const [database, ...namespaceParts] = raw.split(":");
  const namespace = namespaceParts.join(":");
  if (!database || !namespace) {
    return null;
  }

  return {
    database: database.trim(),
    namespace: namespace.trim(),
  };
}

export type ConsoleContext = {
  session: AuthSession;
  setSession: (next: AuthSession) => void;
  scope: ScopeContext;
  setScope: (next: ScopeContext) => void;
  notify: (message: string, kind?: "ok" | "error") => void;
  schemaCatalog: SchemaDefinition[];
  schemaLoading: boolean;
  schemaError: string | null;
  refreshSchemaCatalog: () => Promise<void>;
};

export function useConsoleContext(): ConsoleContext {
  return useOutletContext<ConsoleContext>();
}

export default function App() {
  const [session, setSession] = useState<AuthSession>(() => readStoredSession());
  const [scope, setScope] = useState<ScopeContext>(() => readStoredScope());
  const [scopeHistory, setScopeHistory] = useState<ScopeContext[]>(() => readStoredScopeHistory());
  const [schemaCatalog, setSchemaCatalog] = useState<SchemaDefinition[]>([]);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [schemaError, setSchemaError] = useState<string | null>(null);
  const location = useLocation();
  const navigate = useNavigate();
  const { toggle: toggleSidebar } = useSidebar();
  const { effective: themeEffective } = useTheme();

  // Global keyboard shortcuts: [ to toggle sidebar, 1-5 for nav
  useHotkey("[", toggleSidebar);
  useHotkey("1", useCallback(() => navigate("/explorer"), [navigate]));
  useHotkey("2", useCallback(() => navigate("/query"), [navigate]));
  useHotkey("3", useCallback(() => navigate("/ops"), [navigate]));
  useHotkey("4", useCallback(() => navigate("/admin"), [navigate]));
  useHotkey("5", useCallback(() => navigate("/watch"), [navigate]));
  useHotkey("6", useCallback(() => navigate("/schema"), [navigate]));

  const notify = useCallback((message: string, kind: "ok" | "error" = "ok") => {
    if (kind === "error") {
      toast.error(message);
    } else {
      toast.success(message);
    }
  }, []);

  useEffect(() => {
    if (session.accessToken || session.apiKey || session.refreshToken) {
      writeStoredSession(session);
    } else {
      clearStoredSession();
    }
  }, [session]);

  useEffect(() => {
    writeStoredScope(scope);
    setScopeHistory(appendStoredScopeHistory(scope));
  }, [scope]);

  const roleScopes = useMemo(() => {
    const parsed = (session.principal?.roles ?? [])
      .map((role) => parseScopedRole(role))
      .filter((scopeEntry): scopeEntry is ScopeContext => Boolean(scopeEntry?.database && scopeEntry?.namespace));

    return parsed;
  }, [session.principal]);

  const selectorScopes = useMemo(() => {
    const deduped = new Map<string, ScopeContext>();
    const candidates = [scope, ...scopeHistory, ...roleScopes, { database: "default", namespace: "default" }];

    for (const candidate of candidates) {
      const database = candidate.database.trim() || "default";
      const namespace = candidate.namespace.trim() || "default";
      const key = `${database}::${namespace}`;
      if (!deduped.has(key)) {
        deduped.set(key, { database, namespace });
      }
    }

    return Array.from(deduped.values());
  }, [roleScopes, scope, scopeHistory]);

  const databaseOptions = useMemo(() => {
    const unique = new Set(selectorScopes.map((entry) => entry.database));
    return Array.from(unique.values()).sort((left, right) => left.localeCompare(right));
  }, [selectorScopes]);

  const namespaceOptionsByDatabase = useMemo(() => {
    const map = new Map<string, Set<string>>();
    for (const entry of selectorScopes) {
      const namespaces = map.get(entry.database) ?? new Set<string>();
      namespaces.add(entry.namespace);
      map.set(entry.database, namespaces);
    }

    const sorted: Record<string, string[]> = {};
    for (const [database, namespaces] of map.entries()) {
      sorted[database] = Array.from(namespaces.values()).sort((left, right) => left.localeCompare(right));
    }

    return sorted;
  }, [selectorScopes]);

  const refreshSchemaCatalog = useCallback(async () => {
    setSchemaLoading(true);
    setSchemaError(null);

    try {
      const response = await apiRequest<SchemaListResponse>("/schema", {
        session,
        scope,
      });
      setSchemaCatalog(response.schemas ?? []);
    } catch (error) {
      const normalized = normalizeApiError(error);
      setSchemaError(normalized);
      setSchemaCatalog([]);
    } finally {
      setSchemaLoading(false);
    }
  }, [scope, session]);

  useEffect(() => {
    void refreshSchemaCatalog();
  }, [refreshSchemaCatalog]);

  const contextValue = useMemo<ConsoleContext>(
    () => ({
      session,
      setSession,
      scope,
      setScope,
      notify,
      schemaCatalog,
      schemaLoading,
      schemaError,
      refreshSchemaCatalog,
    }),
    [
      notify,
      refreshSchemaCatalog,
      schemaCatalog,
      schemaError,
      schemaLoading,
      scope,
      session,
    ],
  );

  const routeTitle = routeTitles[location.pathname] ?? "Explorer Workflow";

  const authStatus = session.accessToken
    ? "Bearer Active"
    : session.apiKey
      ? "API Key Active"
      : "No Auth";

  return (
    <>
      <AppShell
        routeTitle={routeTitle}
        authStatus={authStatus}
        headerCenter={<CommandPalette onRefreshSchema={refreshSchemaCatalog} />}
        headerRight={<ThemeToggle />}
        scopeControls={
          <ScopeSwitcher
            scope={scope}
            setScope={setScope}
            databaseOptions={databaseOptions}
            namespaceOptionsByDatabase={namespaceOptionsByDatabase}
            schemaCount={schemaCatalog.length}
            schemaLoading={schemaLoading}
            onRefreshSchema={refreshSchemaCatalog}
          />
        }
        authControls={
          <AuthSheet
            session={session}
            setSession={setSession}
            scope={scope}
            notify={notify}
          />
        }
        schemaMeta={
          schemaError ? (
            <StatusBadge state="warning">Schema Catalog Unavailable</StatusBadge>
          ) : schemaLoading ? (
            <StatusBadge state="active">Refreshing Schemas</StatusBadge>
          ) : (
            <StatusBadge state="ok">Schemas Ready</StatusBadge>
          )
        }
      >
        <Outlet context={contextValue} />
      </AppShell>
      <Toaster
        position="bottom-right"
        closeButton
        richColors
        theme={themeEffective}
        toastOptions={{
          style: {
            background: "var(--panel)",
            color: "var(--foreground)",
            border: "1px solid var(--border)",
          },
        }}
      />
    </>
  );
}
