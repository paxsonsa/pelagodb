import { FormEvent, useEffect, useRef, useState } from "react";
import { apiRequest, postSseStream } from "../lib/api";
import { useConsoleContext } from "../App";

type StreamEvent = {
  timestamp: string;
  event: string;
  payload: unknown;
};

export default function WatchPage() {
  const { session, scope, notify } = useConsoleContext();

  const [watchMode, setWatchMode] = useState<"point" | "query" | "namespace">("query");
  const [entityType, setEntityType] = useState("Person");
  const [nodeId, setNodeId] = useState("1_0");
  const [celExpression, setCelExpression] = useState("age >= 30");
  const [subscriptionId, setSubscriptionId] = useState("");
  const [events, setEvents] = useState<StreamEvent[]>([]);
  const [pollFallback, setPollFallback] = useState(false);
  const [running, setRunning] = useState(false);

  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    return () => {
      controllerRef.current?.abort();
      controllerRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (!pollFallback) {
      return;
    }

    const interval = window.setInterval(async () => {
      try {
        const snapshot = await apiRequest<{ subscriptions: Array<{ subscription_id: string }> }>(
          "/state/watch/subscriptions",
          { session, scope },
        );
        const ids = snapshot.subscriptions.map((item) => item.subscription_id).join(", ");
        setEvents((prev) => [
          {
            timestamp: new Date().toISOString(),
            event: "poll",
            payload: { active_subscriptions: ids || "none" },
          },
          ...prev,
        ].slice(0, 200));
      } catch {
        // keep fallback silent
      }
    }, 5000);

    return () => {
      window.clearInterval(interval);
    };
  }, [pollFallback, scope, session]);

  async function startStream(event: FormEvent) {
    event.preventDefault();
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setRunning(true);
    setPollFallback(false);
    setEvents([]);
    setSubscriptionId("");

    const path =
      watchMode === "point"
        ? "/watch/point/stream"
        : watchMode === "query"
          ? "/watch/query/stream"
          : "/watch/namespace/stream";

    const body =
      watchMode === "point"
        ? {
            entity_type: entityType,
            node_id: nodeId,
            options: {
              include_initial: true,
              initial_snapshot: true,
              ttl_secs: 600,
              heartbeat_secs: 10,
              max_queue_size: 256,
            },
          }
        : watchMode === "query"
          ? {
              entity_type: entityType,
              cel_expression: celExpression,
              options: {
                include_initial: true,
                initial_snapshot: true,
                ttl_secs: 600,
                heartbeat_secs: 10,
                max_queue_size: 256,
              },
            }
          : {
              options: {
                include_initial: false,
                ttl_secs: 600,
                heartbeat_secs: 10,
                max_queue_size: 256,
              },
            };

    void postSseStream(path, body, session, scope, {
      onEvent: (streamEvent) => {
        const payload = streamEvent.data;
        if (
          payload &&
          typeof payload === "object" &&
          "subscription_id" in (payload as Record<string, unknown>)
        ) {
          setSubscriptionId(
            String((payload as Record<string, unknown>).subscription_id ?? ""),
          );
        }
        setEvents((prev) => [
          {
            timestamp: new Date().toISOString(),
            event: streamEvent.event,
            payload,
          },
          ...prev,
        ].slice(0, 200));
      },
      onError: (message) => {
        setPollFallback(true);
        notify(`Watch stream failed, switched to polling fallback: ${message}`, "error");
      },
    }, controller.signal)
      .catch((error) => {
        setPollFallback(true);
        notify(error instanceof Error ? error.message : "Watch stream failed", "error");
      })
      .finally(() => {
        setRunning(false);
      });
  }

  async function stopStream() {
    controllerRef.current?.abort();
    controllerRef.current = null;
    setRunning(false);

    if (!subscriptionId) {
      return;
    }

    try {
      await apiRequest("/watch/cancel", {
        method: "POST",
        session,
        scope,
        body: {
          subscription_id: subscriptionId,
        },
      });
      notify("Subscription cancelled");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Cancel failed", "error");
    }
  }

  return (
    <section className="page-grid two-col">
      <article className="card">
        <h3>Live Watch Stream</h3>
        <form className="form-grid" onSubmit={startStream}>
          <label>
            Watch Mode
            <select
              value={watchMode}
              onChange={(event) => setWatchMode(event.target.value as "point" | "query" | "namespace")}
            >
              <option value="query">Query</option>
              <option value="point">Point</option>
              <option value="namespace">Namespace</option>
            </select>
          </label>

          {watchMode !== "namespace" ? (
            <label>
              Entity Type
              <input value={entityType} onChange={(event) => setEntityType(event.target.value)} />
            </label>
          ) : null}

          {watchMode === "point" ? (
            <label>
              Node ID
              <input value={nodeId} onChange={(event) => setNodeId(event.target.value)} />
            </label>
          ) : null}

          {watchMode === "query" ? (
            <label>
              CEL Expression
              <textarea
                value={celExpression}
                onChange={(event) => setCelExpression(event.target.value)}
                rows={3}
              />
            </label>
          ) : null}

          <div className="actions-row">
            <button type="submit" disabled={running}>
              {running ? "Streaming..." : "Start Stream"}
            </button>
            <button type="button" onClick={stopStream}>
              Stop + Cancel
            </button>
          </div>
        </form>

        <div className="status-line">
          <span>Subscription: {subscriptionId || "(pending)"}</span>
          <span>{pollFallback ? "Polling fallback active" : "Streaming"}</span>
        </div>
      </article>

      <article className="card">
        <h3>Event Feed</h3>
        {events.length === 0 ? (
          <p className="empty-state">No events yet.</p>
        ) : (
          <div className="event-feed">
            {events.map((eventItem, index) => (
              <article key={`${eventItem.timestamp}-${index}`} className="event-row">
                <header>
                  <strong>{eventItem.event}</strong>
                  <time>{eventItem.timestamp}</time>
                </header>
                <pre>{JSON.stringify(eventItem.payload, null, 2)}</pre>
              </article>
            ))}
          </div>
        )}
      </article>
    </section>
  );
}
