import { useEffect, useMemo, useState } from "react";
import { Activity, GaugeCircle, Server, ShieldCheck } from "lucide-react";

import { useConsoleContext } from "@/App";
import { EmptyState } from "@/components/shared/EmptyState";
import { JsonInspector } from "@/components/shared/JsonInspector";
import { SectionHeader } from "@/components/shared/SectionHeader";
import { StatusBadge } from "@/components/shared/StatusBadge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { apiRequest, metricsRaw, normalizeApiError } from "@/lib/api";
import type {
  AuditResponse,
  HealthResponse,
  JobsResponse,
  OpsSnapshotResponse,
  ReplicationResponse,
  SitesResponse,
  WatchSubscriptionsResponse,
} from "@/lib/types";

const initialSnapshot: OpsSnapshotResponse = {
  health: null,
  sites: null,
  replication: null,
  jobs: null,
  audit: null,
  subscriptions: null,
  metrics: "",
};

export default function OpsView() {
  const { session, scope, notify } = useConsoleContext();
  const [snapshot, setSnapshot] = useState<OpsSnapshotResponse>(initialSnapshot);
  const [loading, setLoading] = useState(false);
  const [auditPrincipal, setAuditPrincipal] = useState("");
  const [auditAction, setAuditAction] = useState("");
  const [showRaw, setShowRaw] = useState(false);

  async function loadSnapshot() {
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
      notify("Operations snapshot refreshed");
    } catch (error) {
      notify(normalizeApiError(error), "error");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadSnapshot();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope.database, scope.namespace]);

  const metrics = useMemo(
    () => [
      { label: "Sites", value: snapshot.sites?.sites.length ?? 0, icon: <Server className="h-4 w-4" /> },
      {
        label: "Replication Peers",
        value: snapshot.replication?.peers.length ?? 0,
        icon: <GaugeCircle className="h-4 w-4" />,
      },
      { label: "Jobs", value: snapshot.jobs?.jobs.length ?? 0, icon: <Activity className="h-4 w-4" /> },
      {
        label: "Subscriptions",
        value: snapshot.subscriptions?.subscriptions.length ?? 0,
        icon: <ShieldCheck className="h-4 w-4" />,
      },
    ],
    [snapshot.jobs?.jobs.length, snapshot.replication?.peers.length, snapshot.sites?.sites.length, snapshot.subscriptions?.subscriptions.length],
  );

  return (
    <section className="space-y-4">
      <Card>
        <CardHeader>
          <SectionHeader
            title="Cluster Operations"
            description="KPI cards, state tables, and filtered audit logs with resilient metrics fallback."
            actions={
              <div className="flex items-center gap-2">
                <StatusBadge state={snapshot.health?.status === "SERVING" ? "ok" : "warning"}>
                  Health {snapshot.health?.status ?? "unknown"}
                </StatusBadge>
                <Button type="button" variant="secondary" onClick={loadSnapshot} disabled={loading}>
                  {loading ? "Refreshing..." : "Refresh"}
                </Button>
              </div>
            }
          />
        </CardHeader>
        <CardContent>
          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
            {metrics.map((metric) => (
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
              <EmptyState
                title="No Replication Peers"
                description="Replication state is unavailable for this namespace."
              />
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
                  {snapshot.subscriptions.subscriptions.slice(0, 10).map((subscription) => (
                    <TableRow key={subscription.subscription_id}>
                      <TableCell className="font-mono text-xs">{subscription.subscription_id}</TableCell>
                      <TableCell>{subscription.subscription_type}</TableCell>
                      <TableCell className="font-mono text-xs text-muted">{subscription.expires_at}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <EmptyState
                title="No Watch Subscriptions"
                description="No active watch subscriptions found in current scope."
              />
            )}
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader>
            <SectionHeader
              title="Audit Stream"
              description="Filter by principal and action for fast incident triage."
            />
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="grid gap-3 sm:grid-cols-[1fr_1fr_auto]">
              <Input
                value={auditPrincipal}
                onChange={(event) => setAuditPrincipal(event.target.value)}
                placeholder="principal_id"
              />
              <Input
                value={auditAction}
                onChange={(event) => setAuditAction(event.target.value)}
                placeholder="action"
              />
              <Button type="button" variant="secondary" onClick={loadSnapshot}>
                Apply
              </Button>
            </div>

            {snapshot.audit?.events.length ? (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Action</TableHead>
                    <TableHead>Principal</TableHead>
                    <TableHead>Allowed</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {snapshot.audit.events.slice(0, 12).map((event) => (
                    <TableRow key={event.event_id}>
                      <TableCell>{event.action}</TableCell>
                      <TableCell className="font-mono text-xs">{event.principal_id}</TableCell>
                      <TableCell>{event.allowed ? "yes" : "no"}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ) : (
              <EmptyState
                title="No Audit Events"
                description="No events found for current filters and namespace."
              />
            )}
            <Button type="button" variant="ghost" onClick={() => setShowRaw((prev) => !prev)}>
              {showRaw ? "Hide raw payloads" : "Show raw payloads"}
            </Button>
            {showRaw ? <JsonInspector value={snapshot.audit} maxHeight={260} /> : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Prometheus Metrics</CardTitle>
            <CardDescription>
              Reads from <code>/ui/api/v1/metrics/raw</code>; gracefully falls back when exporter is disabled.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {snapshot.metrics ? (
              <div className="rounded-md border border-border bg-slate-50">
                <pre className="max-h-[420px] overflow-auto p-3 font-mono text-xs whitespace-pre-wrap scrollbar-thin">
                  {snapshot.metrics}
                </pre>
              </div>
            ) : (
              <EmptyState
                title="No Metrics Payload"
                description="Refresh the operations snapshot to fetch metrics from exporter."
              />
            )}
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
