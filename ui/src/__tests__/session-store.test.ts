import {
  appendStoredScopeHistory,
  clearStoredSession,
  EMPTY_SESSION,
  readStoredScope,
  readStoredScopeHistory,
  readStoredSession,
  writeStoredScope,
  writeStoredSession,
} from "@/lib/session-store";

describe("session-store", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
  });

  it("persists and restores auth session", () => {
    writeStoredSession({
      apiKey: "dev-key",
      accessToken: "access-token",
      refreshToken: "refresh-token",
      expiresAt: 123,
      principal: {
        id: "ops-admin",
        type: "user",
        roles: ["admin"],
      },
    });

    expect(readStoredSession()).toEqual({
      apiKey: "dev-key",
      accessToken: "access-token",
      refreshToken: "refresh-token",
      expiresAt: 123,
      principal: {
        id: "ops-admin",
        type: "user",
        roles: ["admin"],
      },
    });
  });

  it("clears session to empty defaults", () => {
    writeStoredSession({
      apiKey: "dev-key",
      accessToken: "token",
      refreshToken: "refresh",
    });

    clearStoredSession();

    expect(readStoredSession()).toEqual(EMPTY_SESSION);
  });

  it("persists and restores scope context", () => {
    writeStoredScope({ database: "analytics", namespace: "prod" });

    expect(readStoredScope()).toEqual({
      database: "analytics",
      namespace: "prod",
    });
  });

  it("tracks scope history for guided selectors", () => {
    appendStoredScopeHistory({ database: "default", namespace: "default" });
    appendStoredScopeHistory({ database: "analytics", namespace: "prod" });
    appendStoredScopeHistory({ database: "analytics", namespace: "staging" });
    appendStoredScopeHistory({ database: "analytics", namespace: "prod" });

    expect(readStoredScopeHistory()).toEqual([
      { database: "analytics", namespace: "prod" },
      { database: "analytics", namespace: "staging" },
      { database: "default", namespace: "default" },
    ]);
  });

  it("caps scope history to recent entries", () => {
    for (let index = 0; index < 30; index += 1) {
      appendStoredScopeHistory({
        database: `db-${index}`,
        namespace: `ns-${index}`,
      });
    }

    const history = readStoredScopeHistory();
    expect(history).toHaveLength(24);
    expect(history[0]).toEqual({ database: "db-29", namespace: "ns-29" });
    expect(history[23]).toEqual({ database: "db-6", namespace: "ns-6" });
  });
});
