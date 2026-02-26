export type AuthSession = {
  apiKey: string;
  accessToken: string;
  refreshToken: string;
};

export type ScopeContext = {
  database: string;
  namespace: string;
};

export type ApiErrorShape = {
  error?: {
    code?: string;
    message?: string;
  };
};

export class ApiError extends Error {
  status: number;
  code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

function authHeaders(session: AuthSession): Record<string, string> {
  if (session.accessToken.trim().length > 0) {
    return { Authorization: `Bearer ${session.accessToken.trim()}` };
  }
  if (session.apiKey.trim().length > 0) {
    return { "x-api-key": session.apiKey.trim() };
  }
  return {};
}

function scopeHeaders(scope: ScopeContext): Record<string, string> {
  return {
    "x-pelago-database": scope.database,
    "x-pelago-namespace": scope.namespace,
  };
}

export async function apiRequest<T>(
  path: string,
  options: {
    method?: "GET" | "POST";
    body?: unknown;
    session: AuthSession;
    scope: ScopeContext;
  },
): Promise<T> {
  const response = await fetch(`/ui/api/v1${path}`, {
    method: options.method ?? "GET",
    headers: {
      "Content-Type": "application/json",
      ...authHeaders(options.session),
      ...scopeHeaders(options.scope),
    },
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });

  if (!response.ok) {
    let parsed: ApiErrorShape | undefined;
    try {
      parsed = (await response.json()) as ApiErrorShape;
    } catch {
      parsed = undefined;
    }
    const message =
      parsed?.error?.message ?? `${response.status} ${response.statusText}`;
    const code = parsed?.error?.code ?? "HTTP_ERROR";
    throw new ApiError(response.status, code, message);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

export async function metricsRaw(
  session: AuthSession,
  scope: ScopeContext,
): Promise<string> {
  const response = await fetch("/ui/api/v1/metrics/raw", {
    method: "GET",
    headers: {
      ...authHeaders(session),
      ...scopeHeaders(scope),
    },
  });

  if (!response.ok) {
    let parsed: ApiErrorShape | undefined;
    try {
      parsed = (await response.json()) as ApiErrorShape;
    } catch {
      parsed = undefined;
    }
    const message =
      parsed?.error?.message ?? `${response.status} ${response.statusText}`;
    const code = parsed?.error?.code ?? "HTTP_ERROR";
    throw new ApiError(response.status, code, message);
  }

  return response.text();
}

type StreamHandlers = {
  onEvent: (event: { event: string; data: unknown }) => void;
  onError: (message: string) => void;
};

function parseEventLine(raw: string): { event: string; data: string } | null {
  const lines = raw.split("\n");
  let event = "message";
  const dataParts: string[] = [];

  for (const line of lines) {
    if (line.startsWith("event:")) {
      event = line.slice(6).trim();
    } else if (line.startsWith("data:")) {
      dataParts.push(line.slice(5).trim());
    }
  }

  if (dataParts.length === 0) {
    return null;
  }

  return { event, data: dataParts.join("\n") };
}

export async function postSseStream(
  path: string,
  body: unknown,
  session: AuthSession,
  scope: ScopeContext,
  handlers: StreamHandlers,
  signal: AbortSignal,
): Promise<void> {
  const response = await fetch(`/ui/api/v1${path}`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "text/event-stream",
      ...authHeaders(session),
      ...scopeHeaders(scope),
    },
    body: JSON.stringify(body),
    signal,
  });

  if (!response.ok || !response.body) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const parsed = (await response.json()) as ApiErrorShape;
      message = parsed?.error?.message ?? message;
    } catch {
      // ignored
    }
    handlers.onError(message);
    return;
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const result = await reader.read();
    if (result.done) {
      break;
    }

    buffer += decoder.decode(result.value, { stream: true });

    let splitAt = buffer.indexOf("\n\n");
    while (splitAt !== -1) {
      const chunk = buffer.slice(0, splitAt).trim();
      buffer = buffer.slice(splitAt + 2);
      const parsed = parseEventLine(chunk);
      if (parsed) {
        try {
          const data = JSON.parse(parsed.data) as unknown;
          handlers.onEvent({ event: parsed.event, data });
        } catch {
          handlers.onEvent({ event: parsed.event, data: parsed.data });
        }
      }
      splitAt = buffer.indexOf("\n\n");
    }
  }
}
