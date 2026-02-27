import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { RadioTower, Search, Waves } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EntityTypeInput } from "@/components/shared/EntityTypeInput";
import { EmptyState } from "@/components/shared/EmptyState";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { apiRequest, normalizeApiError, postSseStream } from "@/lib/api";
import type { StreamEventPayload, WatchEventRow, WatchSubscriptionsResponse } from "@/lib/types";

type WatchMode = "point" | "query" | "namespace";

export default function WatchView() {
  const { session, scope, notify, schemaCatalog } = useConsoleContext();

  const [watchMode, setWatchMode] = useState<WatchMode>("query");
  const [entityType, setEntityType] = useState("Person");
  const [nodeId, setNodeId] = useState("1_0");
  const [celExpression, setCelExpression] = useState("age >= 30");
  const [subscriptionId, setSubscriptionId] = useState("");
  const [events, setEvents] = useState<WatchEventRow[]>([]);
  const [pollFallback, setPollFallback] = useState(false);
  const [running, setRunning] = useState(false);
  const [eventTypeFilter, setEventTypeFilter] = useState("all");
  const [searchQuery, setSearchQuery] = useState("");

  const controllerRef = useRef<AbortController | null>(null);
  const subscriptionIdRef = useRef("");
  const sessionRef = useRef(session);
  const scopeRef = useRef(scope);

  useEffect(() => {
    subscriptionIdRef.current = subscriptionId;
  }, [subscriptionId]);

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  useEffect(() => {
    scopeRef.current = scope;
  }, [scope]);

  const cancelSubscription = useCallback(async (subscription: string, keepalive = false): Promise<boolean> => {
    if (!subscription) {
      return false;
    }

    const currentSession = sessionRef.current;
    const currentScope = scopeRef.current;
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      "x-pelago-database": currentScope.database,
      "x-pelago-namespace": currentScope.namespace,
    };

    if (currentSession.accessToken.trim()) {
      headers.Authorization = `Bearer ${currentSession.accessToken.trim()}`;
    } else if (currentSession.apiKey.trim()) {
      headers["x-api-key"] = currentSession.apiKey.trim();
    }

    try {
      const response = await fetch("/ui/api/v1/watch/cancel", {
        method: "POST",
        headers,
        body: JSON.stringify({ subscription_id: subscription }),
        keepalive,
      });
      return response.ok;
    } catch {
      return false;
    }
  }, []);

  useEffect(() => {
    return () => {
      controllerRef.current?.abort();
      controllerRef.current = null;
      const currentSubscription = subscriptionIdRef.current;
      if (currentSubscription) {
        void cancelSubscription(currentSubscription, true);
      }
    };
  }, [cancelSubscription]);

  useEffect(() => {
    const handleBeforeUnload = () => {
      const currentSubscription = subscriptionIdRef.current;
      if (currentSubscription) {
        void cancelSubscription(currentSubscription, true);
      }
    };

    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
    };
  }, [cancelSubscription]);

  useEffect(() => {
    if (!pollFallback) {
      return;
    }

    const interval = window.setInterval(async () => {
      try {
        const snapshot = await apiRequest<WatchSubscriptionsResponse>(
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
        ].slice(0, 250));
      } catch {
        // silent fallback loop
      }
    }, 5000);

    return () => {
      window.clearInterval(interval);
    };
  }, [pollFallback, scope, session]);

  async function startStream(event: FormEvent) {
    event.preventDefault();

    const existingSubscription = subscriptionIdRef.current;
    if (existingSubscription) {
      await cancelSubscription(existingSubscription);
      subscriptionIdRef.current = "";
      setSubscriptionId("");
    }

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

    void postSseStream(
      path,
      body,
      session,
      scope,
      {
        onEvent: (streamEvent) => {
          const payload = streamEvent.data;
          if (
            payload &&
            typeof payload === "object" &&
            "subscription_id" in (payload as Record<string, unknown>)
          ) {
            setSubscriptionId(String((payload as Record<string, unknown>).subscription_id ?? ""));
          }

          setEvents((prev) => [
            {
              timestamp: new Date().toISOString(),
              event: streamEvent.event,
              payload,
            },
            ...prev,
          ].slice(0, 250));
        },
        onError: (message) => {
          setPollFallback(true);
          notify(`Watch stream failed, switched to polling fallback: ${message}`, "error");
        },
      },
      controller.signal,
    )
      .catch((error) => {
        setPollFallback(true);
        notify(normalizeApiError(error), "error");
      })
      .finally(() => {
        setRunning(false);
      });
  }

  async function stopStream() {
    controllerRef.current?.abort();
    controllerRef.current = null;
    setRunning(false);

    if (!subscriptionIdRef.current) {
      return;
    }

    try {
      const cancelled = await cancelSubscription(subscriptionIdRef.current);
      if (cancelled) {
        subscriptionIdRef.current = "";
        setSubscriptionId("");
        notify("Subscription cancelled");
      } else {
        notify("Unable to cancel subscription cleanly", "error");
      }
    } catch (error) {
      notify(normalizeApiError(error), "error");
    }
  }

  const filteredEvents = events.filter((eventItem) => {
    const eventTypeMatch =
      eventTypeFilter === "all" || eventItem.event === eventTypeFilter;

    if (!eventTypeMatch) {
      return false;
    }

    if (!searchQuery.trim()) {
      return true;
    }

    const haystack = JSON.stringify(eventItem.payload).toLowerCase();
    return haystack.includes(searchQuery.toLowerCase());
  });

  return (
    <section className="space-y-4">
      <div className="grid gap-4 xl:grid-cols-[380px_minmax(0,1fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Stream Setup</CardTitle>
            <CardDescription>Configure watch mode and start a durable stream.</CardDescription>
          </CardHeader>
          <CardContent>
            <form className="space-y-3" onSubmit={startStream}>
              <label className="space-y-1">
                <Label>Mode</Label>
                <Select value={watchMode} onValueChange={(value) => setWatchMode(value as WatchMode)}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="query">Query</SelectItem>
                    <SelectItem value="point">Point</SelectItem>
                    <SelectItem value="namespace">Namespace</SelectItem>
                  </SelectContent>
                </Select>
              </label>

              {watchMode !== "namespace" ? (
                <EntityTypeInput
                  value={entityType}
                  onChange={(value) => setEntityType(value)}
                  schemaCatalog={schemaCatalog}
                />
              ) : null}

              {watchMode === "point" ? (
                <label className="space-y-1">
                  <Label>Node ID</Label>
                  <Input value={nodeId} onChange={(event) => setNodeId(event.target.value)} />
                </label>
              ) : null}

              {watchMode === "query" ? (
                <label className="space-y-1">
                  <Label>CEL Expression</Label>
                  <Textarea
                    value={celExpression}
                    onChange={(event) => setCelExpression(event.target.value)}
                    rows={4}
                    className="font-mono text-xs"
                  />
                </label>
              ) : null}

              <div className="flex gap-2">
                <Button type="submit" disabled={running}>
                  {running ? "Streaming..." : "Start Stream"}
                </Button>
                <Button type="button" variant="secondary" onClick={stopStream}>
                  Stop + Cancel
                </Button>
              </div>
            </form>

            <div className="mt-4 flex flex-wrap gap-2">
              <StatusBadge state={running ? "active" : "idle"}>{running ? "Connected" : "Idle"}</StatusBadge>
              <StatusBadge state={pollFallback ? "warning" : "ok"}>
                {pollFallback ? "Polling Fallback" : "SSE Stream"}
              </StatusBadge>
              <StatusBadge state={subscriptionId ? "ok" : "idle"}>
                {subscriptionId ? `Sub ${subscriptionId.slice(0, 8)}...` : "No Subscription"}
              </StatusBadge>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <SectionHeader
              title="Event Feed"
              description="Filter by event type or payload text while stream is active."
            />
          </CardHeader>
          <CardContent>
            <div className="mb-3 grid gap-3 sm:grid-cols-[220px_minmax(0,1fr)]">
              <label className="space-y-1">
                <Label>Event Type</Label>
                <Select value={eventTypeFilter} onValueChange={setEventTypeFilter}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All</SelectItem>
                    <SelectItem value="watch">Watch</SelectItem>
                    <SelectItem value="error">Error</SelectItem>
                    <SelectItem value="poll">Poll</SelectItem>
                  </SelectContent>
                </Select>
              </label>
              <label className="space-y-1">
                <Label>Search Payload</Label>
                <div className="relative">
                  <Search className="pointer-events-none absolute top-2.5 left-2 h-4 w-4 text-muted" />
                  <Input
                    value={searchQuery}
                    onChange={(event) => setSearchQuery(event.target.value)}
                    placeholder="node_id, reason, event..."
                    className="pl-8"
                  />
                </div>
              </label>
            </div>

            {filteredEvents.length > 0 ? (
              <ScrollArea className="h-[520px] rounded-md border border-border bg-slate-50 p-3">
                <div className="space-y-2">
                  {filteredEvents.map((eventItem, index) => (
                    <article
                      key={`${eventItem.timestamp}-${index}`}
                      className="rounded-md border border-border bg-panel p-3"
                    >
                      <header className="mb-2 flex flex-wrap items-center justify-between gap-2">
                        <div className="flex items-center gap-2">
                          <StatusBadge
                            state={eventItem.event === "error" ? "error" : eventItem.event === "poll" ? "warning" : "active"}
                          >
                            {eventItem.event}
                          </StatusBadge>
                        </div>
                        <time className="font-mono text-xs text-muted">{eventItem.timestamp}</time>
                      </header>
                      <pre className="overflow-x-auto font-mono text-xs whitespace-pre-wrap text-slate-700">
                        {JSON.stringify(eventItem.payload as StreamEventPayload, null, 2)}
                      </pre>
                    </article>
                  ))}
                </div>
              </ScrollArea>
            ) : (
              <EmptyState
                title="No Events"
                description="Start stream or adjust filters to display watch activity."
                icon={<Waves className="h-5 w-5" />}
              />
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Stream Guidance</CardTitle>
          <CardDescription>
            Query mode tracks dynamic result sets, point mode tracks one node, namespace mode captures all events.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3 sm:grid-cols-3">
          <div className="rounded-md border border-border bg-slate-50 p-3 text-sm text-muted">
            <p className="mb-1 font-semibold text-foreground">Query Mode</p>
            Use CEL expression and entity type. Best for SLA watches and threshold alerts.
          </div>
          <div className="rounded-md border border-border bg-slate-50 p-3 text-sm text-muted">
            <p className="mb-1 font-semibold text-foreground">Point Mode</p>
            Tracks one known node id. Best for lifecycle and ownership transfer tracking.
          </div>
          <div className="rounded-md border border-border bg-slate-50 p-3 text-sm text-muted">
            <p className="mb-1 font-semibold text-foreground">Namespace Mode</p>
            High-volume audit-like stream. Use filters in feed to stay focused.
          </div>
        </CardContent>
      </Card>
    </section>
  );
}
