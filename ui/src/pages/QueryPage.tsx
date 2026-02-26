import { FormEvent, useState } from "react";
import { apiRequest } from "../lib/api";
import { useConsoleContext } from "../App";

export default function QueryPage() {
  const { session, scope, notify } = useConsoleContext();
  const [entityType, setEntityType] = useState("Person");
  const [celExpression, setCelExpression] = useState("age >= 30");
  const [pql, setPql] = useState("Person @filter(age >= 30) { uid name age }");
  const [queryResult, setQueryResult] = useState<unknown>(null);
  const [explainResult, setExplainResult] = useState<unknown>(null);
  const [loading, setLoading] = useState(false);

  async function runFind(event: FormEvent) {
    event.preventDefault();
    setLoading(true);
    try {
      const response = await apiRequest<unknown>("/query/find", {
        method: "POST",
        session,
        scope,
        body: {
          entity_type: entityType,
          cel_expression: celExpression,
          limit: 100,
          consistency: "strong",
          snapshot_mode: "strict",
          allow_degrade_to_best_effort: true,
        },
      });
      setQueryResult(response);
      notify("CEL query completed");
    } catch (error) {
      notify(error instanceof Error ? error.message : "CEL query failed", "error");
    } finally {
      setLoading(false);
    }
  }

  async function runPql(event: FormEvent) {
    event.preventDefault();
    setLoading(true);
    try {
      const response = await apiRequest<unknown>("/query/pql", {
        method: "POST",
        session,
        scope,
        body: {
          pql,
          explain: false,
          snapshot_mode: "best_effort",
          allow_degrade_to_best_effort: true,
        },
      });
      setQueryResult(response);
      notify("PQL query completed");
    } catch (error) {
      notify(error instanceof Error ? error.message : "PQL query failed", "error");
    } finally {
      setLoading(false);
    }
  }

  async function runExplain(event: FormEvent) {
    event.preventDefault();
    setLoading(true);
    try {
      const response = await apiRequest<unknown>("/query/explain", {
        method: "POST",
        session,
        scope,
        body: {
          entity_type: entityType,
          cel_expression: celExpression,
        },
      });
      setExplainResult(response);
      notify("Explain plan generated");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Explain failed", "error");
    } finally {
      setLoading(false);
    }
  }

  return (
    <section className="page-grid">
      <article className="card">
        <h3>CEL Query Workbench</h3>
        <form className="form-grid" onSubmit={runFind}>
          <label>
            Entity Type
            <input value={entityType} onChange={(event) => setEntityType(event.target.value)} />
          </label>
          <label>
            CEL Expression
            <textarea
              value={celExpression}
              rows={3}
              onChange={(event) => setCelExpression(event.target.value)}
            />
          </label>
          <div className="actions-row">
            <button type="submit" disabled={loading}>
              Run CEL Find
            </button>
            <button type="button" onClick={runExplain} disabled={loading}>
              Explain
            </button>
          </div>
        </form>
        <div className="json-block">
          <h4>Explain Plan</h4>
          <pre>{JSON.stringify(explainResult, null, 2)}</pre>
        </div>
      </article>

      <article className="card">
        <h3>PQL Query Workbench</h3>
        <form className="form-grid" onSubmit={runPql}>
          <label>
            PQL
            <textarea value={pql} rows={7} onChange={(event) => setPql(event.target.value)} />
          </label>
          <button type="submit" disabled={loading}>
            Run PQL
          </button>
        </form>
      </article>

      <article className="card">
        <h3>Results</h3>
        {queryResult ? (
          <div className="json-block">
            <pre>{JSON.stringify(queryResult, null, 2)}</pre>
          </div>
        ) : (
          <p className="empty-state">Run a query to inspect streamed result payloads.</p>
        )}
      </article>
    </section>
  );
}
