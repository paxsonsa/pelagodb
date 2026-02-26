import { useEffect, useState } from "react";
import { apiRequest, metricsRaw } from "../lib/api";
import { useConsoleContext } from "../App";

type OpsSnapshot = {
  health: unknown;
  sites: unknown;
  replication: unknown;
  jobs: unknown;
  audit: unknown;
  subscriptions: unknown;
  metrics: string;
};

const initialState: OpsSnapshot = {
  health: null,
  sites: null,
  replication: null,
  jobs: null,
  audit: null,
  subscriptions: null,
  metrics: "",
};

export default function OpsPage() {
  const { session, scope, notify } = useConsoleContext();
  const [snapshot, setSnapshot] = useState<OpsSnapshot>(initialState);
  const [loading, setLoading] = useState(false);

  async function load() {
    setLoading(true);
    try {
      const [health, sites, replication, jobs, audit, subscriptions] = await Promise.all([
        apiRequest<unknown>("/state/health", { session, scope }),
        apiRequest<unknown>("/state/sites", { session, scope }),
        apiRequest<unknown>("/state/replication", { session, scope }),
        apiRequest<unknown>("/state/jobs", { session, scope }),
        apiRequest<unknown>("/state/audit?limit=50", { session, scope }),
        apiRequest<unknown>("/state/watch/subscriptions", { session, scope }),
      ]);

      let metrics = "";
      try {
        metrics = await metricsRaw(session, scope);
      } catch (error) {
        metrics = error instanceof Error ? error.message : "Metrics unavailable";
      }

      setSnapshot({
        health,
        sites,
        replication,
        jobs,
        audit,
        subscriptions,
        metrics,
      });
      notify("Operations snapshot refreshed");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Failed to load operations data", "error");
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope.database, scope.namespace]);

  return (
    <section className="page-grid">
      <article className="card">
        <div className="card-header-row">
          <h3>Cluster State</h3>
          <button type="button" onClick={load} disabled={loading}>
            {loading ? "Refreshing..." : "Refresh"}
          </button>
        </div>
        <div className="json-block compact">
          <h4>Health</h4>
          <pre>{JSON.stringify(snapshot.health, null, 2)}</pre>
        </div>
        <div className="json-block compact">
          <h4>Sites</h4>
          <pre>{JSON.stringify(snapshot.sites, null, 2)}</pre>
        </div>
        <div className="json-block compact">
          <h4>Replication</h4>
          <pre>{JSON.stringify(snapshot.replication, null, 2)}</pre>
        </div>
      </article>

      <article className="card">
        <h3>Jobs + Watch State</h3>
        <div className="json-block compact">
          <h4>Jobs</h4>
          <pre>{JSON.stringify(snapshot.jobs, null, 2)}</pre>
        </div>
        <div className="json-block compact">
          <h4>Subscriptions</h4>
          <pre>{JSON.stringify(snapshot.subscriptions, null, 2)}</pre>
        </div>
      </article>

      <article className="card">
        <h3>Audit Stream</h3>
        <div className="json-block compact">
          <pre>{JSON.stringify(snapshot.audit, null, 2)}</pre>
        </div>
      </article>

      <article className="card">
        <h3>Prometheus Raw Metrics</h3>
        <p className="hint">Reads from `/ui/api/v1/metrics/raw`; shows graceful fallback when disabled.</p>
        <div className="json-block metrics">
          <pre>{snapshot.metrics || "No metrics payload yet."}</pre>
        </div>
      </article>
    </section>
  );
}
