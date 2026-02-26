import { FormEvent, useState } from "react";
import { apiRequest } from "../lib/api";
import { useConsoleContext } from "../App";

type MutationResult = {
  title: string;
  payload: unknown;
};

export default function AdminPage() {
  const { session, scope, notify } = useConsoleContext();

  const [dropIndex, setDropIndex] = useState({ entityType: "Person", propertyName: "age", confirm: "" });
  const [stripProperty, setStripProperty] = useState({ entityType: "Person", propertyName: "legacy", confirm: "" });
  const [setOwner, setSetOwner] = useState({ siteId: "", confirm: "" });
  const [transferOwner, setTransferOwner] = useState({ expectedSiteId: "1", targetSiteId: "2", confirm: "" });
  const [transferNode, setTransferNode] = useState({ entityType: "Person", nodeId: "1_0", targetSiteId: "2", confirm: "" });

  const [result, setResult] = useState<MutationResult | null>(null);
  const [loading, setLoading] = useState(false);

  async function runMutation(title: string, path: string, payload: unknown) {
    setLoading(true);
    try {
      const response = await apiRequest<unknown>(path, {
        method: "POST",
        session,
        scope,
        body: payload,
      });
      setResult({ title, payload: response });
      notify(`${title} succeeded`);
    } catch (error) {
      notify(error instanceof Error ? error.message : `${title} failed`, "error");
    } finally {
      setLoading(false);
    }
  }

  function submitDropIndex(event: FormEvent) {
    event.preventDefault();
    void runMutation("Drop Index", "/admin/drop-index", {
      entity_type: dropIndex.entityType,
      property_name: dropIndex.propertyName,
      confirm: dropIndex.confirm,
    });
  }

  function submitStripProperty(event: FormEvent) {
    event.preventDefault();
    void runMutation("Strip Property", "/admin/strip-property", {
      entity_type: stripProperty.entityType,
      property_name: stripProperty.propertyName,
      confirm: stripProperty.confirm,
    });
  }

  function submitSetOwner(event: FormEvent) {
    event.preventDefault();
    void runMutation("Set Namespace Owner", "/admin/set-namespace-schema-owner", {
      site_id: setOwner.siteId,
      confirm: setOwner.confirm,
    });
  }

  function submitTransferNamespaceOwner(event: FormEvent) {
    event.preventDefault();
    void runMutation("Transfer Namespace Owner", "/admin/transfer-namespace-schema-owner", {
      expected_site_id: transferOwner.expectedSiteId,
      target_site_id: transferOwner.targetSiteId,
      confirm: transferOwner.confirm,
    });
  }

  function submitTransferNode(event: FormEvent) {
    event.preventDefault();
    void runMutation("Transfer Node Ownership", "/admin/transfer-node-ownership", {
      entity_type: transferNode.entityType,
      node_id: transferNode.nodeId,
      target_site_id: transferNode.targetSiteId,
      confirm: transferNode.confirm,
    });
  }

  return (
    <section className="page-grid">
      <article className="card">
        <h3>Safe Mutations</h3>
        <p className="hint">Destructive namespace/type drops are intentionally omitted in V1.</p>

        <form className="form-grid bordered" onSubmit={submitDropIndex}>
          <h4>Drop Index</h4>
          <input value={dropIndex.entityType} onChange={(event) => setDropIndex({ ...dropIndex, entityType: event.target.value })} placeholder="Entity type" />
          <input value={dropIndex.propertyName} onChange={(event) => setDropIndex({ ...dropIndex, propertyName: event.target.value })} placeholder="Property" />
          <input value={dropIndex.confirm} onChange={(event) => setDropIndex({ ...dropIndex, confirm: event.target.value })} placeholder="Type DROP_INDEX" />
          <button type="submit" disabled={loading}>Execute</button>
        </form>

        <form className="form-grid bordered" onSubmit={submitStripProperty}>
          <h4>Strip Property</h4>
          <input value={stripProperty.entityType} onChange={(event) => setStripProperty({ ...stripProperty, entityType: event.target.value })} placeholder="Entity type" />
          <input value={stripProperty.propertyName} onChange={(event) => setStripProperty({ ...stripProperty, propertyName: event.target.value })} placeholder="Property" />
          <input value={stripProperty.confirm} onChange={(event) => setStripProperty({ ...stripProperty, confirm: event.target.value })} placeholder="Type STRIP_PROPERTY" />
          <button type="submit" disabled={loading}>Execute</button>
        </form>
      </article>

      <article className="card">
        <h3>Ownership Controls</h3>

        <form className="form-grid bordered" onSubmit={submitSetOwner}>
          <h4>Set Namespace Schema Owner</h4>
          <input value={setOwner.siteId} onChange={(event) => setSetOwner({ ...setOwner, siteId: event.target.value })} placeholder="Site ID (empty clears owner)" />
          <input value={setOwner.confirm} onChange={(event) => setSetOwner({ ...setOwner, confirm: event.target.value })} placeholder="Type SET_NAMESPACE_SCHEMA_OWNER" />
          <button type="submit" disabled={loading}>Execute</button>
        </form>

        <form className="form-grid bordered" onSubmit={submitTransferNamespaceOwner}>
          <h4>Transfer Namespace Owner</h4>
          <input value={transferOwner.expectedSiteId} onChange={(event) => setTransferOwner({ ...transferOwner, expectedSiteId: event.target.value })} placeholder="Expected site" />
          <input value={transferOwner.targetSiteId} onChange={(event) => setTransferOwner({ ...transferOwner, targetSiteId: event.target.value })} placeholder="Target site" />
          <input value={transferOwner.confirm} onChange={(event) => setTransferOwner({ ...transferOwner, confirm: event.target.value })} placeholder="Type TRANSFER_NAMESPACE_SCHEMA_OWNER" />
          <button type="submit" disabled={loading}>Execute</button>
        </form>

        <form className="form-grid bordered" onSubmit={submitTransferNode}>
          <h4>Transfer Node Ownership</h4>
          <input value={transferNode.entityType} onChange={(event) => setTransferNode({ ...transferNode, entityType: event.target.value })} placeholder="Entity type" />
          <input value={transferNode.nodeId} onChange={(event) => setTransferNode({ ...transferNode, nodeId: event.target.value })} placeholder="Node ID" />
          <input value={transferNode.targetSiteId} onChange={(event) => setTransferNode({ ...transferNode, targetSiteId: event.target.value })} placeholder="Target site" />
          <input value={transferNode.confirm} onChange={(event) => setTransferNode({ ...transferNode, confirm: event.target.value })} placeholder="Type TRANSFER_NODE_OWNERSHIP" />
          <button type="submit" disabled={loading}>Execute</button>
        </form>
      </article>

      <article className="card">
        <h3>Mutation Result</h3>
        {result ? (
          <div className="json-block">
            <h4>{result.title}</h4>
            <pre>{JSON.stringify(result.payload, null, 2)}</pre>
          </div>
        ) : (
          <p className="empty-state">Run a mutation to inspect response payloads.</p>
        )}
      </article>
    </section>
  );
}
