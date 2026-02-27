import type { AuthSession, ScopeContext } from "@/lib/types";

const AUTH_KEY = "pelagodb.console.auth.v2";
const SCOPE_KEY = "pelagodb.console.scope.v2";
const SCOPE_HISTORY_KEY = "pelagodb.console.scope-history.v1";
const MAX_SCOPE_HISTORY = 24;

const DEFAULT_SCOPE: ScopeContext = {
  database: "default",
  namespace: "default",
};

const EMPTY_SESSION: AuthSession = {
  apiKey: "",
  accessToken: "",
  refreshToken: "",
};

function normalizeScope(scope: Partial<ScopeContext> | null | undefined): ScopeContext {
  return {
    database: scope?.database?.trim() || DEFAULT_SCOPE.database,
    namespace: scope?.namespace?.trim() || DEFAULT_SCOPE.namespace,
  };
}

function dedupeScopeHistory(scopes: ScopeContext[]): ScopeContext[] {
  const seen = new Set<string>();
  const history: ScopeContext[] = [];

  for (const scope of scopes) {
    const normalized = normalizeScope(scope);
    const key = `${normalized.database}::${normalized.namespace}`;
    if (seen.has(key)) {
      continue;
    }

    history.push(normalized);
    seen.add(key);
    if (history.length >= MAX_SCOPE_HISTORY) {
      break;
    }
  }

  return history;
}

function safeParse<T>(raw: string | null): T | null {
  if (!raw) {
    return null;
  }

  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

function hasSessionStorage(): boolean {
  return typeof window !== "undefined" && Boolean(window.sessionStorage);
}

export function readStoredSession(): AuthSession {
  if (!hasSessionStorage()) {
    return EMPTY_SESSION;
  }

  const parsed = safeParse<AuthSession>(window.sessionStorage.getItem(AUTH_KEY));
  if (!parsed) {
    return EMPTY_SESSION;
  }

  return {
    apiKey: parsed.apiKey ?? "",
    accessToken: parsed.accessToken ?? "",
    refreshToken: parsed.refreshToken ?? "",
    expiresAt: parsed.expiresAt,
    principal: parsed.principal,
  };
}

export function writeStoredSession(session: AuthSession): void {
  if (!hasSessionStorage()) {
    return;
  }

  window.sessionStorage.setItem(AUTH_KEY, JSON.stringify(session));
}

export function clearStoredSession(): void {
  if (!hasSessionStorage()) {
    return;
  }

  window.sessionStorage.removeItem(AUTH_KEY);
}

export function readStoredScope(): ScopeContext {
  if (!hasSessionStorage()) {
    return DEFAULT_SCOPE;
  }

  const parsed = safeParse<Partial<ScopeContext>>(window.sessionStorage.getItem(SCOPE_KEY));
  if (!parsed) {
    return DEFAULT_SCOPE;
  }

  return normalizeScope(parsed);
}

export function writeStoredScope(scope: ScopeContext): void {
  if (!hasSessionStorage()) {
    return;
  }

  window.sessionStorage.setItem(SCOPE_KEY, JSON.stringify(normalizeScope(scope)));
}

export function readStoredScopeHistory(): ScopeContext[] {
  if (!hasSessionStorage()) {
    return [DEFAULT_SCOPE];
  }

  const parsed = safeParse<ScopeContext[]>(window.sessionStorage.getItem(SCOPE_HISTORY_KEY));
  if (!parsed || !Array.isArray(parsed)) {
    return [DEFAULT_SCOPE];
  }

  const history = dedupeScopeHistory(parsed);
  return history.length > 0 ? history : [DEFAULT_SCOPE];
}

export function writeStoredScopeHistory(scopes: ScopeContext[]): ScopeContext[] {
  if (!hasSessionStorage()) {
    return dedupeScopeHistory(scopes);
  }

  const history = dedupeScopeHistory(scopes);
  window.sessionStorage.setItem(SCOPE_HISTORY_KEY, JSON.stringify(history));
  return history;
}

export function appendStoredScopeHistory(scope: ScopeContext): ScopeContext[] {
  const history = readStoredScopeHistory();
  return writeStoredScopeHistory([normalizeScope(scope), ...history]);
}

export { AUTH_KEY, SCOPE_KEY, SCOPE_HISTORY_KEY, DEFAULT_SCOPE, EMPTY_SESSION };
