# PelagoDB Centralized Replicator Conversion Checklist

Date: 2026-02-17  
Source Spec: `.llm/shared/research/2026-02-17-centralized-replicator-scaling-spec-internal.md`  
Scope: convert spec into executable work plan, implement core runtime changes, and provide Kubernetes rollout checklist

## 1. Conversion Matrix

| ID | Workstream | Deliverable | Validation | Status |
|---|---|---|---|---|
| C1 | Control-plane scope | Scoped replication checkpoints by `<db,ns,remote_site>` | Admin `replication-status` returns scoped data | Completed |
| C2 | Cache convergence | Replicator mirrors applied remote ops into local CDC | Projector receives mirrored events and updates cache | Completed |
| C3 | Singleton safety | Lease-based replicator gating (TTL + heartbeat) | Only lease-holder replicator performs pulls/applies | Completed |
| C4 | Config surface | CLI/env/config file flags for lease controls | `--help` and env mapping expose fields | Completed |
| C5 | Ops docs | Admin + replication docs updated with lease and topology guidance | Docs render and reference new spec | Completed |
| C6 | K8s artifacts | Dedicated `api` and `replicator` deployment templates | `kubectl apply --dry-run=client` (operator step) | Completed |
| C7 | Validation harness | Focused Rust tests for touched crates | `cargo test` on affected crates passes | Completed |

## 2. Implementation Breakdown

## Runtime

- [x] Add standalone CDC append helper for system workers.
- [x] Add scoped replication position read/write helpers.
- [x] Add replication lease read/acquire helper.
- [x] Update replicator loop to:
  - [x] gate execution on lease ownership
  - [x] apply operations with existing ownership/LWW filters
  - [x] mirror successfully applied operations into local CDC
  - [x] persist scoped replication checkpoints
- [x] Keep `source_site` pull filtering in place.

## Configuration

- [x] Add lease flags in `ServerConfig`:
  - `replication_lease_enabled`
  - `replication_lease_ttl_ms`
  - `replication_lease_heartbeat_ms`
- [x] Map config file keys to environment variables in server bootstrap.
- [x] Update `pelago-server.example.toml`.

## Admin and Visibility

- [x] Make admin replication status use scoped checkpoint reads keyed by request context `<db,ns>`.

## Documentation

- [x] Add centralized replicator/scaling spec document.
- [x] Add references from replication and docs index pages.
- [x] Document lease config and operational profile guidance.

## Kubernetes

- [x] Create `deploy/k8s` templates for split topology:
  - [x] API deployment/service
  - [x] Dedicated replicator deployment
  - [x] ConfigMap + Secret wiring
  - [x] HPA template for API workload
  - [x] PodDisruptionBudget for API
- [x] Add Kubernetes rollout checklist/runbook.

## 3. Kubernetes Rollout Checklist

## Preflight

- [ ] FoundationDB reachable from cluster nodes.
- [ ] `fdb.cluster` injected via secret or mounted file.
- [ ] Unique `site_id` and stable `site_name` chosen for cluster/site.
- [ ] Replication API key created and stored in Kubernetes secret.

## Deploy API Tier

- [ ] Set `PELAGO_REPLICATION_ENABLED=false`.
- [ ] Set unique cache path per pod (PVC or isolated path).
- [ ] Expose gRPC via ClusterIP/LoadBalancer as needed.
- [ ] Configure HPA for read/query scaling.

## Deploy Replicator Tier

- [ ] Set `PELAGO_REPLICATION_ENABLED=true`.
- [ ] Set `PELAGO_REPLICATION_LEASE_ENABLED=true`.
- [ ] Configure peers with `PELAGO_REPLICATION_PEERS`.
- [ ] Set `PELAGO_REPLICATION_DATABASE` and `PELAGO_REPLICATION_NAMESPACE`.
- [ ] Keep replicas >= 1 (leader elected by lease; standby pods optional).

## Verify

- [ ] `pelago admin sites` lists expected sites.
- [ ] `pelago admin replication-status` advances per remote peer.
- [ ] Audit log does not show sustained `replication.conflict` spikes.
- [ ] Cache projector lag remains bounded under load.

## Failure Drill

- [ ] Kill active replicator pod and verify lease failover.
- [ ] Confirm checkpoint resumes from last applied versionstamp.
- [ ] Confirm API read/query behavior remains healthy during failover.

## 4. Remaining Validation Tasks

- [x] Run targeted tests:
  - [x] `cargo test -p pelago-storage --lib`
  - [x] `cargo test -p pelago-server`
  - [x] `cargo test -p pelago-api --lib`
- [ ] Run local two-site replication smoke check with replicated cache convergence.
