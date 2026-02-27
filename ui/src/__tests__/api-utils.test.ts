import { ApiError, normalizeApiError, parseEventLine } from "@/lib/api";

describe("api utilities", () => {
  it("parses SSE event and payload data lines", () => {
    const parsed = parseEventLine('event: watch\ndata: {"subscription_id":"abc"}\n');

    expect(parsed).toEqual({
      event: "watch",
      data: '{"subscription_id":"abc"}',
    });
  });

  it("returns null when SSE line has no data", () => {
    const parsed = parseEventLine("event: ping\n");
    expect(parsed).toBeNull();
  });

  it("adds retry guidance for rate-limited errors", () => {
    const message = normalizeApiError(
      new ApiError(429, "RATE_LIMITED", "too many requests", 12),
    );

    expect(message).toContain("Retry in 12s");
  });

  it("adds re-auth guidance for unauthorized errors", () => {
    const message = normalizeApiError(new ApiError(401, "UNAUTHORIZED", "auth required"));
    expect(message).toContain("Re-authenticate");
  });
});
