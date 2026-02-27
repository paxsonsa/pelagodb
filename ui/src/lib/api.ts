import type {
  ApiErrorShape,
  AuthSession,
  ScopeContext,
  StreamEventPayload,
} from "@/lib/types";

export class ApiError extends Error {
  status: number;
  code: string;
  retryAfter?: number;

  constructor(status: number, code: string, message: string, retryAfter?: number) {
    super(message);
    this.status = status;
    this.code = code;
    this.retryAfter = retryAfter;
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

function parseRetryAfter(raw: string | null): number | undefined {
  if (!raw) {
    return undefined;
  }

  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

async function parseErrorResponse(response: Response): Promise<ApiError> {
  const retryAfter = parseRetryAfter(response.headers.get("retry-after"));
  let parsed: ApiErrorShape | undefined;

  try {
    parsed = (await response.json()) as ApiErrorShape;
  } catch {
    parsed = undefined;
  }

  const message = parsed?.error?.message ?? `${response.status} ${response.statusText}`;
  const code = parsed?.error?.code ?? "HTTP_ERROR";
  return new ApiError(response.status, code, message, retryAfter);
}

export function normalizeApiError(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 429 && error.retryAfter) {
      return `${error.message}. Retry in ${error.retryAfter}s.`;
    }

    if (error.status === 401) {
      return `${error.message}. Re-authenticate and try again.`;
    }

    return error.message;
  }

  if (error instanceof Error) {
    return error.message;
  }

  return "Unexpected request error";
}

export async function apiRequest<T>(
  path: string,
  options: {
    method?: "GET" | "POST";
    body?: unknown;
    session: AuthSession;
    scope: ScopeContext;
    signal?: AbortSignal;
  },
): Promise<T> {
  const response = await fetch(`/ui/api/v1${path}`, {
    method: options.method ?? "GET",
    headers: {
      ...(options.body !== undefined ? { "Content-Type": "application/json" } : {}),
      ...authHeaders(options.session),
      ...scopeHeaders(options.scope),
    },
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    signal: options.signal,
  });

  if (!response.ok) {
    throw await parseErrorResponse(response);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    return (await response.json()) as T;
  }

  return (await response.text()) as T;
}

export async function metricsRaw(
  session: AuthSession,
  scope: ScopeContext,
  signal?: AbortSignal,
): Promise<string> {
  const response = await fetch("/ui/api/v1/metrics/raw", {
    method: "GET",
    headers: {
      ...authHeaders(session),
      ...scopeHeaders(scope),
    },
    signal,
  });

  if (!response.ok) {
    throw await parseErrorResponse(response);
  }

  return response.text();
}

type ParsedEventLine = {
  event: string;
  data: string;
};

export function parseEventLine(raw: string): ParsedEventLine | null {
  const lines = raw.split(/\r?\n/);
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

  return {
    event,
    data: dataParts.join("\n"),
  };
}

type StreamHandlers = {
  onEvent: (event: { event: string; data: StreamEventPayload | string }) => void;
  onError: (message: string) => void;
};

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
    handlers.onError((await parseErrorResponse(response)).message);
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

    let splitAt = buffer.search(/\r?\n\r?\n/);
    while (splitAt !== -1) {
      const chunk = buffer.slice(0, splitAt).trim();
      const separatorMatch = buffer.match(/\r?\n\r?\n/);
      const separatorLength = separatorMatch ? separatorMatch[0].length : 2;
      buffer = buffer.slice(splitAt + separatorLength);

      const parsed = parseEventLine(chunk);
      if (parsed) {
        try {
          const data = JSON.parse(parsed.data) as StreamEventPayload;
          handlers.onEvent({ event: parsed.event, data });
        } catch {
          handlers.onEvent({ event: parsed.event, data: parsed.data });
        }
      }

      splitAt = buffer.search(/\r?\n\r?\n/);
    }
  }
}
