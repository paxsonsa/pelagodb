# Kubernetes Rollout Checklist (Centralized Replicator)

## Topology Intent

- API tier scales horizontally.
- Replicator tier is centralized and lease-gated.
- Cache projector runs on API tier from local CDC.

## Preflight

- [ ] FoundationDB reachable from pods.
- [ ] `pelagodb-secrets` populated with valid `fdb.cluster`.
- [ ] Site identity (`PELAGO_SITE_ID`, `PELAGO_SITE_NAME`) is correct for this site.
- [ ] Peer list in `PELAGO_REPLICATION_PEERS` is accurate.
- [ ] Replication API key is provisioned on source/target.

## Apply

- [ ] `kubectl apply -f deploy/k8s/configmap.yaml`
- [ ] `kubectl apply -f deploy/k8s/api-deployment.yaml`
- [ ] `kubectl apply -f deploy/k8s/api-service.yaml`
- [ ] `kubectl apply -f deploy/k8s/replicator-deployment.yaml`
- [ ] Optional: `api-hpa.yaml` and `api-pdb.yaml`

## Validate

- [ ] API pods are ready and serving traffic.
- [ ] Replicator pods are running; only one should actively pull/apply due to lease.
- [ ] `pelago admin replication-status` shows advancing checkpoints for configured namespace.
- [ ] Audit log does not show sustained `replication.conflict` increase.
- [ ] Cache hit behavior remains stable after replicated writes.

## Failure Drill

- [ ] Delete active replicator pod.
- [ ] Confirm standby pod acquires lease and resumes replication.
- [ ] Confirm no duplicate mutation amplification.
- [ ] Confirm API traffic remains healthy during failover.
