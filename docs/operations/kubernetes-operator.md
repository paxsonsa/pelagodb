# Kubernetes Operator

PelagoDB's recommended Kubernetes deployment path is the `PelagoSite` custom resource managed by the `pelago-operator`.

## What The Operator Manages

A single `PelagoSite` resource manages one site topology in one namespace:

- API Deployment (`<name>-api`)
- Replicator Deployment (`<name>-replicator`)
- API Service (`<name>-api`)
- ConfigMap (`<name>-config`)
- Optional API HPA (`<name>-api`)
- Optional API PodDisruptionBudget (`<name>-api`)

It also updates `status` conditions and tier readiness on the `PelagoSite` resource.

## Prerequisites

- Kubernetes cluster `>= 1.31`
- cert-manager installed (recommended and enabled by default in chart)
- FoundationDB managed separately (this operator consumes existing cluster file secret)

## Install Operator (Helm)

```bash
helm upgrade --install pelago-operator deploy/operator/chart/pelago-operator \
  --namespace pelagodb-system \
  --create-namespace
```

## Create Site Resources

Single-site baseline:

```bash
kubectl apply -n pelagodb -f deploy/operator/examples/single-site-basic.yaml
```

Split API/replicator with peer topology:

```bash
kubectl apply -n pelagodb -f deploy/operator/examples/split-topology-with-peers.yaml
```

## Observe Status

```bash
kubectl get pelagosites -n pelagodb
kubectl get pelagosite pelago-site1 -n pelagodb -o yaml
```

Status fields include:

- `status.conditions` (`Ready`, `ConfigValid`, `ApiAvailable`, `ReplicatorAvailable`, `Progressing`, `Degraded`)
- `status.api.readyReplicas` / `desiredReplicas`
- `status.replicator.readyReplicas` / `desiredReplicas`

## Secret References

`PelagoSite` references existing secrets for:

- FoundationDB cluster file (`spec.fdb.clusterFileSecretRef`)
- API keys (`spec.auth.apiKeysSecretRef`)
- Replication peer API key (`spec.replicator.peers[*].apiKeySecretRef`)
- Replication TLS refs (`spec.replicator.tls.*SecretRef`)

Missing referenced secrets set the resource into a degraded reconciliation state.

## Webhooks

The operator ships mutating and validating admission webhooks:

- Mutating webhook defaults:
  - `replicator.enabled=true` when peers are configured
  - replication scopes fallback to `defaults.database/defaults.namespace`
- Validating webhook enforces:
  - peer ID uniqueness and no self-peer
  - non-empty unique scopes
  - HPA min/max consistency
  - replication lease constraints
  - TLS mode/reference coherence

## Legacy Static Manifests

`deploy/k8s/*.yaml` remain available for manual installs, but are now legacy.

Use `PelagoSite` for new environments and ongoing operations.

## Related

- [Kubernetes Deployment](kubernetes.md)
- [Deployment Guide](deployment.md)
- [Configuration Reference](../reference/configuration.md)
