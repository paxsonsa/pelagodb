import { FormEvent, useMemo, useState } from "react";
import { KeyRound, ShieldCheck } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { apiRequest, normalizeApiError } from "@/lib/api";
import type {
  AuthenticateResponse,
  AuthSession,
  RefreshTokenResponse,
  ScopeContext,
  ValidateTokenResponse,
} from "@/lib/types";

function toPrincipal(
  principal?: { principal_id: string; principal_type: string; roles: string[] },
): AuthSession["principal"] {
  if (!principal) {
    return undefined;
  }

  return {
    id: principal.principal_id,
    type: principal.principal_type,
    roles: principal.roles,
  };
}

export function AuthSheet({
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
  const [open, setOpen] = useState(false);

  const [apiKeyInput, setApiKeyInput] = useState(session.apiKey);
  const [tokenInput, setTokenInput] = useState(session.accessToken);
  const [refreshInput, setRefreshInput] = useState(session.refreshToken);
  const [busy, setBusy] = useState(false);

  const currentIdentity = useMemo(() => {
    if (session.principal) {
      return `${session.principal.type}:${session.principal.id}`;
    }

    if (session.accessToken) {
      return "Bearer token active";
    }

    if (session.apiKey) {
      return "API key active";
    }

    return "Not authenticated";
  }, [session]);

  async function authenticateWithApiKey(event: FormEvent) {
    event.preventDefault();
    if (!apiKeyInput.trim()) {
      notify("Enter an API key", "error");
      return;
    }

    setBusy(true);

    try {
      const response = await apiRequest<AuthenticateResponse>("/auth/authenticate", {
        method: "POST",
        session: {
          ...session,
          apiKey: apiKeyInput,
        },
        scope,
        body: { api_key: apiKeyInput },
      });

      const nextSession: AuthSession = {
        apiKey: apiKeyInput,
        accessToken: response.access_token,
        refreshToken: response.refresh_token,
        expiresAt: response.expires_at,
        principal: toPrincipal(response.principal),
      };

      setSession(nextSession);
      setTokenInput(nextSession.accessToken);
      setRefreshInput(nextSession.refreshToken);
      notify("Authenticated via API key");
      setOpen(false);
    } catch (error) {
      notify(normalizeApiError(error), "error");
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
      const response = await apiRequest<ValidateTokenResponse>("/auth/validate", {
        method: "POST",
        session,
        scope,
        body: { token: tokenInput },
      });

      if (!response.valid) {
        notify("Token validation failed", "error");
        return;
      }

      const nextSession: AuthSession = {
        ...session,
        accessToken: tokenInput,
        refreshToken: refreshInput,
        expiresAt: response.expires_at,
        principal: toPrincipal(response.principal),
      };

      setSession(nextSession);
      notify("Token validated");
    } catch (error) {
      notify(normalizeApiError(error), "error");
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
      const response = await apiRequest<RefreshTokenResponse>("/auth/refresh", {
        method: "POST",
        session,
        scope,
        body: { refresh_token: refreshInput },
      });

      const nextSession: AuthSession = {
        ...session,
        accessToken: response.access_token,
        refreshToken: response.refresh_token,
        expiresAt: response.expires_at,
      };

      setSession(nextSession);
      setTokenInput(nextSession.accessToken);
      setRefreshInput(nextSession.refreshToken);
      notify("Token refreshed");
    } catch (error) {
      notify(normalizeApiError(error), "error");
    } finally {
      setBusy(false);
    }
  }

  function clearSession() {
    setSession({
      apiKey: "",
      accessToken: "",
      refreshToken: "",
    });
    setApiKeyInput("");
    setTokenInput("");
    setRefreshInput("");
    notify("Session cleared");
  }

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger asChild>
        <Button variant="secondary" size="sm">
          <ShieldCheck className="h-4 w-4" /> Session
        </Button>
      </SheetTrigger>
      <SheetContent side="right" className="w-full max-w-xl">
        <SheetHeader>
          <SheetTitle>Authentication Session</SheetTitle>
          <SheetDescription>Use API key or bearer token for all console requests.</SheetDescription>
          <p className="text-xs text-muted">Current identity: {currentIdentity}</p>
        </SheetHeader>

        <Tabs value={mode} onValueChange={(next) => setMode(next as "api" | "token")}> 
          <TabsList>
            <TabsTrigger value="api">
              <KeyRound className="h-3.5 w-3.5" /> API Key
            </TabsTrigger>
            <TabsTrigger value="token">Bearer</TabsTrigger>
          </TabsList>

          <TabsContent value="api" className="space-y-4">
            <form className="space-y-3" onSubmit={authenticateWithApiKey}>
              <label className="space-y-1">
                <Label>API Key</Label>
                <Input
                  value={apiKeyInput}
                  onChange={(event) => setApiKeyInput(event.target.value)}
                  placeholder="dev-admin-key"
                />
              </label>
              <Button type="submit" disabled={busy}>
                {busy ? "Authenticating..." : "Authenticate"}
              </Button>
            </form>
          </TabsContent>

          <TabsContent value="token" className="space-y-4">
            <form className="space-y-3" onSubmit={validateToken}>
              <label className="space-y-1">
                <Label>Access Token</Label>
                <Textarea
                  value={tokenInput}
                  onChange={(event) => setTokenInput(event.target.value)}
                  rows={4}
                  className="font-mono text-xs"
                />
              </label>
              <Button type="submit" disabled={busy}>
                {busy ? "Validating..." : "Validate Token"}
              </Button>
            </form>

            <form className="space-y-3" onSubmit={refreshToken}>
              <label className="space-y-1">
                <Label>Refresh Token</Label>
                <Input
                  value={refreshInput}
                  onChange={(event) => setRefreshInput(event.target.value)}
                  className="font-mono text-xs"
                />
              </label>
              <Button type="submit" variant="secondary" disabled={busy}>
                {busy ? "Refreshing..." : "Refresh Token"}
              </Button>
            </form>
          </TabsContent>
        </Tabs>

        <div className="mt-auto pt-4">
          <Button type="button" variant="ghost" onClick={clearSession}>
            Clear Session
          </Button>
        </div>
      </SheetContent>
    </Sheet>
  );
}
