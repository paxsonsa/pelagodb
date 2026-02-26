import { NavLink, Outlet, useLocation, useOutletContext } from "react-router-dom";
import { FormEvent, useMemo, useState } from "react";
import { apiRequest, AuthSession, ScopeContext } from "./lib/api";

export type ConsoleContext = {
  session: AuthSession;
  setSession: (next: AuthSession) => void;
  scope: ScopeContext;
  setScope: (next: ScopeContext) => void;
  notify: (message: string, kind?: "ok" | "error") => void;
};

const emptySession: AuthSession = {
  apiKey: "",
  accessToken: "",
  refreshToken: "",
};

const defaultScope: ScopeContext = {
  database: "default",
  namespace: "default",
};

type Toast = {
  message: string;
  kind: "ok" | "error";
};

export function useConsoleContext(): ConsoleContext {
  return useOutletContext<ConsoleContext>();
}

function AuthPanel({
  session,
  setSession,
  scope,
  notify,
}: {
  session: AuthSession;
  setSession: (next: AuthSession) => void;
  scope: ScopeContext;
  notify: (message: string, kind?: "ok" | "error") => void;
}) {
  const [mode, setMode] = useState<"api" | "token">("api");
  const [apiKeyInput, setApiKeyInput] = useState(session.apiKey);
  const [tokenInput, setTokenInput] = useState(session.accessToken);
  const [refreshInput, setRefreshInput] = useState(session.refreshToken);
  const [busy, setBusy] = useState(false);

  async function authenticateWithApiKey(event: FormEvent) {
    event.preventDefault();
    if (!apiKeyInput.trim()) {
      notify("Enter an API key", "error");
      return;
    }

    setBusy(true);
    try {
      const response = await apiRequest<{
        access_token: string;
        refresh_token: string;
      }>("/auth/authenticate", {
        method: "POST",
        scope,
        session: {
          ...session,
          apiKey: apiKeyInput,
        },
        body: {
          api_key: apiKeyInput,
        },
      });
      const next = {
        apiKey: apiKeyInput,
        accessToken: response.access_token,
        refreshToken: response.refresh_token,
      };
      setSession(next);
      setTokenInput(next.accessToken);
      setRefreshInput(next.refreshToken);
      notify("Authenticated via API key");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Authentication failed", "error");
    } finally {
      setBusy(false);
    }
  }

  async function validateToken(event: FormEvent) {
    event.preventDefault();
    if (!tokenInput.trim()) {
      notify("Enter a bearer token", "error");
      return;
    }

    setBusy(true);
    try {
      await apiRequest<{ valid: boolean }>("/auth/validate", {
        method: "POST",
        scope,
        session,
        body: {
          token: tokenInput,
        },
      });
      setSession({
        ...session,
        accessToken: tokenInput,
        refreshToken: refreshInput,
      });
      notify("Token validated");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Token validation failed", "error");
    } finally {
      setBusy(false);
    }
  }

  async function refreshToken(event: FormEvent) {
    event.preventDefault();
    if (!refreshInput.trim()) {
      notify("Enter a refresh token", "error");
      return;
    }

    setBusy(true);
    try {
      const response = await apiRequest<{
        access_token: string;
        refresh_token: string;
      }>("/auth/refresh", {
        method: "POST",
        scope,
        session,
        body: {
          refresh_token: refreshInput,
        },
      });
      const next = {
        ...session,
        accessToken: response.access_token,
        refreshToken: response.refresh_token,
      };
      setSession(next);
      setTokenInput(next.accessToken);
      setRefreshInput(next.refreshToken);
      notify("Token refreshed");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Token refresh failed", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="auth-shell" aria-label="Authentication">
      <header>
        <h2>Session</h2>
        <p>Use API key or bearer token for Console requests.</p>
      </header>
      <div className="auth-mode">
        <button
          className={mode === "api" ? "is-active" : ""}
          onClick={() => setMode("api")}
          type="button"
        >
          API Key
        </button>
        <button
          className={mode === "token" ? "is-active" : ""}
          onClick={() => setMode("token")}
          type="button"
        >
          Bearer
        </button>
      </div>
      {mode === "api" ? (
        <form onSubmit={authenticateWithApiKey} className="auth-form">
          <label>
            API Key
            <input
              value={apiKeyInput}
              onChange={(event) => setApiKeyInput(event.target.value)}
              placeholder="dev-admin-key"
            />
          </label>
          <button type="submit" disabled={busy}>
            {busy ? "Authenticating..." : "Authenticate"}
          </button>
        </form>
      ) : (
        <>
          <form onSubmit={validateToken} className="auth-form">
            <label>
              Access Token
              <textarea
                value={tokenInput}
                onChange={(event) => setTokenInput(event.target.value)}
                rows={3}
              />
            </label>
            <button type="submit" disabled={busy}>
              {busy ? "Validating..." : "Validate Token"}
            </button>
          </form>
          <form onSubmit={refreshToken} className="auth-form">
            <label>
              Refresh Token
              <input
                value={refreshInput}
                onChange={(event) => setRefreshInput(event.target.value)}
              />
            </label>
            <button type="submit" disabled={busy}>
              {busy ? "Refreshing..." : "Refresh Token"}
            </button>
          </form>
        </>
      )}
    </section>
  );
}

export default function App() {
  const [session, setSession] = useState<AuthSession>(emptySession);
  const [scope, setScope] = useState<ScopeContext>(defaultScope);
  const [toast, setToast] = useState<Toast | null>(null);
  const location = useLocation();

  const contextValue = useMemo<ConsoleContext>(
    () => ({
      session,
      setSession,
      scope,
      setScope,
      notify: (message, kind = "ok") => {
        setToast({ message, kind });
        window.setTimeout(() => setToast(null), 4000);
      },
    }),
    [scope, session],
  );

  return (
    <div className="shell">
      <aside className="sidebar">
        <h1>PelagoDB Console</h1>
        <p className="tagline">Graph Workbench + Ops Command Center</p>
        <div className="scope-controls">
          <label>
            Database
            <input
              value={scope.database}
              onChange={(event) =>
                setScope({
                  ...scope,
                  database: event.target.value,
                })
              }
            />
          </label>
          <label>
            Namespace
            <input
              value={scope.namespace}
              onChange={(event) =>
                setScope({
                  ...scope,
                  namespace: event.target.value,
                })
              }
            />
          </label>
        </div>
        <nav>
          <NavLink to="/explorer">Explorer</NavLink>
          <NavLink to="/query">Query</NavLink>
          <NavLink to="/ops">Ops</NavLink>
          <NavLink to="/admin">Admin</NavLink>
          <NavLink to="/watch">Watch</NavLink>
        </nav>
        <AuthPanel
          scope={scope}
          session={session}
          setSession={setSession}
          notify={contextValue.notify}
        />
      </aside>
      <main className="panel">
        <header className="panel-header">
          <h2>{location.pathname.replace("/", "").toUpperCase() || "EXPLORER"}</h2>
          <div className="status-pill">
            {session.accessToken ? "Bearer Active" : session.apiKey ? "API Key" : "No Auth"}
          </div>
        </header>
        <Outlet context={contextValue} />
      </main>
      {toast ? <div className={`toast ${toast.kind}`}>{toast.message}</div> : null}
    </div>
  );
}
