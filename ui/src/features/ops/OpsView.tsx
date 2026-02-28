import { useCallback, useEffect, useMemo, useState } from "react";
import { Activity, GaugeCircle, Server, ShieldCheck } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EmptyState } from "@/components/shared/EmptyState";
import { JsonInspector } from "@/components/shared/JsonInspector";
import { PaginationControls } from "@/components/shared/PaginationControls";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { apiRequest, metricsRaw, normalizeApiError } from "@/lib/api";
import { useInterval } from "@/hooks/use-interval";
import type {
  AuditResponse,
  HealthResponse,
  JobsResponse,
  OpsSnapshotResponse,
  ReplicationResponse,
  SitesResponse,
  WatchSubscriptionsResponse,
} from "@/lib/types";
import { MetricCard } from "./MetricCard";
import { parsePrometheusMetrics, type ParsedMetric } from "./MetricsParser";

const initialSnapshot: OpsSnapshotResponse = {
  health: null,
  sites: null,
  replication: null,
  jobs: null,
  audit: null,
  subscriptions: null,
  metrics: "",
};

type RefreshInterval = null | 10000 | 30000 | 60000;

function formatRelativeTime(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ago`;
}

const AUDIT_PAGE_SIZE = 20;

export default function OpsView() {
  const { session, scope, notify } = useConsoleContext();
  const [snapshot, setSnapshot] = useState<OpsSnapshotResponse>(initialSnapshot);
  const [loading, setLoading] = useState(false);
  const [auditPrincipal, setAuditPrincipal] = useState("");
  const [auditAction, setAuditAction] = useState("");
  const [showRaw, setShowRaw] = useState(false);
  const [showRawMetrics, setShowRawMetrics] = useState(false);
  const [refreshInterval, setRefreshInterval] = useState<RefreshInterval>(null);
  const [lastRefreshed, setLastRefreshed] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [auditPage, setAuditPage] = useState(1);

  const loadSnapshot = useCallback(async () => {
    setLoading(true);

    try {
      const [health, sites, replication, jobs, subscriptions] = await Promise.all([
        apiRequest<HealthResponse>("/state/health", { session, scope }),
        apiRequest<SitesResponse>("/state/sites", { session, scope }),
        apiRequest<ReplicationResponse>("/state/replication", { session, scope }),
        apiRequest<JobsResponse>("/state/jobs", { session, scope }),
        apiRequest<WatchSubscriptionsResponse>("/state/watch/subscriptions", { session, scope }),
      ]);

      const auditPath = `/state/audit?limit=50${
        auditPrincipal ? `&principal_id=${encodeURIComponent(auditPrincipal)}` : ""
      }${auditAction ? `&action=${encodeURIComponent(auditAction)}` : ""}`;

      const audit = await apiRequest<AuditResponse>(auditPath, { session, scope });

      let metrics = "";
      try {
        metrics = await metricsRaw(session, scope);
      } catch (error) {
        metrics = normalizeApiError(error);
      }

      setSnapshot({ health, sites, replication, jobs, audit, subscriptions, metrics });
      setLastRefreshed(Date.now());
      setAuditPage(1);
      notify("Operations snapshot refreshed");
    } catch (error) {
      notify(normalizeApiError(error), "error");
    } finally {
      setLoading(false);
    }
  }, [scope, session, notify, auditPrincipal, auditAction]);

  useEffect(() => {
    void loadSnapshot();
  }, [scope.database, scope.namespace]); // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-refresh
  useInterval(() => {
    void loadSnapshot();
  }, refreshInterval);

  // Elapsed timer
  useInterval(() => {
    if (lastRefreshed) setElapsed(Date.now() - lastRefreshed);
  }, 1000);

  const kpiCards = useMemo(
    () => [
      { label: "Sites", value: snapshot.sites?.sites.length ?? 0, icon: <Server className="h-4 w-4" /> },
      { label: "Replication Peers", value: snapshot.replication?.peers.length ?? 0, icon: <GaugeCircle className="h-4 w-4" /> },
      { label: "Jobs", value: snapshot.jobs?.jobs.length ?? 0, icon: <Activity className="h-4 w-4" /> },
      { label: "Subscriptions", value: snapshot.subscriptions?.subscriptions.length ?? 0, icon: <ShieldCheck className="h-4 w-4" /> },
    ],
    [snapshot.jobs?.jobs.length, snapshot.replication?.peers.length, snapshot.sites?.sites.length, snapshot.subscriptions?.subscriptions.length],
  );

  const parsedMetrics = useMemo<ParsedMetric[]>(
    () => (snapshot.metrics ? parsePrometheusMetrics(snapshot.metrics) : []),
    [snapshot.metrics],
  );

  const auditEvents = snapshot.audit?.events ?? [];
  const auditPageCount = Math.max(1, Math.ceil(auditEvents.length / AUDIT_PAGE_SIZE));
  const auditStart = (Math.min(auditPage, auditPageCount) - 1) * AUDIT_PAGE_SIZE;
  const pagedAudit = auditEvents.slice(auditStart, auditStart + AUDIT_PAGE_SIZE);

  function formatTimestamp(ts: number): string {
    if (!ts) return "-";
    const d = new Date(ts);
    const diffMs = Date.now() - d.getTime();
    const relative = formatRelativeTime(diffMs);
    return `${d.toLocaleTimeString()} (${relative})`;
  }

  return (
    <section className="space-y-4">
      <Card>
        <CardHeader>
          <SectionHeader
            title="Cluster Operations"
            description="KPI cards, state tables, and filtered audit logs."
            actions={
              <div className="flex items-center gap-2">
                <StatusBadge state={snapshot.health?.status === "SERVING" ? "ok" : "warning"}>
                  Health {snapshot.health?.status ?? "unknown"}
                </StatusBadge>

                {/* Refresh interval selector */}
                <Select
                  value={refreshInterval?.toString() ?? "off"}
                  onValueChange={(v) => setRefreshInterval(v === "off" ? null : parseInt(v, 10) as RefreshInterval)}
                >
                  <SelectTrigger className="h-8 w-[100px]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="off">Off</SelectItem>
                    <SelectItem value="10000">10s</SelectItem>
                    <SelectItem value="30000">30s</SelectItem>
                    <SelectItem value="60000">60s</SelectItem>
                  </SelectContent>
                </Select>

                {lastRefreshed && (
                  <span className="text-[10px] text-muted">
                    Last: {formatRelativeTime(elapsed)}
                  </span>
                )}

                <Button type="button" variant="secondary" onClick={() => void loadSnapshot()} disabled={loading}>
                  {loading ? "Refreshing..." : "Refresh"}
                </Button>
              </div>
            }
          />
        </CardHeader>
        <CardContent>
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            {kpiCards.map((metric) => (
              <div key={metric.label} className="metric-card">
                <div className="flex items-center justify-between text-muted">
                  <span className="text-xs uppercase tracking-wide">{metric.label}</span>
                  {metric.icon}
                </div>
                <p className="mt-2 text-2xl font-semibold">{metric.value}</p>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      <div className="grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Sites + Replication</CardTitle>
            <CardDescription>Topology and replication lag signals.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {snapshot.sites?.sites.length ? (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Site</TableHead>
                    <TableHead>Name</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {snapshot.sites.sites.map((site) => (
                    <TableRow key={site.site_id}>
                      <TableCell>{site.site_id}</TableCell>
                      <TableCell>{site.site_name || "-"}</TableCell>
                      <TableCell>{site.status}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <EmptyState title="No Sites" description="No site records returned for current scope." />
            )}

            {snapshot.replication?.peers.length ? (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Remote Site</TableHead>
                    <TableHead>Lag Events</TableHead>
                    <TableHead>Updated At</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {snapshot.replication.peers.map((peer) => (
                    <TableRow key={peer.remote_site_id}>
                      <TableCell>{peer.remote_site_id}</TableCell>
                      <TableCell>{peer.lag_events}</TableCell>
                      <TableCell className="font-mono text-xs text-muted">{peer.updated_at}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <EmptyState title="No Replication Peers" description="Replication state is unavailable for this namespace." />
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Jobs + Watch Subscriptions</CardTitle>
            <CardDescription>Operational job queue and watch health surface.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {snapshot.jobs?.jobs.length ? (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Job</TableHead>
                    <TableHead>Type</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Progress</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {snapshot.jobs.jobs.slice(0, 10).map((job) => (
                    <TableRow key={job.job_id}>
                      <TableCell className="font-mono text-xs">{job.job_id}</TableCell>
                      <TableCell>{job.job_type}</TableCell>
                      <TableCell>{job.status}</TableCell>
                      <TableCell>{job.progress}%</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <EmptyState title="No Jobs" description="No active background jobs returned." />
            )}

            {snapshot.subscriptions?.subscriptions.length ? (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>ID</TableHead>
                    <TableHead>Type</TableHead>
                    <TableHead>Expires</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {snapshot.subscriptions.subscriptions.slice(0, 10).map((sub) => (
                    <TableRow key={sub.subscription_id}>
                      <TableCell className="font-mono text-xs">{sub.subscription_id}</TableCell>
                      <TableCell>{sub.subscription_type}</TableCell>
                      <TableCell className="font-mono text-xs text-muted">{sub.expires_at}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <EmptyState title="No Watch Subscriptions" description="No active watch subscriptions found in current scope." />
            )}
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <SectionHeader title="Audit Stream" description="Filter by principal and action for fast incident triage." />
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="grid gap-3 sm:grid-cols-[1fr_1fr_auto]">
              <Input value={auditPrincipal} onChange={(e) => setAuditPrincipal(e.target.value)} placeholder="principal_id" />
              <Input value={auditAction} onChange={(e) => setAuditAction(e.target.value)} placeholder="action" />
              <Button type="button" variant="secondary" onClick={() => void loadSnapshot()}>Apply</Button>
            </div>

            {auditEvents.length ? (
              <>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Timestamp</TableHead>
                      <TableHead>Action</TableHead>
                      <TableHead>Principal</TableHead>
                      <TableHead>Allowed</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {pagedAudit.map((event) => (
                      <TableRow key={event.event_id}>
                        <TableCell className="font-mono text-xs text-muted">
                          {formatTimestamp(event.timestamp)}
                        </TableCell>
                        <TableCell>{event.action}</TableCell>
                        <TableCell className="font-mono text-xs">{event.principal_id}</TableCell>
                        <TableCell>{event.allowed ? "yes" : "no"}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
                <PaginationControls
                  totalItems={auditEvents.length}
                  pageSize={AUDIT_PAGE_SIZE}
                  currentPage={auditPage}
                  onPageChange={setAuditPage}
                />
              </>
            ) : (
              <EmptyState title="No Audit Events" description="No events found for current filters and namespace." />
            )}
            <Button type="button" variant="ghost" onClick={() => setShowRaw(!showRaw)}>
              {showRaw ? "Hide raw payloads" : "Show raw payloads"}
            </Button>
            {showRaw ? <JsonInspector value={snapshot.audit} maxHeight={260} /> : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Prometheus Metrics</CardTitle>
            <CardDescription>
              Structured metric cards parsed from Prometheus text format.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            {parsedMetrics.length > 0 ? (
              <>
                <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                  {parsedMetrics.slice(0, 12).map((m) => (
                    <MetricCard key={m.name} metric={m} />
                  ))}
                </div>
                <Button type="button" variant="ghost" onClick={() => setShowRawMetrics(!showRawMetrics)}>
                  {showRawMetrics ? "Hide raw text" : "Show raw text"}
                </Button>
                {showRawMetrics && (
                  <div className="rounded-md border border-border bg-surface-subtle">
                    <pre className="max-h-[420px] overflow-auto p-3 font-mono text-xs whitespace-pre-wrap scrollbar-thin text-foreground/80">
                      {snapshot.metrics}
                    </pre>
                  </div>
                )}
              </>
            ) : snapshot.metrics ? (
              <div className="rounded-md border border-border bg-surface-subtle">
                <pre className="max-h-[420px] overflow-auto p-3 font-mono text-xs whitespace-pre-wrap scrollbar-thin text-foreground/80">
                  {snapshot.metrics}
                </pre>
              </div>
            ) : (
              <EmptyState title="No Metrics Payload" description="Refresh the operations snapshot to fetch metrics from exporter." />
            )}
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
